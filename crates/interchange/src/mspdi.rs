//! Microsoft Project XML (MSPDI) import, from Microsoft's published schema documentation
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/project-element>).
//!
//! Imported so far: the project's name and start date, hours per day and week and days per month,
//! the project calendar's working week (its `WeekDays` with `DayType` 1 to 7 and their
//! `WorkingTimes`), and each task's name, outline level and duration (in the unit its
//! `DurationFormat` gives). The project summary task (UID 0) is skipped: ProjectCraft rolls the
//! project up itself. Everything else that the file holds and the project can't is a report line.
//!
//! Durations are `xs:duration` values; Project writes them in hours, minutes and seconds
//! (`PT16H0M0S` is two 8-hour days). A day component (`P1D`) is 24 hours, as `xs:duration` has
//! it; years and months are refused as unreadable, having no fixed length.

use std::collections::HashSet;

use crate::model::{
    self, Calendar, Constraint, DEFAULT_CURRENCY_CODE, Date, DurationUnit, MAX_MINUTES, MAX_OUTLINE_LEVEL, MAX_TASKS, MINUTES_IN_DAY, Options,
    Outline, Project, Task, TaskId, TaskType, WorkingTime,
};
use crate::xml::{self, Visitor};
use crate::{Format, Import, ImportError, ImportReport, Outcome, ReportLine};

/// The MSPDI namespace Microsoft Project writes on its root `Project` element.
pub(crate) const NAMESPACE: &str = "http://schemas.microsoft.com/project";
/// Calendars kept for reading; more are only counted.
const MAX_CALENDARS: usize = 10_000;
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
    Links,
    Constraint,
    Deadline,
    TaskType,
    EffortDriven,
    FixedCost,
    TaskCalendar,
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
    const ALL: [Note; 26] = [
        Note::NoDuration,
        Note::BadDuration,
        Note::NegativeDuration,
        Note::LongDuration,
        Note::Elapsed,
        Note::Estimated,
        Note::Outline,
        Note::Manual,
        Note::Recurring,
        Note::Links,
        Note::Constraint,
        Note::Deadline,
        Note::TaskType,
        Note::EffortDriven,
        Note::FixedCost,
        Note::TaskCalendar,
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
                "read as the same amount of working time; elapsed durations are converted on the calendar in a later version",
            ),
            Note::Estimated => (Dropped, "Estimated duration (?)", "ProjectCraft has no estimate mark; the duration is kept"),
            Note::Outline => (Converted, "Outline level", "deeper than the row above allows; moved up to fit the outline"),
            Note::Manual => (Converted, "Manually scheduled", "auto-scheduled: ProjectCraft schedules every task"),
            Note::Recurring => (Converted, "Recurring task", "imported as a summary task with its occurrences as subtasks"),
            Note::Links => (NotYetImported, "Links (predecessors)", "links come in a later version; the task is scheduled without them"),
            Note::Constraint => (NotYetImported, "Constraint", "constraints come in a later version; the task is As Soon As Possible"),
            Note::Deadline => (NotYetImported, "Deadline", "deadlines come in a later version"),
            Note::TaskType => (NotYetImported, "Task type", "task types come in a later version; the task is Fixed Units"),
            Note::EffortDriven => (NotYetImported, "Effort-driven", "comes in a later version; the task is not effort-driven"),
            Note::FixedCost => (NotYetImported, "Fixed cost", "costs come in a later version"),
            Note::TaskCalendar => (Dropped, "Task calendar", "ProjectCraft schedules tasks on the project calendar"),
            Note::Progress => (Dropped, "Progress (actuals, % complete)", "tracking comes in a later version"),
            Note::Baseline => (Dropped, "Baselines", "baselines come in a later version"),
            Note::Notes => (Dropped, "Notes", "ProjectCraft doesn't hold task notes yet"),
            Note::Hyperlink => (Dropped, "Hyperlink", "ProjectCraft doesn't hold hyperlinks yet"),
            Note::CustomFields => (Dropped, "Custom fields", "ProjectCraft doesn't hold custom fields yet"),
            Note::OutlineCodes => (Dropped, "Outline codes", "ProjectCraft doesn't hold custom outline codes yet"),
            Note::Priority => (Dropped, "Priority", "ProjectCraft doesn't level resources yet, so priority has no use"),
            Note::LevelingDelay => (Dropped, "Leveling delay", "ProjectCraft doesn't level resources yet"),
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
    /// [`Note`] bits.
    notes: u32,
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
}

#[derive(Debug, Default)]
struct FileCalendar {
    uid: Option<i64>,
    base: bool,
    week: Vec<FileWeekDay>,
    exceptions: usize,
    work_weeks: usize,
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
    schedule_options: bool,
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
    time: Option<(String, String)>,
    resources: usize,
    resource_uid: Option<i64>,
    assignments: usize,
    assignment_resource: Option<i64>,
    custom_fields: usize,
    outline_codes: usize,
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
const CALENDAR: &[&str] = &["Project", "Calendars", "Calendar"];
const WEEKDAY: &[&str] = &["Project", "Calendars", "Calendar", "WeekDays", "WeekDay"];
const WORKING_TIME: &[&str] = &["Project", "Calendars", "Calendar", "WeekDays", "WeekDay", "WorkingTimes", "WorkingTime"];
const EXCEPTION: &[&str] = &["Project", "Calendars", "Calendar", "Exceptions", "Exception"];
const WORK_WEEK: &[&str] = &["Project", "Calendars", "Calendar", "WorkWeeks", "WorkWeek"];
const RESOURCE: &[&str] = &["Project", "Resources", "Resource"];
const ASSIGNMENT: &[&str] = &["Project", "Assignments", "Assignment"];

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
        if is(path, TASK) {
            self.task = Some(FileTask::default());
        } else if let (Some(name), Some(task)) = (child(path, TASK), self.task.as_mut()) {
            let note = match name {
                "PredecessorLink" => Some(Note::Links),
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
        } else if is(path, WORKING_TIME) {
            self.time = Some(Default::default());
        } else if let Some(cal) = self.calendar.as_mut() {
            if is(path, EXCEPTION) {
                cal.exceptions = cal.exceptions.saturating_add(1);
            } else if is(path, WORK_WEEK) {
                cal.work_weeks = cal.work_weeks.saturating_add(1);
            }
        } else if is(path, RESOURCE) {
            self.resource_uid = None;
        } else if is(path, ASSIGNMENT) {
            self.assignment_resource = None;
        } else if child(path, &["Project", "ExtendedAttributes"]) == Some("ExtendedAttribute") {
            self.custom_fields = self.custom_fields.saturating_add(1);
        } else if child(path, &["Project", "OutlineCodes"]) == Some("OutlineCode") {
            self.outline_codes = self.outline_codes.saturating_add(1);
        }
        Ok(())
    }

    fn close(&mut self, path: &[String], text: &str) -> Result<(), ImportError> {
        if is(path, TASK) {
            if let Some(t) = self.task.take() {
                if self.tasks.len() >= MAX_TASKS.saturating_add(1) {
                    return Err(ImportError::TooManyTasks);
                }
                self.tasks.push(t);
            }
        } else if let Some(name) = child(path, TASK) {
            self.task_field(name, text);
        } else if is(path, WORKING_TIME) {
            if let (Some(t), Some(day)) = (self.time.take(), self.weekday.as_mut())
                && day.times.len() <= model::MAX_WORKING_TIMES
            {
                day.times.push(t);
            }
        } else if let Some(name) = child(path, WORKING_TIME) {
            if let Some(t) = self.time.as_mut() {
                match name {
                    "FromTime" => t.0 = text.trim().to_owned(),
                    "ToTime" => t.1 = text.trim().to_owned(),
                    _ => {}
                }
            }
        } else if is(path, WEEKDAY) {
            if let (Some(d), Some(cal)) = (self.weekday.take(), self.calendar.as_mut())
                && cal.week.len() < 14
            {
                cal.week.push(d);
            }
        } else if let Some(name) = child(path, WEEKDAY) {
            if let Some(d) = self.weekday.as_mut() {
                match name {
                    "DayType" => d.day_type = int(text),
                    "DayWorking" => d.working = Some(truthy(text)),
                    _ => {}
                }
            }
        } else if is(path, CALENDAR) {
            if let Some(c) = self.calendar.take()
                && self.calendars.len() < MAX_CALENDARS
            {
                self.calendars.push(c);
            }
        } else if let Some(name) = child(path, CALENDAR) {
            if let Some(c) = self.calendar.as_mut() {
                match name {
                    "UID" => c.uid = int(text),
                    "IsBaseCalendar" => c.base = truthy(text),
                    _ => {}
                }
            }
        } else if is(path, RESOURCE) {
            // UID 0 is Project's own placeholder, not a resource.
            if self.resource_uid != Some(0) {
                self.resources = self.resources.saturating_add(1);
            }
        } else if child(path, RESOURCE) == Some("UID") {
            self.resource_uid = int(text);
        } else if is(path, ASSIGNMENT) {
            // A task without resources has an assignment to resource -65535.
            if !self.assignment_resource.is_some_and(|r| r < 0) {
                self.assignments = self.assignments.saturating_add(1);
            }
        } else if child(path, ASSIGNMENT) == Some("ResourceUID") {
            self.assignment_resource = int(text);
        } else if let Some(name) = child(path, &["Project"]) {
            self.project_field(name, text);
        }
        Ok(())
    }
}

impl FileProject {
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
            "CurrencyCode" => f.currency = text_field(text),
            "DefaultTaskType" => f.schedule_options |= int(text).is_some_and(|t| t != 0),
            "NewTasksEffortDriven" => f.schedule_options |= truthy(text),
            _ => {}
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
            "Start" => {
                if let Some(d) = date_of(text) {
                    self.first_start = Some(self.first_start.map_or(d, |f| f.min(d)));
                }
                None
            }
            "Manual" => truthy(text).then_some(Note::Manual),
            "Recurring" => truthy(text).then_some(Note::Recurring),
            "IsSubproject" => truthy(text).then_some(Note::Subproject),
            "ConstraintType" => int(text).is_some_and(|c| c != 0).then_some(Note::Constraint),
            "Deadline" => text_field(text).map(|_| Note::Deadline),
            "Type" => int(text).is_some_and(|c| c != 0).then_some(Note::TaskType),
            "EffortDriven" => truthy(text).then_some(Note::EffortDriven),
            "FixedCost" => nonzero(text).then_some(Note::FixedCost),
            "CalendarUID" => int(text).is_some_and(|c| c != -1).then_some(Note::TaskCalendar),
            "PercentComplete" | "PercentWorkComplete" | "PhysicalPercentComplete" => nonzero(text).then_some(Note::Progress),
            "ActualStart" | "ActualFinish" => text_field(text).map(|_| Note::Progress),
            "Notes" => text_field(text).map(|_| Note::Notes),
            "Hyperlink" | "HyperlinkAddress" | "HyperlinkSubAddress" => text_field(text).map(|_| Note::Hyperlink),
            "Priority" => int(text).is_some_and(|p| p != 500).then_some(Note::Priority),
            "LevelingDelay" => nonzero(text).then_some(Note::LevelingDelay),
            _ => None,
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
        if f.from_finish {
            lines.push(line(
                Outcome::Converted,
                "Scheduled from the finish date",
                "Project",
                "scheduled from the start date instead; scheduling from the finish comes later",
            ));
        }
        if f.schedule_options {
            lines.push(line(
                Outcome::NotYetImported,
                "Default task type, new tasks effort-driven",
                "Project",
                "these schedule options come with task types in a later version",
            ));
        }
        if let Some(code) = f.currency.as_deref().filter(|c| !c.eq_ignore_ascii_case(DEFAULT_CURRENCY_CODE)) {
            let code: String = code.chars().take(10).collect();
            lines.push(line(
                Outcome::NotYetImported,
                "Currency",
                &code,
                "the currency comes with costs in a later version; the project is in pounds sterling",
            ));
        }
        p.calendar = self.calendar(&mut lines);
        let (tasks, task_lines) = tasks(std::mem::take(&mut self.tasks), &p.options)?;
        lines.extend(task_lines);
        let count = tasks.len();
        p.next_id = tasks.iter().map(|t| t.id.0).max().unwrap_or(0).saturating_add(1);
        p.tasks = tasks;
        lines.extend(self.collections());
        lines.extend(outline_lines(&mut p));
        lines.sort_by_key(|l| l.outcome);
        Ok(Import { project: p, report: ImportReport { format: Format::Mspdi, tasks: count, lines } })
    }

    /// The project calendar's working week; the rest of the calendars as report lines.
    fn calendar(&self, lines: &mut Vec<ReportLine>) -> Calendar {
        let uid = self.fields.calendar_uid;
        let index = self.calendars.iter().position(|c| c.uid.is_some() && c.uid == uid);
        let project = index.and_then(|i| self.calendars.get(i));
        let others = self.calendars.iter().enumerate().filter(|(i, _)| Some(*i) != index).map(|(_, c)| c);
        let (bases, resources) = others.fold((0usize, 0usize), |(b, r), c| if c.base { (b + 1, r) } else { (b, r + 1) });
        if bases > 0 {
            lines.push(line(
                Outcome::Dropped,
                "Other base calendars",
                &plural(bases, "calendar"),
                "only the project calendar is imported; every task is scheduled on it",
            ));
        }
        if resources > 0 {
            lines.push(line(
                Outcome::NotYetImported,
                "Resource calendars",
                &plural(resources, "calendar"),
                "resource calendars come with resources in a later version",
            ));
        }
        let Some(cal) = project else {
            lines.push(line(
                Outcome::Converted,
                "Project calendar",
                "Project",
                "the file names no project calendar it holds; the Standard week is used (Monday to Friday, 8:00–12:00 and 13:00–17:00)",
            ));
            return Calendar::default();
        };
        let exceptions = cal.exceptions.saturating_add(cal.week.iter().filter(|d| d.day_type == Some(0)).count());
        if exceptions > 0 {
            lines.push(line(
                Outcome::NotYetImported,
                "Calendar exceptions",
                &plural(exceptions, "exception"),
                "holidays and other exceptions come in a later version; those days follow the working week",
            ));
        }
        if cal.work_weeks > 0 {
            lines.push(line(
                Outcome::NotYetImported,
                "Work weeks",
                &plural(cal.work_weeks, "work week"),
                "work weeks come in a later version; the default week is used all year",
            ));
        }
        week(cal, lines)
    }

    /// Resources, assignments and the project's custom field definitions, as report lines.
    fn collections(&self) -> Vec<ReportLine> {
        let mut out = Vec::new();
        if self.resources > 0 {
            out.push(line(Outcome::NotYetImported, "Resources", &plural(self.resources, "resource"), "resources come in a later version"));
        }
        if self.assignments > 0 {
            out.push(line(
                Outcome::NotYetImported,
                "Assignments",
                &plural(self.assignments, "assignment"),
                "assignments come with resources in a later version",
            ));
        }
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

fn line(outcome: Outcome, feature: &str, place: &str, why: &str) -> ReportLine {
    ReportLine { outcome, feature: feature.into(), place: place.into(), why: why.into() }
}

fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
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
        let Some(i) = d.day_type.filter(|t| (1..=7).contains(t)).map(|t| ((t + 5) % 7) as usize) else { continue };
        let day: Option<Vec<WorkingTime>> = match d.working {
            Some(false) => Some(Vec::new()),
            _ if d.times.is_empty() => Some(model::DEFAULT_WORKING_TIMES.to_vec()),
            _ => d
                .times
                .iter()
                .map(|(from, to)| WorkingTime::new(clock(from, false)?, clock(to, true)?).ok())
                .collect::<Option<Vec<_>>>()
                .filter(|t| model::check_working_times(t).is_ok()),
        };
        match (day, times.get_mut(i)) {
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

/// The file's tasks as the project's, in the file's ID order, with the report lines about them.
fn tasks(file: Vec<FileTask>, options: &Options) -> Result<(Vec<Task>, Vec<ReportLine>), ImportError> {
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
    for (row, mut t) in kept.into_iter().enumerate() {
        let (unit, elapsed, estimated) = unit_of(t.format);
        let duration = match t.duration.as_deref().map(duration_minutes) {
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
        out.push(Task {
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
        });
        notes.push(t.notes);
    }
    lines.extend(task_lines(&out, &notes));
    Ok((out, lines))
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
        let named: Vec<String> = rows
            .iter()
            .take(NAMED_IN_LINE)
            .filter_map(|&r| tasks.get(r).map(|t| format!("ID {} {}", r + 1, t.name.chars().take(40).collect::<String>())))
            .collect();
        let mut place = named.join(", ");
        if rows.len() > NAMED_IN_LINE {
            place = format!("{place} and {}", plural(rows.len() - NAMED_IN_LINE, "more task"));
        }
        let (outcome, feature, why) = n.line();
        out.push(line(outcome, feature, &place, why));
    }
    out
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
