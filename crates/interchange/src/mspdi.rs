//! Microsoft Project XML (MSPDI) import, from Microsoft's published schema documentation
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/project-element>).
//!
//! Imported so far: the project's name and start date, hours per day and week and days per month,
//! its default task type and whether new tasks are effort-driven,
//! the project calendar's working week (its `WeekDays` with `DayType` 1 to 7 and their
//! `WorkingTimes`) and its exceptions (each `Exception`, recurring ones worked out to their days,
//! with its own working times; or, in older files, `DayType` 0 weekdays), and each task's name,
//! outline level and duration (in the unit its `DurationFormat` gives), constraint, deadline, task
//! type, effort-driven setting and fixed cost ([`fields`]), priority and leveling delay; the currency (`CurrencyCode`,
//! `CurrencySymbol`, `CurrencyDigits`); and the resources, their calendars, the assignments and
//! their costs ([`resources`]). Other base calendars and task calendars are dropped, a report
//! line each. The project summary task (UID 0) is skipped:
//! ProjectCraft rolls the project up itself. Everything else that the file holds and the project
//! can't is a report line.
//!
//! Durations are `xs:duration` values; Project writes them in hours, minutes and seconds
//! (`PT16H0M0S` is two 8-hour days). A day component (`P1D`) is 24 hours, as `xs:duration` has
//! it; years and months are refused as unreadable, having no fixed length.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::model::{
    self, Calendar, Constraint, Currency, DEFAULT_CURRENCY_CODE, Date, DurationUnit, MAX_CURRENCY_DIGITS, MAX_MINUTES, MAX_OUTLINE_LEVEL, MAX_TASKS,
    MINUTES_IN_DAY, Options, Outline, Project, ResourceCalendar, Task, TaskId, TaskType, WorkingTime,
};
use crate::mspdi_links::{self, FileRow};
use crate::xml::{self, Visitor};
use crate::{Format, Import, ImportError, ImportReport, Outcome, ReportLine};

mod fields;
mod number;
mod resources;

/// The MSPDI namespace Microsoft Project writes on its root `Project` element.
pub(crate) const NAMESPACE: &str = "http://schemas.microsoft.com/project";
/// Calendars kept for reading; more are only counted.
const MAX_CALENDARS: usize = 10_000;
/// Exceptions kept for reading across the file's calendars; more are only counted.
const MAX_STORED_EXCEPTIONS: usize = 2 * model::MAX_EXCEPTIONS;
/// Days looked at while working out recurring exceptions' days, across the whole calendar. Keeps
/// thousands of exceptions recurring over thousands of years from taking long.
const MAX_EXCEPTION_STEPS: u64 = 2_000_000;
/// Calendars named one to a report line before the rest share a line.
const NAMED_CALENDARS: usize = 50;
/// Tasks named in one report line before "and n more".
const NAMED_IN_LINE: usize = 3;

/// Whether the root element is MSPDI's: `Project`, in the MSPDI namespace or none.
fn root_is_mspdi(name: &str, ns: Option<&str>) -> Result<(), ImportError> {
    match (name, ns) {
        ("Project", None) => Ok(()),
        ("Project", Some(ns)) if ns == NAMESPACE => Ok(()),
        ("Project", Some(ns)) => Err(ImportError::NotMspdi(format!("Project in the namespace {}", ns.chars().take(80).collect::<String>()))),
        (other, _) => Err(ImportError::NotMspdi(other.chars().take(40).collect())),
    }
}

pub(crate) fn detect(bytes: &[u8]) -> bool {
    xml::text_of(bytes).and_then(xml::root).is_ok_and(|(name, ns)| root_is_mspdi(&name, ns.as_deref()).is_ok())
}

pub(crate) fn import(bytes: &[u8]) -> Result<Import, ImportError> {
    let text = xml::text_of(bytes)?;
    let (name, ns) = xml::root(text)?;
    root_is_mspdi(&name, ns.as_deref())?;
    let mut file = FileProject::default();
    xml::walk(text, &mut file)?;
    file.into_import()
}

/// Something about a task that the report mentions. The order is the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Note {
    NoDuration,
    BadDuration,
    NegativeDuration,
    LongDuration,
    Elapsed,
    Estimated,
    Outline,
    Manual,
    Recurring,
    Progress,
    Baseline,
    Notes,
    Hyperlink,
    CustomFields,
    OutlineCodes,
    Priority,
    LevelingDelay,
    Timephased,
    Subproject,
}

impl Note {
    const ALL: [Note; 19] = [
        Note::NoDuration,
        Note::BadDuration,
        Note::NegativeDuration,
        Note::LongDuration,
        Note::Elapsed,
        Note::Estimated,
        Note::Outline,
        Note::Manual,
        Note::Recurring,
        Note::Progress,
        Note::Baseline,
        Note::Notes,
        Note::Hyperlink,
        Note::CustomFields,
        Note::OutlineCodes,
        Note::Priority,
        Note::LevelingDelay,
        Note::Timephased,
        Note::Subproject,
    ];

    fn bit(self) -> u32 {
        1 << (self as u8)
    }

    /// Outcome, feature and why.
    fn line(self) -> (Outcome, &'static str, &'static str) {
        use Outcome::*;
        match self {
            Note::NoDuration => (Converted, "Duration", "the file gives none; 1 day used"),
            Note::BadDuration => (Converted, "Duration", "unreadable; 1 day used"),
            Note::NegativeDuration => (Converted, "Duration", "negative; read as 0 (a milestone)"),
            Note::LongDuration => (Converted, "Duration", "longer than ProjectCraft holds; cut to 100,000 days"),
            Note::Elapsed => (
                Converted,
                "Elapsed duration",
                "ProjectCraft counts durations in working time, so the elapsed span is the same number of working units (3ed is 3d)",
            ),
            Note::Estimated => (Dropped, "Estimated duration (?)", "ProjectCraft has no estimate mark; the duration is kept"),
            Note::Outline => (Converted, "Outline level", "deeper than the row above allows; moved up to fit the outline"),
            Note::Manual => (Converted, "Manually scheduled", "auto-scheduled: ProjectCraft schedules every task"),
            Note::Recurring => {
                (Dropped, "Recurring task", "the recurrence is dropped; its occurrences are imported as ordinary tasks under a summary task")
            }
            Note::Progress => (Dropped, "Progress (actuals, % complete)", "tracking comes in a later version"),
            Note::Baseline => (Dropped, "Baselines", "baselines come in a later version"),
            Note::Notes => (Dropped, "Notes", "ProjectCraft doesn't hold task notes yet"),
            Note::Hyperlink => (Dropped, "Hyperlink", "ProjectCraft doesn't hold hyperlinks yet"),
            Note::CustomFields => (Dropped, "Custom fields", "ProjectCraft doesn't hold custom fields yet"),
            Note::OutlineCodes => (Dropped, "Outline codes", "ProjectCraft doesn't hold custom outline codes yet"),
            Note::Priority => (Converted, "Priority", "unreadable or outside 0 to 1,000; brought within it (500 if unreadable)"),
            Note::LevelingDelay => (
                Converted,
                "Leveling delay",
                "elapsed, a percentage, negative, unreadable or longer than ProjectCraft holds; read as working time within 0 to 100,000 days (an elapsed span as the same number of working units, a percentage or an unreadable one as none)",
            ),
            Note::Timephased => (Dropped, "Timephased data", "ProjectCraft works out each day's work itself"),
            Note::Subproject => (Dropped, "Inserted project", "inserted projects are imported as ordinary tasks"),
        }
    }
}

/// A task as the file has it, with the fields read so far.
#[derive(Debug, Default)]
struct FileTask {
    uid: Option<i64>,
    id: Option<i64>,
    name: String,
    level: Option<i64>,
    duration: Option<String>,
    format: Option<i64>,
    null: bool,
    /// Its own calendar's UID (`-1`, none, is left out).
    calendar: Option<i64>,
    /// [`Note`] bits.
    notes: u32,
    /// Constraint, deadline, task type and the like ([`fields`]).
    fields: fields::TaskFields,
    /// Its links and the file's dates for it.
    row: FileRow,
    /// `Priority`, 0 to 1,000, as the file writes it.
    priority: Option<String>,
    /// `LevelingDelay`, in tenths of a minute, and `LevelingDelayFormat`, the unit it shows in.
    leveling_delay: Option<String>,
    leveling_delay_format: Option<i64>,
}

impl FileTask {
    fn note(&mut self, n: Note) {
        self.notes |= n.bit();
    }
}

#[derive(Debug, Default)]
struct FileWeekDay {
    day_type: Option<i64>,
    working: Option<bool>,
    /// From and to, as the file writes them; more than a day holds are refused later.
    times: Vec<(String, String)>,
    /// An exception's days (`DayType` 0, as files before Project 2007 have exceptions).
    from: Option<Date>,
    to: Option<Date>,
}

/// One `Exception`, as the file has it
/// (<https://learn.microsoft.com/office-project/xml-data-interchange/exception-element>).
#[derive(Debug, Default)]
struct FileException {
    name: String,
    from: Option<Date>,
    to: Option<Date>,
    by_occurrences: bool,
    occurrences: Option<i64>,
    /// `Type`: 1 daily, 2 yearly on a day, 3 yearly by position, 4 monthly on a day, 5 monthly
    /// by position, 6 weekly, 7 by day count, 8 by weekday count, 9 none.
    kind: Option<i64>,
    period: Option<i64>,
    days_of_week: Option<i64>,
    month: Option<i64>,
    month_day: Option<i64>,
    month_item: Option<i64>,
    month_position: Option<i64>,
    working: Option<bool>,
    times: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct FileCalendar {
    uid: Option<i64>,
    name: Option<String>,
    base: bool,
    /// `BaseCalendarUID`: the base calendar a resource calendar is based on.
    base_uid: Option<i64>,
    week: Vec<FileWeekDay>,
    /// The `Exceptions` collection, in file order.
    exceptions: Vec<FileException>,
    /// `WeekDay`s with `DayType` 0: exceptions as older files write them.
    old_exceptions: Vec<FileException>,
    /// Exceptions past [`MAX_STORED_EXCEPTIONS`], only counted.
    overflow: usize,
    work_weeks: usize,
}

impl FileCalendar {
    fn stored(&self) -> usize {
        self.exceptions.len().saturating_add(self.old_exceptions.len())
    }
}

/// The project-level fields read.
#[derive(Debug, Default)]
struct Fields {
    name: Option<String>,
    title: Option<String>,
    start: Option<String>,
    calendar_uid: Option<i64>,
    minutes_per_day: Option<i64>,
    minutes_per_week: Option<i64>,
    days_per_month: Option<i64>,
    from_finish: bool,
    currency: Option<String>,
    currency_symbol: Option<String>,
    currency_digits: Option<i64>,
    /// The type and effort-driven setting new tasks get ([`fields`]).
    defaults: fields::ProjectDefaults,
    /// `WeekStartDay`: 0 Sunday … 6 Saturday.
    week_start: Option<i64>,
}

#[derive(Debug, Default)]
struct FileProject {
    fields: Fields,
    tasks: Vec<FileTask>,
    task: Option<FileTask>,
    /// The earliest task start, for a file without a project start.
    first_start: Option<Date>,
    calendars: Vec<FileCalendar>,
    calendar: Option<FileCalendar>,
    weekday: Option<FileWeekDay>,
    exception: Option<FileException>,
    /// Exceptions held across every calendar, against [`MAX_STORED_EXCEPTIONS`].
    stored_exceptions: usize,
    time: Option<(String, String)>,
    /// The resources and assignments ([`resources`]).
    res: resources::Reader,
    custom_fields: usize,
    outline_codes: usize,
    /// Links kept, and links past [`mspdi_links::MAX_FILE_LINKS`], only counted.
    links: usize,
    links_overflow: usize,
}

fn is(path: &[String], pattern: &[&str]) -> bool {
    path.len() == pattern.len() && path.iter().zip(pattern).all(|(a, b)| a == b)
}

/// `path` is `pattern` and one more element; that element's name.
fn child<'a>(path: &'a [String], pattern: &[&str]) -> Option<&'a str> {
    match path.split_last() {
        Some((last, parent)) if is(parent, pattern) => Some(last),
        _ => None,
    }
}

const TASK: &[&str] = &["Project", "Tasks", "Task"];
const LINK: &[&str] = &["Project", "Tasks", "Task", "PredecessorLink"];
const CALENDAR: &[&str] = &["Project", "Calendars", "Calendar"];
const WEEKDAY: &[&str] = &["Project", "Calendars", "Calendar", "WeekDays", "WeekDay"];
const WORKING_TIME: &[&str] = &["Project", "Calendars", "Calendar", "WeekDays", "WeekDay", "WorkingTimes", "WorkingTime"];
const WEEKDAY_PERIOD: &[&str] = &["Project", "Calendars", "Calendar", "WeekDays", "WeekDay", "TimePeriod"];
const EXCEPTION: &[&str] = &["Project", "Calendars", "Calendar", "Exceptions", "Exception"];
const EXCEPTION_PERIOD: &[&str] = &["Project", "Calendars", "Calendar", "Exceptions", "Exception", "TimePeriod"];
const EXCEPTION_TIME: &[&str] = &["Project", "Calendars", "Calendar", "Exceptions", "Exception", "WorkingTimes", "WorkingTime"];
const WORK_WEEK: &[&str] = &["Project", "Calendars", "Calendar", "WorkWeeks", "WorkWeek"];

fn int(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// An `xs:boolean`: `1` or `true`.
fn truthy(text: &str) -> bool {
    matches!(text.trim(), "1" | "true")
}

/// Whether a number field holds something other than zero (or nothing).
fn nonzero(text: &str) -> bool {
    text.trim().parse::<f64>().is_ok_and(|n| n != 0.0)
}

/// The date of an `xs:dateTime` (`2026-11-02T08:00:00`).
fn date_of(text: &str) -> Option<Date> {
    text.trim().get(..10)?.parse().ok()
}

fn text_field(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

impl Visitor for FileProject {
    fn open(&mut self, path: &[String]) -> Result<(), ImportError> {
        if self.res.open(path) {
            return Ok(());
        }
        if is(path, TASK) {
            self.task = Some(FileTask::default());
        } else if let (Some(name), Some(task)) = (child(path, TASK), self.task.as_mut()) {
            let note = match name {
                "PredecessorLink" => {
                    task.row.open_link();
                    None
                }
                "Baseline" => Some(Note::Baseline),
                "ExtendedAttribute" | "EnterpriseExtendedAttribute" => Some(Note::CustomFields),
                "OutlineCode" => Some(Note::OutlineCodes),
                "TimephasedData" => Some(Note::Timephased),
                "Project" => Some(Note::Subproject),
                _ => None,
            };
            if let Some(n) = note {
                task.note(n);
            }
        } else if is(path, CALENDAR) {
            self.calendar = Some(FileCalendar::default());
        } else if is(path, WEEKDAY) {
            self.weekday = Some(FileWeekDay::default());
        } else if is(path, WORKING_TIME) || is(path, EXCEPTION_TIME) {
            self.time = Some(Default::default());
        } else if is(path, EXCEPTION) {
            self.exception = Some(FileException::default());
        } else if let Some(cal) = self.calendar.as_mut() {
            if is(path, WORK_WEEK) {
                cal.work_weeks = cal.work_weeks.saturating_add(1);
            }
        } else if child(path, &["Project", "ExtendedAttributes"]) == Some("ExtendedAttribute") {
            self.custom_fields = self.custom_fields.saturating_add(1);
        } else if child(path, &["Project", "OutlineCodes"]) == Some("OutlineCode") {
            self.outline_codes = self.outline_codes.saturating_add(1);
        }
        Ok(())
    }

    fn close(&mut self, path: &[String], text: &str) -> Result<(), ImportError> {
        if self.res.close(path, text) {
            return Ok(());
        }
        if is(path, TASK) {
            if let Some(t) = self.task.take() {
                if self.tasks.len() >= MAX_TASKS.saturating_add(1) {
                    return Err(ImportError::TooManyTasks);
                }
                self.tasks.push(t);
            }
        } else if is(path, LINK) {
            if let Some(t) = self.task.as_mut() {
                t.row.close_link(&mut self.links, &mut self.links_overflow);
            }
        } else if let Some(name) = child(path, LINK) {
            if let Some(t) = self.task.as_mut() {
                t.row.link_field(name, text);
            }
        } else if let Some(name) = child(path, TASK) {
            self.task_field(name, text);
        } else if is(path, WORKING_TIME) {
            if let (Some(t), Some(day)) = (self.time.take(), self.weekday.as_mut())
                && day.times.len() <= model::MAX_WORKING_TIMES
            {
                day.times.push(t);
            }
        } else if is(path, EXCEPTION_TIME) {
            if let (Some(t), Some(e)) = (self.time.take(), self.exception.as_mut())
                && e.times.len() <= model::MAX_WORKING_TIMES
            {
                e.times.push(t);
            }
        } else if let Some(name) = child(path, WORKING_TIME).or_else(|| child(path, EXCEPTION_TIME)) {
            if let Some(t) = self.time.as_mut() {
                match name {
                    "FromTime" => t.0 = text.trim().to_owned(),
                    "ToTime" => t.1 = text.trim().to_owned(),
                    _ => {}
                }
            }
        } else if let Some(name) = child(path, WEEKDAY_PERIOD) {
            if let Some(d) = self.weekday.as_mut() {
                match name {
                    "FromDate" => d.from = date_of(text),
                    "ToDate" => d.to = date_of(text),
                    _ => {}
                }
            }
        } else if is(path, WEEKDAY) {
            if let Some(d) = self.weekday.take() {
                self.close_weekday(d);
            }
        } else if let Some(name) = child(path, WEEKDAY) {
            if let Some(d) = self.weekday.as_mut() {
                match name {
                    "DayType" => d.day_type = int(text),
                    "DayWorking" => d.working = Some(truthy(text)),
                    _ => {}
                }
            }
        } else if let Some(name) = child(path, EXCEPTION_PERIOD) {
            if let Some(e) = self.exception.as_mut() {
                match name {
                    "FromDate" => e.from = date_of(text),
                    "ToDate" => e.to = date_of(text),
                    _ => {}
                }
            }
        } else if is(path, EXCEPTION) {
            if let Some(e) = self.exception.take() {
                self.store_exception(e, false);
            }
        } else if let Some(name) = child(path, EXCEPTION) {
            if let Some(e) = self.exception.as_mut() {
                exception_field(e, name, text);
            }
        } else if is(path, CALENDAR) {
            if let Some(c) = self.calendar.take() {
                self.close_calendar(c);
            }
        } else if let Some(name) = child(path, CALENDAR) {
            if let Some(c) = self.calendar.as_mut() {
                match name {
                    "UID" => c.uid = int(text),
                    "Name" => c.name = text_field(text).map(|n| n.chars().take(40).collect()),
                    "IsBaseCalendar" => c.base = truthy(text),
                    "BaseCalendarUID" => c.base_uid = int(text),
                    _ => {}
                }
            }
        } else if let Some(name) = child(path, &["Project"]) {
            self.project_field(name, text);
        }
        Ok(())
    }
}

fn exception_field(e: &mut FileException, name: &str, text: &str) {
    match name {
        "Name" => e.name = text.trim().chars().take(40).collect(),
        "EnteredByOccurrences" => e.by_occurrences = truthy(text),
        "Occurrences" => e.occurrences = int(text),
        "Type" => e.kind = int(text),
        "Period" => e.period = int(text),
        "DaysOfWeek" => e.days_of_week = int(text),
        "Month" => e.month = int(text),
        "MonthDay" => e.month_day = int(text),
        "MonthItem" => e.month_item = int(text),
        "MonthPosition" => e.month_position = int(text),
        "DayWorking" => e.working = Some(truthy(text)),
        _ => {}
    }
}

impl FileProject {
    /// A weekday of the calendar being read; `DayType` 0 is an exception over its `TimePeriod`.
    fn close_weekday(&mut self, d: FileWeekDay) {
        if d.day_type == Some(0) {
            let e = FileException { from: d.from, to: d.to, working: d.working, times: d.times, ..FileException::default() };
            self.store_exception(e, true);
        } else if let Some(cal) = self.calendar.as_mut()
            && cal.week.len() < 14
        {
            cal.week.push(d);
        }
    }

    /// Keeps an exception of the calendar being read, while the file's total allows.
    fn store_exception(&mut self, e: FileException, old: bool) {
        let Some(cal) = self.calendar.as_mut() else { return };
        if self.stored_exceptions >= MAX_STORED_EXCEPTIONS {
            cal.overflow = cal.overflow.saturating_add(1);
            return;
        }
        self.stored_exceptions += 1;
        if old {
            cal.old_exceptions.push(e);
        } else {
            cal.exceptions.push(e);
        }
    }

    /// Keeps a calendar read. A base calendar known not to be the project calendar keeps no
    /// exceptions: they are never imported. A resource calendar keeps its own.
    fn close_calendar(&mut self, mut c: FileCalendar) {
        if c.base && self.fields.calendar_uid.is_some() && c.uid != self.fields.calendar_uid {
            self.stored_exceptions = self.stored_exceptions.saturating_sub(c.stored());
            c.exceptions = Vec::new();
            c.old_exceptions = Vec::new();
        }
        if self.calendars.len() < MAX_CALENDARS {
            self.calendars.push(c);
        } else {
            self.stored_exceptions = self.stored_exceptions.saturating_sub(c.stored());
        }
    }

    fn project_field(&mut self, name: &str, text: &str) {
        let f = &mut self.fields;
        match name {
            "Name" => f.name = text_field(text),
            "Title" => f.title = text_field(text),
            "StartDate" => f.start = text_field(text),
            "CalendarUID" => f.calendar_uid = int(text),
            "MinutesPerDay" => f.minutes_per_day = int(text),
            "MinutesPerWeek" => f.minutes_per_week = int(text),
            "DaysPerMonth" => f.days_per_month = int(text),
            "ScheduleFromStart" => f.from_finish = !truthy(text),
            "CurrencyCode" => f.currency = text_field(text).map(|c| c.chars().take(20).collect()),
            "CurrencySymbol" => f.currency_symbol = text_field(text).map(|c| c.chars().take(20).collect()),
            "CurrencyDigits" => f.currency_digits = int(text),
            "WeekStartDay" => f.week_start = int(text),
            _ => {
                f.defaults.read(name, text);
            }
        }
    }

    fn task_field(&mut self, name: &str, text: &str) {
        let Some(t) = self.task.as_mut() else { return };
        let note = match name {
            "UID" => {
                t.uid = int(text);
                None
            }
            "ID" => {
                t.id = int(text);
                None
            }
            "Name" => {
                t.name = text.trim().to_owned();
                None
            }
            "OutlineLevel" => {
                t.level = int(text);
                None
            }
            "Duration" => {
                t.duration = Some(text.trim().to_owned());
                None
            }
            "DurationFormat" => {
                t.format = int(text);
                None
            }
            "IsNull" => {
                t.null = truthy(text);
                None
            }
            "Start" | "Finish" => {
                t.row.date_field(name, text);
                if let Some(d) = date_of(text).filter(|_| name == "Start") {
                    self.first_start = Some(self.first_start.map_or(d, |f| f.min(d)));
                }
                None
            }
            "Manual" => truthy(text).then_some(Note::Manual),
            "Recurring" => truthy(text).then_some(Note::Recurring),
            "IsSubproject" => truthy(text).then_some(Note::Subproject),
            "CalendarUID" => {
                t.calendar = int(text).filter(|c| *c != -1);
                None
            }
            "PercentComplete" | "PercentWorkComplete" | "PhysicalPercentComplete" => nonzero(text).then_some(Note::Progress),
            "ActualStart" | "ActualFinish" => text_field(text).map(|_| Note::Progress),
            "ActualDuration" | "ActualWork" | "ActualOvertimeWork" => duration_minutes(text).is_some_and(|m| m != 0).then_some(Note::Progress),
            "ActualCost" | "ActualOvertimeCost" => nonzero(text).then_some(Note::Progress),
            "Notes" => text_field(text).map(|_| Note::Notes),
            "Hyperlink" | "HyperlinkAddress" | "HyperlinkSubAddress" => text_field(text).map(|_| Note::Hyperlink),
            "Priority" => {
                t.priority = text_field(text).map(|p| p.chars().take(64).collect());
                None
            }
            "LevelingDelay" => {
                t.leveling_delay = text_field(text).map(|d| d.chars().take(64).collect());
                None
            }
            "LevelingDelayFormat" => {
                t.leveling_delay_format = int(text);
                None
            }
            _ => {
                t.fields.read(name, text);
                None
            }
        };
        if let Some(n) = note {
            t.note(n);
        }
    }

    fn into_import(mut self) -> Result<Import, ImportError> {
        let mut lines = Vec::new();
        let f = &self.fields;
        let name = f.title.clone().or_else(|| f.name.as_deref().map(|n| n.strip_suffix(".xml").unwrap_or(n).to_owned()));
        let start = f.start.as_deref().and_then(date_of).or(self.first_start);
        if start.is_none() {
            lines.push(line(Outcome::Converted, "Project start", "Project", "the file gives no start date; 1970-01-01 used"));
        }
        let mut p = Project::new(name.unwrap_or_else(|| "Imported project".into()), start.unwrap_or(Date::EPOCH));
        p.options = options(f, &mut lines);
        f.defaults.apply(&mut p.options, &mut lines);
        if f.from_finish {
            lines.push(line(
                Outcome::Converted,
                "Scheduled from the finish date",
                "Project",
                "scheduled from the start date instead; scheduling from the finish comes later",
            ));
        }
        p.currency = currency(f, &mut lines);
        p.calendar = self.calendar(&mut lines);
        let (tasks, task_lines, on, rows) = tasks(std::mem::take(&mut self.tasks), &p.options, p.start, p.currency.digits)?;
        lines.extend(task_lines);
        let resource_calendars = self.res.calendar_uids();
        lines.extend(self.other_calendars(&tasks, &on, &resource_calendars));
        let built = self.resource_calendars(&resource_calendars, &p.calendar);
        let count = tasks.len();
        p.next_id = tasks.iter().map(|t| t.id.0).max().unwrap_or(0).saturating_add(1);
        p.tasks = tasks;
        lines.extend(self.collections());
        lines.extend(outline_lines(&mut p));
        lines.extend(mspdi_links::links(&mut p, &rows, self.links_overflow));
        let uids: Vec<Option<i64>> = rows.iter().map(|r| r.uid).collect();
        lines.extend(resources::apply(&mut p, std::mem::take(&mut self.res), &uids, &built));
        lines.extend(mspdi_links::date_lines(&p, &rows));
        lines.sort_by_key(|l| l.outcome);
        Ok(Import { project: p, report: ImportReport { format: Format::Mspdi, tasks: count, lines } })
    }

    /// The index of the project calendar among the calendars read.
    fn project_calendar(&self) -> Option<usize> {
        let uid = self.fields.calendar_uid;
        self.calendars.iter().position(|c| c.uid.is_some() && c.uid == uid)
    }

    /// The project calendar: its working week and exceptions, with report lines about them.
    fn calendar(&self, lines: &mut Vec<ReportLine>) -> Calendar {
        let Some(cal) = self.project_calendar().and_then(|i| self.calendars.get(i)) else {
            lines.push(line(
                Outcome::Converted,
                "Project calendar",
                "Project",
                "the file names no project calendar it holds; the Standard week is used (Monday to Friday, 8:00–12:00 and 13:00–17:00)",
            ));
            return Calendar::default();
        };
        if cal.work_weeks > 0 {
            lines.push(line(
                Outcome::NotYetImported,
                "Work weeks",
                &plural(cal.work_weeks, "work week"),
                "work weeks come in a later version; the default week is used all year",
            ));
        }
        let mut c = week(cal, lines);
        // Newer files write each exception twice, as an Exception and as a DayType 0 WeekDay.
        let each = if cal.exceptions.is_empty() { &cal.old_exceptions } else { &cal.exceptions };
        let week_start = self.fields.week_start.filter(|d| (0..7).contains(d)).unwrap_or(0);
        let mut steps = 0u64;
        lines.extend(exception_days(&mut c, each, cal.overflow, week_start, &mut steps));
        c
    }

    /// A line for each calendar other than the project calendar: a task calendar names the tasks
    /// on it; a base calendar no task uses is dropped, and so is a resource calendar no resource
    /// uses. `on` is each task's calendar, by row; `resources` the calendars resources name.
    fn other_calendars(&self, tasks: &[Task], on: &[Option<i64>], resources: &BTreeSet<i64>) -> Vec<ReportLine> {
        let project = self.project_calendar();
        let project_uid = project.and_then(|i| self.calendars.get(i)).and_then(|c| c.uid);
        let label = |c: &FileCalendar| match c.uid {
            Some(uid) => format!("{} (UID {uid})", c.name.as_deref().unwrap_or("Calendar")),
            None => c.name.clone().unwrap_or_else(|| "A calendar".into()),
        };
        // Task calendars in the order of their first task, each with its tasks' rows.
        let mut used: Vec<(i64, Vec<usize>)> = Vec::new();
        for (row, uid) in on.iter().enumerate() {
            let Some(uid) = uid.filter(|u| Some(*u) != project_uid) else { continue };
            match used.iter_mut().find(|(u, _)| *u == uid) {
                Some((_, rows)) => rows.push(row),
                None => used.push((uid, vec![row])),
            }
        }
        let mut task_lines = Vec::new();
        for (uid, rows) in &used {
            let cal = self.calendars.iter().find(|c| c.uid == Some(*uid));
            let name = cal.map_or_else(|| format!("UID {uid}, not in the file"), label);
            task_lines.push(line(
                Outcome::Dropped,
                "Task calendar",
                &format!("{name}: {}", rows_named(tasks, rows)),
                "ProjectCraft schedules every task on the project calendar; these tasks are scheduled on it",
            ));
        }
        let others =
            self.calendars.iter().enumerate().filter(|(i, c)| Some(*i) != project && !c.uid.is_some_and(|u| used.iter().any(|(x, _)| *x == u)));
        let (mut base_lines, mut unused) = (Vec::new(), Vec::new());
        for (_, c) in others {
            if c.base {
                base_lines.push(line(Outcome::Dropped, "Base calendar", &label(c), "only the project calendar is imported; no task uses this one"));
            } else if !c.uid.is_some_and(|u| resources.contains(&u)) {
                unused.push(line(Outcome::Dropped, "Resource calendars", &label(c), "no resource uses this calendar"));
            }
        }
        let mut out = capped(task_lines, "Task calendar");
        out.extend(capped(base_lines, "Base calendar"));
        out.extend(capped(unused, "Resource calendars"));
        out
    }

    /// Each resource calendar the resources name, built on the project calendar `base`: its own
    /// weekdays (and those of another base calendar it is based on), and its exceptions worked
    /// out to their days. Exceptions are worked out against one budget of steps for the file.
    fn resource_calendars(&self, wanted: &BTreeSet<i64>, base: &Calendar) -> BTreeMap<i64, resources::BuiltCalendar> {
        let project = self.project_calendar().and_then(|i| self.calendars.get(i)).and_then(|c| c.uid);
        let week_start = self.fields.week_start.filter(|d| (0..7).contains(d)).unwrap_or(0);
        let mut steps = 0u64;
        let mut out = BTreeMap::new();
        // Base calendars by UID, the first of any repeated.
        let mut bases: HashMap<i64, &FileCalendar> = HashMap::new();
        for c in self.calendars.iter().filter(|c| c.base) {
            if let Some(u) = c.uid {
                bases.entry(u).or_insert(c);
            }
        }
        for c in self.calendars.iter().filter(|c| !c.base && c.uid.is_some_and(|u| wanted.contains(&u))) {
            let Some(uid) = c.uid else { continue };
            if out.contains_key(&uid) {
                continue;
            }
            let mut built = resources::BuiltCalendar::default();
            let mut week: [Option<Vec<WorkingTime>>; 7] = Default::default();
            // Another base calendar's own weekdays first, then the calendar's own on top.
            let other = c.base_uid.filter(|b| *b >= 0 && Some(*b) != project).and_then(|b| bases.get(&b).copied());
            built.other_base = other.is_some();
            for (inherited, cal) in other.into_iter().map(|o| (true, o)).chain(std::iter::once((false, c))) {
                for d in &cal.week {
                    let Some(i) = weekday_index(d) else { continue };
                    match (day_times(d), week.get_mut(i)) {
                        (Some(times), Some(slot)) => *slot = Some(times),
                        _ if !inherited => built.lines.push(line(
                            Outcome::Converted,
                            "Working times",
                            WEEKDAY_NAMES.get(i).copied().unwrap_or("A weekday"),
                            "unreadable, overlapping or out of order; the day follows the project calendar",
                        )),
                        _ => {}
                    }
                }
            }
            // A weekday the same as the project's follows it, so a later change there reaches it.
            for (i, day) in week.iter_mut().enumerate() {
                if day.as_deref().is_some_and(|t| t == base.weekday_times(i)) {
                    *day = None;
                }
            }
            if c.work_weeks > 0 {
                built.lines.push(line(
                    Outcome::NotYetImported,
                    "Work weeks",
                    &plural(c.work_weeks, "work week"),
                    "work weeks come in a later version; the default week is used all year",
                ));
            }
            let mut scratch = Calendar::default();
            let each = if c.exceptions.is_empty() { &c.old_exceptions } else { &c.exceptions };
            built.lines.extend(exception_days(&mut scratch, each, c.overflow, week_start, &mut steps));
            let exceptions = scratch.exceptions.keys().map(|d| (*d, scratch.times_on(*d).to_vec())).collect();
            built.calendar = ResourceCalendar { week, exceptions };
            out.insert(uid, built);
        }
        out
    }

    /// The project's custom field definitions, as report lines.
    fn collections(&self) -> Vec<ReportLine> {
        let mut out = Vec::new();
        if self.custom_fields > 0 {
            out.push(line(
                Outcome::Dropped,
                "Custom field definitions",
                &plural(self.custom_fields, "field"),
                "ProjectCraft doesn't hold custom fields yet",
            ));
        }
        if self.outline_codes > 0 {
            out.push(line(
                Outcome::Dropped,
                "Outline code definitions",
                &plural(self.outline_codes, "code"),
                "ProjectCraft doesn't hold custom outline codes yet",
            ));
        }
        out
    }
}

pub(crate) fn line(outcome: Outcome, feature: &str, place: &str, why: &str) -> ReportLine {
    ReportLine { outcome, feature: feature.into(), place: place.into(), why: why.into() }
}

pub(crate) fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// The project's currency: the file's `CurrencyCode` with its `CurrencySymbol` and
/// `CurrencyDigits` (<https://learn.microsoft.com/office-project/xml-data-interchange/currencydigits-element>),
/// as far as they can be used; pounds sterling for a file without one.
fn currency(f: &Fields, lines: &mut Vec<ReportLine>) -> Currency {
    let mut report = |why: String| lines.push(line(Outcome::Converted, "Currency", "Project", &why));
    let digits = match f.currency_digits {
        None => None,
        Some(d) => match u32::try_from(d).ok().filter(|d| *d <= MAX_CURRENCY_DIGITS) {
            Some(d) => Some(d),
            None => {
                report(format!("{d} decimal places is more than a currency has; the currency's own are used"));
                None
            }
        },
    };
    let symbol = f.currency_symbol.as_deref();
    let code = match f.currency.as_deref() {
        Some(code) => code,
        None if symbol.is_none() && digits.is_none() => return Currency::default(),
        None => {
            report(format!("the file gives no currency code; {DEFAULT_CURRENCY_CODE} is used with the file's symbol and decimal places"));
            DEFAULT_CURRENCY_CODE
        }
    };
    if let Ok(c) = Currency::new(code, symbol, digits) {
        return c;
    }
    if let Ok(c) = Currency::new(code, None, digits) {
        report(format!("the symbol `{}` could be mistaken for part of an amount or is too long; {} used", symbol.unwrap_or(""), c.symbol));
        return c;
    }
    report(format!("`{code}` isn't a currency code (three letters, such as GBP); the project is in pounds sterling"));
    Currency::default()
}

/// Hours per day and week and days per month, when the file's are in range.
fn options(f: &Fields, lines: &mut Vec<ReportLine>) -> Options {
    let d = Options::default();
    let pick = |v: Option<i64>, default: u32| v.and_then(|v| u32::try_from(v).ok()).unwrap_or(default);
    let o = Options {
        minutes_per_day: pick(f.minutes_per_day, d.minutes_per_day),
        minutes_per_week: pick(f.minutes_per_week, d.minutes_per_week),
        days_per_month: pick(f.days_per_month, d.days_per_month),
        ..d
    };
    if o.check().is_ok() {
        o
    } else {
        lines.push(line(
            Outcome::Converted,
            "Hours per day and week, days per month",
            "Project",
            "out of range (a day of 1 to 24 hours, a week of 1 to 168, a month of 1 to 31 days); 8, 40 and 20 used",
        ));
        d
    }
}

/// `08:00:00` as minutes since midnight; a `ToTime` of midnight is the end of the day.
fn clock(text: &str, is_end: bool) -> Option<u16> {
    let mut parts = text.trim().split(':');
    let num = |p: Option<&str>| p.filter(|s| (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())).and_then(|s| s.parse::<u16>().ok());
    let (h, m) = (num(parts.next())?, num(parts.next())?);
    let _seconds = parts.next();
    if parts.next().is_some() || h > 24 || m > 59 {
        return None;
    }
    let total = h * 60 + m;
    match total {
        0 if is_end => Some(MINUTES_IN_DAY),
        t if t <= MINUTES_IN_DAY => Some(t),
        _ => None,
    }
}

const WEEKDAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The working week of the file's project calendar. `DayType` 1 is Sunday, 2 Monday … 7 Saturday
/// (<https://learn.microsoft.com/office-project/xml-data-interchange/daytype-element>); a
/// weekday the file leaves out keeps the Standard calendar's day.
fn week(cal: &FileCalendar, lines: &mut Vec<ReportLine>) -> Calendar {
    let standard = Calendar::default();
    let mut times: [Vec<WorkingTime>; 7] = std::array::from_fn(|i| standard.weekday_times(i).to_vec());
    for d in &cal.week {
        let Some(i) = weekday_index(d) else { continue };
        match (day_times(d), times.get_mut(i)) {
            (Some(day), Some(slot)) => *slot = day,
            _ => lines.push(line(
                Outcome::Converted,
                "Working times",
                WEEKDAY_NAMES.get(i).copied().unwrap_or("A weekday"),
                "unreadable, overlapping or out of order; the Standard times are used",
            )),
        }
    }
    let mut c = Calendar::default();
    let working: [bool; 7] = std::array::from_fn(|i| times.get(i).is_some_and(|t| !t.is_empty()));
    if c.set_working(working).is_err() {
        lines.push(line(Outcome::Converted, "Project calendar", "Project", "no working weekday; the Standard week is used"));
        return Calendar::default();
    }
    for (i, t) in times.into_iter().enumerate() {
        if !t.is_empty() {
            // Checked above; a refusal leaves the Standard times.
            let _ = c.set_weekday_times(i, t);
        }
    }
    c
}

/// A weekday's index, Monday first, from its `DayType` (1 Sunday … 7 Saturday).
fn weekday_index(d: &FileWeekDay) -> Option<usize> {
    d.day_type.filter(|t| (1..=7).contains(t)).and_then(|t| usize::try_from((t + 5) % 7).ok())
}

/// A weekday's working times (none: non-working), or `None` if unreadable. A working day with no
/// times of its own has the default times.
fn day_times(d: &FileWeekDay) -> Option<Vec<WorkingTime>> {
    match d.working {
        Some(false) => Some(Vec::new()),
        _ if d.times.is_empty() => Some(model::DEFAULT_WORKING_TIMES.to_vec()),
        _ => d
            .times
            .iter()
            .map(|(from, to)| WorkingTime::new(clock(from, false)?, clock(to, true)?).ok())
            .collect::<Option<Vec<_>>>()
            .filter(|t| model::check_working_times(t).is_ok()),
    }
}

/// Minutes in an `xs:duration` (`PT16H0M0S`, `-PT1H30M`, `P1DT2H`, `PT0.5S`), rounded to the
/// nearest minute; `None` when unreadable, or when it has years or months (no fixed length).
pub(crate) fn duration_minutes(text: &str) -> Option<i64> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let t = t.strip_prefix('P')?;
    let (date, time) = match t.split_once('T') {
        Some((d, tm)) if !tm.is_empty() => (d, tm),
        Some(_) => return None,
        None => (t, ""),
    };
    if date.is_empty() && time.is_empty() {
        return None;
    }
    // Thousandths of a second.
    let mut ms: i128 = 0;
    for (part, units) in [(date, &[('Y', 0i128), ('M', 0), ('D', 86_400)][..]), (time, &[('H', 3_600), ('M', 60), ('S', 1)][..])] {
        let mut number = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                if number.len() >= 20 {
                    return None;
                }
                number.push(c);
                continue;
            }
            let &(_, seconds) = units.iter().find(|(u, _)| *u == c)?;
            let value = thousandths(&number, c == 'S' && seconds == 1)?;
            if seconds == 0 && value != 0 {
                return None;
            }
            ms = ms.checked_add(value.checked_mul(seconds)?)?;
            number.clear();
        }
        if !number.is_empty() {
            return None;
        }
    }
    let minutes = (ms + 30_000) / 60_000;
    let minutes = i64::try_from(minutes).unwrap_or(i64::MAX);
    Some(if negative { -minutes } else { minutes })
}

/// `12` or `1.5` as thousandths (`12000`, `1500`); a fraction only where `fraction` allows.
fn thousandths(number: &str, fraction: bool) -> Option<i128> {
    let (whole, frac) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() || (!fraction && number.contains('.')) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let whole: i128 = whole.parse().ok()?;
    let digits: String = frac.chars().chain("000".chars()).take(3).collect();
    whole.checked_mul(1000)?.checked_add(digits.parse::<i128>().ok()?)
}

/// The unit a `DurationFormat` shows, whether it is elapsed, and whether estimated
/// (<https://learn.microsoft.com/office-project/xml-data-interchange/durationformat-element>).
fn unit_of(format: Option<i64>) -> (DurationUnit, bool, bool) {
    let Some(f) = format else { return (DurationUnit::Days, false, false) };
    let estimated = (35..=52).contains(&f);
    let base = if estimated { f - 32 } else { f };
    let (unit, elapsed) = match base {
        3 => (DurationUnit::Minutes, false),
        4 => (DurationUnit::Minutes, true),
        5 => (DurationUnit::Hours, false),
        6 => (DurationUnit::Hours, true),
        8 => (DurationUnit::Days, true),
        9 => (DurationUnit::Weeks, false),
        10 => (DurationUnit::Weeks, true),
        11 => (DurationUnit::Months, false),
        12 => (DurationUnit::Months, true),
        _ => (DurationUnit::Days, false),
    };
    (unit, elapsed, estimated)
}

/// The project's tasks, the report lines about them, and by row each one's own calendar and its
/// links and dates in the file.
type Tasks = (Vec<Task>, Vec<ReportLine>, Vec<Option<i64>>, Vec<FileRow>);

/// The file's tasks as the project's, in the file's ID order. An elapsed duration is read as the
/// same number of working units.
fn tasks(file: Vec<FileTask>, options: &Options, start: Date, digits: u32) -> Result<Tasks, ImportError> {
    let mut lines = Vec::new();
    let (mut summary, mut blank) = (false, 0usize);
    let mut kept: Vec<FileTask> = Vec::with_capacity(file.len());
    for t in file {
        if t.uid == Some(0) {
            summary = true;
        } else if t.null {
            blank += 1;
        } else {
            kept.push(t);
        }
    }
    if summary {
        lines.push(line(Outcome::Dropped, "Project summary task", "UID 0", "ProjectCraft rolls the whole project up itself"));
    }
    if blank > 0 {
        lines.push(line(Outcome::Dropped, "Blank rows", &plural(blank, "row"), "ProjectCraft has no blank rows"));
    }
    if kept.len() > MAX_TASKS {
        return Err(ImportError::TooManyTasks);
    }
    if kept.iter().all(|t| t.id.is_some()) {
        kept.sort_by_key(|t| t.id);
    }
    // The file's Unique IDs, when every one is usable: positive, unique, and leaving room to add.
    let room = u32::MAX - u32::try_from(MAX_TASKS).unwrap_or(u32::MAX / 2);
    let mut seen = HashSet::new();
    let keep_uids = kept.iter().all(|t| t.uid.and_then(|u| u32::try_from(u).ok()).is_some_and(|u| u >= 1 && u < room && seen.insert(u)));
    if !keep_uids && !kept.is_empty() {
        lines.push(line(Outcome::Converted, "Unique IDs", "Tasks", "missing, repeated or out of range in the file; the tasks are numbered from 1"));
    }
    let day = options.day();
    let mut out = Vec::with_capacity(kept.len());
    let mut notes = Vec::with_capacity(kept.len());
    let mut field_notes = Vec::with_capacity(kept.len());
    let mut on = Vec::with_capacity(kept.len());
    let mut rows = Vec::with_capacity(kept.len());
    for (row, mut t) in kept.into_iter().enumerate() {
        let (unit, elapsed, estimated) = unit_of(t.format);
        let read =
            t.duration.as_deref().map(|d| duration_minutes(d).map(|m| if elapsed { mspdi_links::elapsed_to_working(m, unit, options) } else { m }));
        let duration = match read {
            None => {
                t.note(Note::NoDuration);
                day
            }
            Some(None) => {
                t.note(Note::BadDuration);
                day
            }
            Some(Some(m)) if m < 0 => {
                t.note(Note::NegativeDuration);
                0
            }
            Some(Some(m)) if m > MAX_MINUTES => {
                t.note(Note::LongDuration);
                MAX_MINUTES
            }
            Some(Some(m)) => m,
        };
        if elapsed {
            t.note(Note::Elapsed);
        }
        if estimated {
            t.note(Note::Estimated);
        }
        let id = match (keep_uids, t.uid.and_then(|u| u32::try_from(u).ok())) {
            (true, Some(u)) => u,
            _ => u32::try_from(row).unwrap_or(u32::MAX - 1).saturating_add(1),
        };
        let level = t.level.unwrap_or(1).clamp(1, i64::from(MAX_OUTLINE_LEVEL));
        let mut task = Task {
            id: TaskId(id),
            name: std::mem::take(&mut t.name),
            duration,
            duration_unit: unit,
            preds: Vec::new(),
            constraint: Constraint::Asap,
            outline_level: u32::try_from(level).unwrap_or(1),
            deadline: None,
            fixed_cost: 0,
            task_type: TaskType::FixedUnits,
            effort_driven: false,
            priority: model::DEFAULT_PRIORITY,
            leveling_delay: 0,
        };
        if let Some(text) = t.priority.as_deref() {
            match int(text).map(|p| p.clamp(0, i64::from(model::MAX_PRIORITY))) {
                Some(p) => {
                    task.priority = u32::try_from(p).unwrap_or(model::DEFAULT_PRIORITY);
                    if int(text) != Some(p) {
                        t.note(Note::Priority);
                    }
                }
                None => t.note(Note::Priority),
            }
        }
        if let Some(text) = t.leveling_delay.as_deref() {
            let (delay, exact) = leveling_delay(text, t.leveling_delay_format, options);
            task.leveling_delay = delay;
            if !exact {
                t.note(Note::LevelingDelay);
            }
        }
        field_notes.push(fields::apply(&t.fields, &mut task, start, digits));
        out.push(task);
        notes.push(t.notes);
        on.push(t.calendar);
        t.row.uid = t.uid;
        rows.push(std::mem::take(&mut t.row));
    }
    lines.extend(task_lines(&out, &notes));
    lines.extend(fields::lines(&out, &field_notes));
    Ok((out, lines, on, rows))
}

/// A task's `LevelingDelay`, in tenths of a minute as MSPDI's other spans are (`LinkLag`,
/// `TotalSlack`), shown in its `LevelingDelayFormat`: working minutes, and whether it was read
/// as it is. An elapsed delay is the same number of working units, as an elapsed lag is; a
/// percentage, or an unreadable one, is none; all within 0 to [`MAX_MINUTES`].
fn leveling_delay(text: &str, format: Option<i64>, options: &Options) -> (i64, bool) {
    let Some(tenths) = int(text).or_else(|| text.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f.round() as i64)) else {
        return (0, false);
    };
    let minutes = tenths.saturating_add(if tenths < 0 { -5 } else { 5 }) / 10;
    let base = format.map(|f| if (35..=52).contains(&f) { f - 32 } else { f });
    if matches!(base, Some(19 | 20)) {
        return (0, tenths == 0);
    }
    let (unit, elapsed, _) = unit_of(base);
    let working = if elapsed { mspdi_links::elapsed_to_working(minutes, unit, options) } else { minutes };
    let kept = working.clamp(0, MAX_MINUTES);
    (kept, kept == working && (!elapsed || minutes == 0))
}

/// Rows whose level breaks the outline rule are moved up to fit, each with a report line.
fn outline_lines(p: &mut Project) -> Vec<ReportLine> {
    let legal = Outline::of(p).levels;
    let mut moved = vec![0u32; p.tasks.len()];
    for ((t, l), m) in p.tasks.iter_mut().zip(legal).zip(moved.iter_mut()) {
        if t.outline_level != l {
            t.outline_level = l;
            *m = Note::Outline.bit();
        }
    }
    task_lines(&p.tasks, &moved)
}

/// One line per note, naming the first few tasks that have it.
fn task_lines(tasks: &[Task], notes: &[u32]) -> Vec<ReportLine> {
    let mut out = Vec::new();
    for n in Note::ALL {
        let rows: Vec<usize> = notes.iter().enumerate().filter(|(_, bits)| *bits & n.bit() != 0).map(|(i, _)| i).collect();
        if rows.is_empty() {
            continue;
        }
        let (outcome, feature, why) = n.line();
        out.push(line(outcome, feature, &rows_named(tasks, &rows), why));
    }
    out
}

/// The first few of these rows' tasks by ID and name, and how many more.
pub(crate) fn rows_named(tasks: &[Task], rows: &[usize]) -> String {
    let named: Vec<String> =
        rows.iter().filter_map(|&r| tasks.get(r).map(|t| format!("ID {} {}", r + 1, t.name.chars().take(40).collect::<String>()))).collect();
    named_list(named, rows.len(), "more task")
}

/// The first [`NAMED_IN_LINE`] of `names`, then "and n more …" for the rest of `total`.
fn named_list(names: Vec<String>, total: usize, more: &str) -> String {
    let mut place = names.into_iter().take(NAMED_IN_LINE).collect::<Vec<_>>().join(", ");
    if total > NAMED_IN_LINE {
        place = format!("{place} and {}", plural(total - NAMED_IN_LINE, more));
    }
    place
}

/// The first [`NAMED_CALENDARS`] lines, and one more line for the rest.
fn capped(mut lines: Vec<ReportLine>, feature: &str) -> Vec<ReportLine> {
    if lines.len() > NAMED_CALENDARS {
        let rest = lines.len() - NAMED_CALENDARS;
        let why = lines.get(NAMED_CALENDARS).map(|l| l.why.clone()).unwrap_or_default();
        lines.truncate(NAMED_CALENDARS);
        lines.push(line(Outcome::Dropped, feature, &plural(rest, "more calendar"), &why));
    }
    lines
}

/// `100000` as `100,000`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Why an exception gave no more days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// Its days ran out (or its occurrences).
    Done,
    /// Its pattern, dates or working times can't be read.
    Unreadable,
    /// The calendar holds [`model::MAX_EXCEPTIONS`] exception days.
    Full,
    /// [`MAX_EXCEPTION_STEPS`] days have been looked at.
    OutOfSteps,
}

/// Sets each exception's days on `c`, in file order (a later one wins on a shared day), with
/// report lines for those dropped. `overflow` exceptions were never kept; `week_start` is the
/// file's first day of the week (0 Sunday), for weekly exceptions every n weeks; `steps` the days
/// looked at so far, against [`MAX_EXCEPTION_STEPS`].
fn exception_days(c: &mut Calendar, each: &[FileException], overflow: usize, week_start: i64, steps: &mut u64) -> Vec<ReportLine> {
    let mut lines = Vec::new();
    let mut unreadable = Vec::new();
    let mut unreadable_count = 0usize;
    let mut stopped = None;
    for (n, e) in each.iter().enumerate() {
        match expand(c, e, week_start, steps) {
            Stop::Done => {}
            Stop::Unreadable => {
                unreadable_count += 1;
                if unreadable.len() < NAMED_IN_LINE {
                    unreadable.push(if e.name.is_empty() { format!("Exception {}", n + 1) } else { e.name.clone() });
                }
            }
            s => {
                stopped = Some((s, each.len() - n));
                break;
            }
        }
    }
    if unreadable_count > 0 {
        lines.push(line(
            Outcome::Dropped,
            "Calendar exceptions",
            &named_list(unreadable, unreadable_count, "more exception"),
            "their dates, recurrence or working times can't be read; those days follow the working week",
        ));
    }
    match stopped {
        Some((Stop::Full, left)) => lines.push(line(
            Outcome::Dropped,
            "Calendar exceptions",
            &plural(left, "exception"),
            &format!(
                "a calendar holds at most {} exception days; the days after that are dropped and follow the working week",
                thousands(model::MAX_EXCEPTIONS)
            ),
        )),
        Some((_, left)) => lines.push(line(
            Outcome::Dropped,
            "Calendar exceptions",
            &plural(left, "exception"),
            "these recurring exceptions run too long to work out; their days follow the working week",
        )),
        None => {}
    }
    if overflow > 0 {
        lines.push(line(
            Outcome::Dropped,
            "Calendar exceptions",
            &plural(overflow, "exception"),
            &format!("the file holds more than {} exceptions; the rest are dropped", thousands(MAX_STORED_EXCEPTIONS)),
        ));
    }
    lines
}

/// The working times an exception gives its days (none: non-working), or `None` if unreadable.
/// `DayWorking` missing means working when the exception has working times.
fn exception_times(e: &FileException) -> Option<Vec<WorkingTime>> {
    if !e.working.unwrap_or(!e.times.is_empty()) {
        return Some(Vec::new());
    }
    if e.times.is_empty() {
        return Some(model::DEFAULT_WORKING_TIMES.to_vec());
    }
    e.times
        .iter()
        .map(|(from, to)| WorkingTime::new(clock(from, false)?, clock(to, true)?).ok())
        .collect::<Option<Vec<_>>>()
        .filter(|t| model::check_working_times(t).is_ok())
}

/// MSPDI's weekday index: 0 Sunday … 6 Saturday.
fn mspdi_weekday(d: Date) -> i64 {
    (i64::from(d.weekday()) + 1) % 7
}

/// Months since year 0: `y * 12 + m - 1`.
fn month_index(d: Date) -> i64 {
    let (y, m, _) = d.ymd();
    i64::from(y) * 12 + i64::from(m) - 1
}

/// The `position` (0 first … 3 fourth, 4 last) day of month `index` that is a `item` (0 day,
/// 1 weekday, 2 weekend day, 3 Sunday … 9 Saturday). Looks at up to 31 days, counted in `steps`.
fn nth_in_month(index: i64, item: i64, position: i64, steps: &mut u64) -> Option<Date> {
    let y = i32::try_from(index.div_euclid(12)).ok()?;
    let m = u32::try_from(index.rem_euclid(12) + 1).ok()?;
    let fits = |d: Date| {
        let w = mspdi_weekday(d);
        match item {
            0 => true,
            1 => (1..=5).contains(&w),
            2 => w == 0 || w == 6,
            i => w == i - 3,
        }
    };
    let mut found = None;
    let mut seen = 0i64;
    for day in 1..=31 {
        let Some(d) = Date::from_ymd(y, m, day) else { break };
        *steps = steps.saturating_add(1);
        if fits(d) {
            if seen == position {
                return Some(d);
            }
            seen += 1;
            found = Some(d);
        }
    }
    if position == 4 { found } else { None }
}

/// Sets `e`'s days on `c`. Daily (types 1, 7 and 9) every `Period` days; weekly (6 and 8) on its
/// `DaysOfWeek` every `Period` weeks; monthly (4, 5) every `Period` months; yearly (2, 3) every
/// year; all within its `TimePeriod`, and no more than its `Occurrences` when entered by them.
fn expand(c: &mut Calendar, e: &FileException, week_start: i64, steps: &mut u64) -> Stop {
    let Some(times) = exception_times(e) else { return Stop::Unreadable };
    let Some(from) = e.from else { return Stop::Unreadable };
    let limit = e.occurrences.filter(|n| e.by_occurrences && *n >= 1).and_then(|n| usize::try_from(n).ok());
    let to = match (e.to, limit) {
        (Some(to), _) => to,
        // Entered by occurrences without an end: as far as the occurrences go.
        (None, Some(_)) => Date::from_ymd(9999, 12, 31).unwrap_or(from),
        (None, None) => from,
    };
    if to < from {
        return Stop::Unreadable;
    }
    let every = e.period.filter(|p| *p >= 1).unwrap_or(1);
    let mut count = 0usize;
    // Sets one day; `Some` stops the exception.
    let mut emit = |d: Date, steps: &mut u64| -> Option<Stop> {
        *steps = steps.saturating_add(1);
        if *steps > MAX_EXCEPTION_STEPS {
            return Some(Stop::OutOfSteps);
        }
        if d < from {
            return None;
        }
        if d > to {
            return Some(Stop::Done);
        }
        if c.set_exception(d, times.clone()).is_err() {
            return Some(Stop::Full);
        }
        count += 1;
        limit.is_some_and(|l| count >= l).then_some(Stop::Done)
    };
    match e.kind {
        None | Some(1 | 7 | 9) => {
            let mut d = from;
            loop {
                if let Some(s) = emit(d, steps) {
                    return s;
                }
                match d.add_days(every) {
                    Some(next) => d = next,
                    None => return Stop::Done,
                }
            }
        }
        Some(6 | 8) => {
            let mask = e.days_of_week.unwrap_or(1 << mspdi_weekday(from));
            if !(1..=127).contains(&mask) {
                return Stop::Unreadable;
            }
            let back = (mspdi_weekday(from) - week_start).rem_euclid(7);
            let Some(mut week) = from.add_days(-back) else { return Stop::Unreadable };
            loop {
                for k in 0..7 {
                    let Some(d) = week.add_days(k) else { return Stop::Done };
                    if d > to {
                        return Stop::Done;
                    }
                    if mask & (1 << mspdi_weekday(d)) != 0 {
                        if let Some(s) = emit(d, steps) {
                            return s;
                        }
                    } else {
                        *steps = steps.saturating_add(1);
                    }
                }
                match every.checked_mul(7).and_then(|n| week.add_days(n)) {
                    Some(next) => week = next,
                    None => return Stop::Done,
                }
            }
        }
        Some(kind @ 2..=5) => {
            let yearly = kind <= 3;
            let by_day = kind == 2 || kind == 4;
            let day = e.month_day.filter(|d| (1..=31).contains(d));
            let item = e.month_item.filter(|i| (0..=9).contains(i));
            let position = e.month_position.filter(|p| (0..=4).contains(p));
            if (by_day && day.is_none()) || (!by_day && (item.is_none() || position.is_none())) {
                return Stop::Unreadable;
            }
            let (mut index, step) = if yearly {
                let Some(month) = e.month.filter(|m| (0..=11).contains(m)) else { return Stop::Unreadable };
                let (y, _, _) = from.ymd();
                (i64::from(y) * 12 + month, 12)
            } else {
                (month_index(from), every)
            };
            let last = month_index(to);
            while index <= last {
                *steps = steps.saturating_add(1);
                if *steps > MAX_EXCEPTION_STEPS {
                    return Stop::OutOfSteps;
                }
                let d = match (day, item, position) {
                    (Some(day), _, _) if by_day => i32::try_from(index.div_euclid(12))
                        .ok()
                        .and_then(|y| Date::from_ymd(y, u32::try_from(index.rem_euclid(12) + 1).ok()?, u32::try_from(day).ok()?)),
                    (_, Some(item), Some(position)) => nth_in_month(index, item, position, steps),
                    _ => None,
                };
                // A month without the day (the 31st of April) has no occurrence.
                if let Some(d) = d
                    && let Some(s) = emit(d, steps)
                {
                    return s;
                }
                index = index.saturating_add(step);
            }
            Stop::Done
        }
        Some(_) => Stop::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_xs_duration_to_the_minute() {
        for (text, minutes) in [
            ("PT16H0M0S", Some(960)),
            ("PT8H", Some(480)),
            ("PT0H0M0S", Some(0)),
            ("PT1H30M", Some(90)),
            ("PT29S", Some(0)),
            ("PT30S", Some(1)),
            ("PT0.5S", Some(0)),
            ("PT90.5S", Some(2)),
            ("P1DT2H", Some(1560)),
            ("P0Y0M2D", Some(2880)),
            ("-PT1H", Some(-60)),
            (" PT8H0M0S ", Some(480)),
            ("P1Y", None),
            ("P1M", None),
            ("PT1.5H", None),
            ("PT", None),
            ("P", None),
            ("8h", None),
            ("PTH", None),
            ("PT8", None),
            ("PT99999999999999999999H", Some(i64::MAX)),
            ("PT999999999999999999999H", None),
            ("", None),
        ] {
            assert_eq!(duration_minutes(text), minutes, "{text}");
        }
    }

    #[test]
    fn duration_formats_give_the_unit_shown() {
        assert_eq!(unit_of(Some(7)), (DurationUnit::Days, false, false));
        assert_eq!(unit_of(Some(39)), (DurationUnit::Days, false, true));
        assert_eq!(unit_of(Some(5)), (DurationUnit::Hours, false, false));
        assert_eq!(unit_of(Some(8)), (DurationUnit::Days, true, false));
        assert_eq!(unit_of(Some(44)), (DurationUnit::Months, true, true));
        assert_eq!(unit_of(Some(53)), (DurationUnit::Days, false, false));
        assert_eq!(unit_of(None), (DurationUnit::Days, false, false));
        assert_eq!(unit_of(Some(-4)), (DurationUnit::Days, false, false));
    }

    #[test]
    fn counts_are_written_with_thousands_separators() {
        for (n, text) in [(0, "0"), (999, "999"), (1000, "1,000"), (100_000, "100,000"), (1_234_567, "1,234,567")] {
            assert_eq!(thousands(n), text);
        }
    }

    #[test]
    fn clock_times_read_with_midnight_ending_the_day() {
        assert_eq!(clock("08:00:00", false), Some(480));
        assert_eq!(clock("17:30:00", true), Some(1050));
        assert_eq!(clock("00:00:00", true), Some(1440));
        assert_eq!(clock("00:00:00", false), Some(0));
        assert_eq!(clock("24:00:00", true), Some(1440));
        for bad in ["", "8", "25:00:00", "08:60:00", "x:00", "08:00:00:00", "123:00"] {
            assert_eq!(clock(bad, true), None, "{bad}");
        }
    }
}
