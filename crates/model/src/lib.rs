//! ProjectCraft project model: a project's tasks, the links between them, its working calendar, and
//! the critical-path scheduler ([`schedule`]).
//!
//! Everything here is plain data with edit primitives; undo and commands live in the engine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod assignment;
mod cost;
mod duration;
mod outline;
mod resource;
mod rows;
mod schedule;
mod task_type;
mod view;
mod working_time;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub use assignment::{
    Assignment, AssignmentWork, MAX_ASSIGNMENTS, MAX_NAMES_TEXT, MAX_QUANTITY, MAX_TASK_ASSIGNMENTS, Units, assignment_work, default_units,
    parse_assignment_units, quantity_text,
};
pub use cost::{Currency, DEFAULT_CURRENCY_CODE, KNOWN_CURRENCIES, MAX_CURRENCY_DIGITS, MAX_CURRENCY_SYMBOL, assignment_cost, div_half_even};
pub use duration::{DurationUnit, duration_text, lag_text, parse_duration, parse_lag, parse_work};
pub use outline::{CIRCULAR_OUTLINE, MAX_OUTLINE_LEVEL, Outline};
pub use projectcraft_time::Date;
pub use resource::{
    AccrueAt, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS, FULL_TIME, MAX_MONEY, MAX_RESOURCE_TEXT, MAX_RESOURCES, MAX_UNITS, Resource,
    ResourceField, ResourceId, ResourceType, UNITS_PER_PERCENT, check_money, check_name, format_money, material_rate_text, parse_material_rate,
    parse_money, parse_units, parse_units_as, units_text, units_text_as,
};
pub use schedule::{Driver, Moment, Schedule, TaskDates, schedule};
pub use task_type::{MAX_WORK, TaskType};
pub use view::{Column, Columns};
pub use working_time::{
    DEFAULT_WORKING_TIMES, MAX_WORKING_TIMES, MINUTES_IN_DAY, Options, UnitsFormat, WorkingTime, check_working_times, clock, minutes_of, parse_clock,
    parse_working_time,
};

/// Longest duration or lag accepted in days (about 400 years of working days): keeps all
/// arithmetic in range.
pub const MAX_DAYS: i64 = 100_000;
/// Longest duration or lag accepted, in working minutes: [`MAX_DAYS`] days of 24 hours, so a
/// day count within [`MAX_DAYS`] fits at any hours per day.
pub const MAX_MINUTES: i64 = MAX_DAYS * MINUTES_IN_DAY as i64;
/// Most tasks in one project. ponytail: a cap, not a measured limit; raise with a benchmark.
pub const MAX_TASKS: usize = 100_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ModelError {
    #[error("no task {0}")]
    NoTask(TaskId),
    #[error("the links make a loop through task {0}")]
    Cycle(TaskId),
    #[error("{0}")]
    Invalid(String),
}

/// A task's Unique ID: permanent for the life of the project, never reused. Links and commands use
/// it; the ID column shows the row number instead ([`Project::row_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(pub u32);

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How a link ties its successor to its predecessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LinkKind {
    /// Finish-to-start: the successor starts when the predecessor finishes.
    #[default]
    FS,
    SS,
    FF,
    SF,
}

/// A predecessor of the task that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub pred: TaskId,
    #[serde(default)]
    pub kind: LinkKind,
    /// Working minutes; negative is lead.
    #[serde(default)]
    pub lag: i64,
    /// The unit the lag was typed in, to show it the same way. A file without one has days.
    #[serde(default, skip_serializing_if = "DurationUnit::is_days")]
    pub lag_unit: DurationUnit,
}

/// A date rule on a task's start or finish, as Microsoft Project's Constraint Type field.
/// Constraints win over links. Written to the file as `"ASAP"` or `{"SNET": "2026-01-12"}`;
/// a file without one is As Soon As Possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Constraint {
    /// No constraint: as early as the links allow.
    #[default]
    #[serde(rename = "ASAP")]
    Asap,
    #[serde(rename = "SNET")]
    StartNoEarlierThan(Date),
    #[serde(rename = "FNET")]
    FinishNoEarlierThan(Date),
    #[serde(rename = "SNLT")]
    StartNoLaterThan(Date),
    #[serde(rename = "FNLT")]
    FinishNoLaterThan(Date),
    #[serde(rename = "MSO")]
    MustStartOn(Date),
    #[serde(rename = "MFO")]
    MustFinishOn(Date),
}

impl Constraint {
    /// Every type's short code, ASAP first.
    pub const CODES: [&'static str; 7] = ["ASAP", "SNET", "FNET", "SNLT", "FNLT", "MSO", "MFO"];

    /// Builds a constraint from its short code (any case) and date. ASAP takes no date and ignores one given.
    pub fn from_code(code: &str, date: Option<Date>) -> Result<Constraint, ModelError> {
        let code = code.trim().to_ascii_uppercase();
        let make: fn(Date) -> Constraint = match code.as_str() {
            "ASAP" => return Ok(Constraint::Asap),
            "SNET" => Constraint::StartNoEarlierThan,
            "FNET" => Constraint::FinishNoEarlierThan,
            "SNLT" => Constraint::StartNoLaterThan,
            "FNLT" => Constraint::FinishNoLaterThan,
            "MSO" => Constraint::MustStartOn,
            "MFO" => Constraint::MustFinishOn,
            _ => {
                let code: String = code.chars().take(20).collect();
                return Err(ModelError::Invalid(format!("unknown constraint type `{code}`; expected one of {}", Constraint::CODES.join(", "))));
            }
        };
        date.map(make).ok_or_else(|| ModelError::Invalid(format!("{} needs a date", make(Date::EPOCH).name())))
    }

    pub fn code(self) -> &'static str {
        match self {
            Constraint::Asap => "ASAP",
            Constraint::StartNoEarlierThan(_) => "SNET",
            Constraint::FinishNoEarlierThan(_) => "FNET",
            Constraint::StartNoLaterThan(_) => "SNLT",
            Constraint::FinishNoLaterThan(_) => "FNLT",
            Constraint::MustStartOn(_) => "MSO",
            Constraint::MustFinishOn(_) => "MFO",
        }
    }

    /// The name Microsoft Project's Constraint Type field shows.
    pub fn name(self) -> &'static str {
        match self {
            Constraint::Asap => "As Soon As Possible",
            Constraint::StartNoEarlierThan(_) => "Start No Earlier Than",
            Constraint::FinishNoEarlierThan(_) => "Finish No Earlier Than",
            Constraint::StartNoLaterThan(_) => "Start No Later Than",
            Constraint::FinishNoLaterThan(_) => "Finish No Later Than",
            Constraint::MustStartOn(_) => "Must Start On",
            Constraint::MustFinishOn(_) => "Must Finish On",
        }
    }

    /// The constraint date; `None` for ASAP.
    pub fn date(self) -> Option<Date> {
        match self {
            Constraint::Asap => None,
            Constraint::StartNoEarlierThan(d)
            | Constraint::FinishNoEarlierThan(d)
            | Constraint::StartNoLaterThan(d)
            | Constraint::FinishNoLaterThan(d)
            | Constraint::MustStartOn(d)
            | Constraint::MustFinishOn(d) => Some(d),
        }
    }

    pub fn is_asap(&self) -> bool {
        *self == Constraint::Asap
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub name: String,
    /// Working minutes; 0 is a milestone. A day of duration is the project's hours per day
    /// ([`Options::minutes_per_day`]).
    pub duration: i64,
    /// The unit the duration was typed in, to show it the same way (`4h` stays `4h`). A file
    /// without one has days.
    #[serde(default, skip_serializing_if = "DurationUnit::is_days")]
    pub duration_unit: DurationUnit,
    #[serde(default)]
    pub preds: Vec<Link>,
    #[serde(default, skip_serializing_if = "Constraint::is_asap")]
    pub constraint: Constraint,
    /// Depth in the outline, 1 at the top ([`Outline`]). A file without one is level 1.
    #[serde(default = "top_level", skip_serializing_if = "is_top_level")]
    pub outline_level: u32,
    /// A target finish, as Microsoft Project's Deadline field. Not a constraint: it never moves the
    /// task, but caps its late finish, so finishing after it gives negative slack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<Date>,
    /// A cost of the task itself, not of a resource, in minor units of the project currency, as
    /// Microsoft Project's Fixed Cost field. A file without one has none.
    #[serde(default, skip_serializing_if = "is_no_cost")]
    pub fixed_cost: i64,
    /// Which of duration, units and work stays when another changes ([`TaskType`]). A file
    /// without one has Fixed Units.
    #[serde(default, skip_serializing_if = "TaskType::is_fixed_units")]
    pub task_type: TaskType,
    /// Keeps the task's work as resources are added or removed ([`Project::is_effort_driven`]).
    /// Always on for Fixed Work. A file without it has it off.
    #[serde(default, skip_serializing_if = "is_false")]
    pub effort_driven: bool,
}

fn is_no_cost(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn top_level() -> u32 {
    1
}

fn is_top_level(l: &u32) -> bool {
    *l == 1
}

/// Most exceptions one calendar holds: about 270 years of one a day. Keeps a hostile file or
/// command from growing the calendar without bound.
pub const MAX_EXCEPTIONS: usize = 100_000;

/// Which days are worked, and when: working weekdays with their **working times**, and
/// **exceptions** (days made non-working, a holiday, or working with their own times, an overtime
/// Saturday 9:00–13:00), as Microsoft Project's Change Working Time. An exception wins over its
/// weekday. A day with no working times is non-working.
///
/// `working` and `exceptions` say which days are worked (the scheduler counts those); the times
/// are read through [`Calendar::weekday_times`] and [`Calendar::times_on`]. A working day without
/// times of its own has the default times ([`DEFAULT_WORKING_TIMES`]), so the two always agree.
///
/// Written to the file as `{"working": [Mon…Sun], "times": [[{"from": 480, "to": 720}, …] × 7],
/// "exceptions": [{"date": "2026-12-25", "working": false}, {"date": "2027-01-09", "working":
/// true, "times": [{"from": 540, "to": 780}]}]}`. Files without `times` load with the default
/// times on every working day. Files from before exceptions held `"holidays": ["2026-12-25"]`;
/// those load as non-working exceptions and are written back as exceptions.
///
/// ponytail: one project calendar; work weeks, resource and task calendars come later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "CalendarFile", try_from = "CalendarFile")]
pub struct Calendar {
    /// Monday first.
    pub working: [bool; 7],
    /// Exception days: `true` worked, `false` not worked, whatever the weekday says.
    pub exceptions: BTreeMap<Date, bool>,
    /// Each weekday's working times when they differ from the default; empty means the default.
    week_times: [Vec<WorkingTime>; 7],
    /// A working exception's times when they differ from the default.
    exception_times: BTreeMap<Date, Vec<WorkingTime>>,
}

/// The calendar as the file holds it.
#[derive(Serialize, Deserialize)]
struct CalendarFile {
    working: [bool; 7],
    /// Each weekday's working times, Monday first; a weekday with none is non-working. Absent in
    /// older files: every working weekday has the default times.
    #[serde(default)]
    times: Option<[Vec<WorkingTime>; 7]>,
    #[serde(default)]
    exceptions: Vec<ExceptionFile>,
    /// Before exceptions: non-working days. Read, never written.
    #[serde(default, skip_serializing)]
    holidays: BTreeSet<Date>,
}

#[derive(Serialize, Deserialize)]
struct ExceptionFile {
    date: Date,
    #[serde(default)]
    working: bool,
    /// A working exception's times; absent means the default times.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    times: Vec<WorkingTime>,
}

/// `times`, or nothing when they are the default (how the calendar stores them).
fn own_times(times: Vec<WorkingTime>) -> Vec<WorkingTime> {
    if times == DEFAULT_WORKING_TIMES { Vec::new() } else { times }
}

impl TryFrom<CalendarFile> for Calendar {
    type Error = ModelError;

    /// Refuses working times out of order, overlapping or outside the day, and times on a
    /// non-working exception.
    fn try_from(f: CalendarFile) -> Result<Calendar, ModelError> {
        let mut c = Calendar { working: f.working, ..Calendar::default() };
        if let Some(times) = f.times {
            for (i, day) in times.into_iter().enumerate() {
                check_working_times(&day)?;
                if let (Some(w), Some(t)) = (c.working.get_mut(i), c.week_times.get_mut(i)) {
                    *w = !day.is_empty();
                    *t = own_times(day);
                }
            }
        }
        c.exceptions = f.holidays.into_iter().map(|d| (d, false)).collect();
        for e in f.exceptions {
            check_working_times(&e.times)?;
            if !e.working && !e.times.is_empty() {
                return Err(ModelError::Invalid(format!("the non-working exception on {} has working times", e.date)));
            }
            c.exceptions.insert(e.date, e.working);
            c.exception_times.remove(&e.date);
            let times = own_times(e.times);
            if !times.is_empty() {
                c.exception_times.insert(e.date, times);
            }
        }
        Ok(c)
    }
}

impl From<Calendar> for CalendarFile {
    fn from(c: Calendar) -> CalendarFile {
        let times = std::array::from_fn(|i| c.weekday_times(i).to_vec());
        let exceptions = c
            .exceptions
            .iter()
            .map(|(&date, &working)| ExceptionFile {
                date,
                working,
                times: c.exception_times.get(&date).filter(|_| working).cloned().unwrap_or_default(),
            })
            .collect();
        CalendarFile { working: c.working, times: Some(times), exceptions, holidays: BTreeSet::new() }
    }
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar {
            working: [true, true, true, true, true, false, false],
            exceptions: BTreeMap::new(),
            week_times: Default::default(),
            exception_times: BTreeMap::new(),
        }
    }
}

impl Calendar {
    pub fn is_workday(&self, d: Date) -> bool {
        self.exceptions.get(&d).copied().unwrap_or_else(|| self.works_weekday(d))
    }

    /// Whether `d`'s weekday is worked, ignoring exceptions.
    pub fn works_weekday(&self, d: Date) -> bool {
        self.working.get(usize::from(d.weekday())).copied().unwrap_or(false)
    }

    /// The working times of weekday `i` (0 is Monday); none when it is not worked.
    pub fn weekday_times(&self, i: usize) -> &[WorkingTime] {
        match (self.working.get(i), self.week_times.get(i)) {
            (Some(true), Some(t)) if !t.is_empty() => t,
            (Some(true), _) => &DEFAULT_WORKING_TIMES,
            _ => &[],
        }
    }

    /// The working times on `d`: its exception's, else its weekday's; none on a day off.
    pub fn times_on(&self, d: Date) -> &[WorkingTime] {
        match self.exceptions.get(&d) {
            Some(true) => self.exception_times.get(&d).map_or(&DEFAULT_WORKING_TIMES[..], Vec::as_slice),
            Some(false) => &[],
            None => self.weekday_times(usize::from(d.weekday())),
        }
    }

    /// Sets the working weekdays (Monday first). Refuses a week with no working day, even when
    /// working exceptions exist: such a calendar would run out of days after its last exception.
    /// A weekday made working gets the default times; one made non-working loses its times.
    pub fn set_working(&mut self, working: [bool; 7]) -> Result<(), ModelError> {
        if !working.contains(&true) {
            return Err(ModelError::Invalid("a calendar needs at least one working weekday".into()));
        }
        for ((old, new), times) in self.working.iter().zip(working).zip(self.week_times.iter_mut()) {
            if *old != new {
                times.clear();
            }
        }
        self.working = working;
        Ok(())
    }

    /// Sets weekday `i`'s working times (0 is Monday); none makes it non-working. Refuses times
    /// out of order, overlapping or outside the day, and a week left with no working day.
    pub fn set_weekday_times(&mut self, i: usize, times: Vec<WorkingTime>) -> Result<(), ModelError> {
        check_working_times(&times)?;
        let mut working = self.working;
        let w = working.get_mut(i).ok_or_else(|| ModelError::Invalid(format!("no weekday {i}")))?;
        *w = !times.is_empty();
        if !working.contains(&true) {
            return Err(ModelError::Invalid("a calendar needs at least one working weekday".into()));
        }
        self.working = working;
        if let Some(t) = self.week_times.get_mut(i) {
            *t = own_times(times);
        }
        Ok(())
    }

    /// Makes `date` a working (`true`, with the default times) or non-working day, replacing any
    /// exception already on it.
    pub fn add_exception(&mut self, date: Date, working: bool) -> Result<(), ModelError> {
        self.set_exception(date, if working { DEFAULT_WORKING_TIMES.to_vec() } else { Vec::new() })
    }

    /// Makes `date` an exception with these working times (none: a non-working day), replacing
    /// any exception already on it. Refuses times out of order, overlapping or outside the day.
    pub fn set_exception(&mut self, date: Date, times: Vec<WorkingTime>) -> Result<(), ModelError> {
        check_working_times(&times)?;
        if self.exceptions.len() >= MAX_EXCEPTIONS && !self.exceptions.contains_key(&date) {
            return Err(ModelError::Invalid(format!("a calendar holds at most {MAX_EXCEPTIONS} exceptions")));
        }
        self.exceptions.insert(date, !times.is_empty());
        self.exception_times.remove(&date);
        let times = own_times(times);
        if !times.is_empty() {
            self.exception_times.insert(date, times);
        }
        Ok(())
    }

    /// Removes the exception on `date`; the day goes back to its weekday.
    pub fn remove_exception(&mut self, date: Date) -> Result<(), ModelError> {
        self.exception_times.remove(&date);
        self.exceptions.remove(&date).map(|_| ()).ok_or_else(|| ModelError::Invalid(format!("there is no exception on {date}")))
    }

    /// Working minutes on `d`: none on a day off.
    pub fn minutes_on(&self, d: Date) -> i64 {
        i64::from(minutes_of(self.times_on(d)))
    }

    /// Working minutes of weekday `i` (0 is Monday), ignoring exceptions.
    fn weekday_minutes(&self, i: usize) -> i64 {
        i64::from(minutes_of(self.weekday_times(i)))
    }

    /// Working minutes on or after `from` and before `to`; 0 when `to` is not after `from`. Whole
    /// weeks are counted arithmetically, then each exception in the range corrects its day, so
    /// any range in years 1–9999 is quick.
    pub fn minutes_between(&self, from: Date, to: Date) -> i64 {
        let n = to.days_since(from);
        if n <= 0 {
            return 0;
        }
        let per_week: i64 = (0..7).map(|i| self.weekday_minutes(i)).sum();
        let (weeks, rest) = (n / 7, n % 7);
        let tail: i64 = (0..rest).filter_map(|k| from.add_days(weeks * 7 + k)).map(|d| self.weekday_minutes(usize::from(d.weekday()))).sum();
        let correction: i64 =
            self.exceptions.range(from..to).map(|(d, _)| self.minutes_on(*d) - self.weekday_minutes(usize::from(d.weekday()))).sum();
        weeks * per_week + tail + correction
    }

    /// The working days from `from` on, each with the working minutes before it (counted from
    /// `from`), until working minute `last` is covered: the table a minute offset is turned into a
    /// date with. `None` when the calendar runs out of working days, past year 9999, or past
    /// `max_days` working days.
    pub fn working_days(&self, from: Date, last: i64, max_days: usize) -> Option<Vec<(Date, i64)>> {
        let mut out = Vec::new();
        let mut before = 0i64;
        let mut push = |d: Date, m: i64| {
            if m > 0 && before <= last {
                out.push((d, before));
                before += m;
            }
            before > last || out.len() > max_days
        };
        if !self.working.contains(&true) {
            // Only the working exceptions are worked (a damaged file; commands refuse this week).
            for (d, _) in self.exceptions.range(from..).filter(|(_, w)| **w) {
                if push(*d, self.minutes_on(*d)) {
                    break;
                }
            }
        } else {
            let mut d = from;
            // Terminates: at least one weekday is worked, exceptions are finite, and add_days stops at year 9999.
            while !push(d, self.minutes_on(d)) {
                d = d.add_days(1)?;
            }
        }
        (before > last && out.len() <= max_days).then_some(out)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub start: Date,
    #[serde(default)]
    pub calendar: Calendar,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub next_id: u32,
    /// The Resource Sheet's rows, top to bottom. A file without them has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    /// The Unique ID the next resource gets; repaired on load so none is reused.
    #[serde(default, skip_serializing_if = "is_first_id")]
    pub next_resource_id: u32,
    /// Which resources work on which tasks, in the order they were assigned. A file without them
    /// has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignments: Vec<Assignment>,
    /// Hours per day and week, days per month. A file without them has the defaults (8, 40, 20).
    #[serde(default)]
    pub options: Options,
    /// The one currency every amount is in. A file without one is in pounds sterling.
    #[serde(default, skip_serializing_if = "Currency::is_default")]
    pub currency: Currency,
    /// The task sheet's columns after ID and Task Name. A file without them shows every column.
    #[serde(default, skip_serializing_if = "Columns::is_default")]
    pub columns: Columns,
}

fn is_first_id(n: &u32) -> bool {
    *n <= 1
}

impl Project {
    pub fn new(name: impl Into<String>, start: Date) -> Self {
        Project {
            name: name.into(),
            start,
            calendar: Calendar::default(),
            tasks: Vec::new(),
            next_id: 1,
            resources: Vec::new(),
            next_resource_id: 1,
            assignments: Vec::new(),
            options: Options::default(),
            currency: Currency::default(),
            columns: Columns::default(),
        }
    }

    pub fn task(&self, id: TaskId) -> Result<&Task, ModelError> {
        self.tasks.iter().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    pub fn task_mut(&mut self, id: TaskId) -> Result<&mut Task, ModelError> {
        self.tasks.iter_mut().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    /// Adds a task of `duration` working minutes at the end, at the top level, and returns its Unique ID.
    pub fn add_task(&mut self, name: impl Into<String>, duration: i64) -> Result<TaskId, ModelError> {
        let task = self.new_task(name.into(), duration, 1)?;
        let id = task.id;
        self.tasks.push(task);
        Ok(id)
    }

    /// The 1-based row a task is on: its ID, which renumbers as rows are inserted, moved or deleted.
    pub fn row_of(&self, id: TaskId) -> Option<usize> {
        self.tasks.iter().position(|t| t.id == id).map(|i| i.saturating_add(1))
    }

    /// The task on a 1-based row.
    pub fn task_at_row(&self, row: u32) -> Option<TaskId> {
        let i = usize::try_from(row).ok()?.checked_sub(1)?;
        self.tasks.get(i).map(|t| t.id)
    }

    /// The project's day of duration in working minutes (its hours per day), always in range.
    pub fn minutes_per_day(&self) -> i64 {
        self.options.day()
    }

    /// `days` days of duration or lag in working minutes, at the project's hours per day; refused
    /// past [`MAX_DAYS`].
    pub fn days_to_minutes(&self, days: i64) -> Result<i64, ModelError> {
        Ok(check_days(days)? * self.minutes_per_day())
    }

    /// Predecessors as the Predecessors column shows them: by row ID. A link to a missing task
    /// (only a damaged file could hold one) shows as `?`.
    pub fn preds_text_rows(&self, preds: &[Link]) -> String {
        let one = |l: &Link| match self.row_of(l.pred) {
            Some(row) => link_text(row, l, &self.options),
            None => link_text('?', l, &self.options),
        };
        preds.iter().map(one).collect::<Vec<_>>().join(", ")
    }

    /// Reads the Predecessors column: row IDs, returned as links by Unique ID.
    pub fn parse_preds_rows(&self, text: &str) -> Result<Vec<Link>, ModelError> {
        parse_preds(text, &self.options)?
            .into_iter()
            .map(|l| {
                let pred = self.task_at_row(l.pred.0).ok_or_else(|| ModelError::Invalid(format!("no task with ID {}", l.pred)))?;
                Ok(Link { pred, ..l })
            })
            .collect()
    }

    /// The Predecessors column's text with each row ID turned into its task's Unique ID and every
    /// lag kept exactly as typed (`2SS+1.33d`), for `task.setPredecessors`. Refuses what
    /// [`Project::parse_preds_rows`] refuses.
    pub fn preds_rows_to_ids(&self, text: &str) -> Result<String, ModelError> {
        self.parse_preds_rows(text)?;
        let parts = text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()).map(|part| {
            let digits = part.find(|c: char| !c.is_ascii_digit()).unwrap_or(part.len());
            let (row, rest) = (part.get(..digits).unwrap_or(""), part.get(digits..).unwrap_or(""));
            let id =
                row.parse::<u32>().ok().and_then(|r| self.task_at_row(r)).ok_or_else(|| ModelError::Invalid(format!("no task with ID {row}")))?;
            Ok(format!("{id}{rest}"))
        });
        Ok(parts.collect::<Result<Vec<_>, ModelError>>()?.join(", "))
    }

    /// Replaces a task's predecessors, refusing links to missing tasks, to itself, between a
    /// summary and its own subtask, or that close a loop.
    pub fn set_preds(&mut self, id: TaskId, preds: Vec<Link>) -> Result<(), ModelError> {
        self.task(id)?;
        for l in &preds {
            self.task(l.pred)?;
            if l.pred == id {
                return Err(ModelError::Cycle(id));
            }
            check_minutes(l.lag.saturating_abs())?;
        }
        self.check_outline_links(id, &preds)?;
        let old = std::mem::replace(&mut self.task_mut(id)?.preds, preds);
        if let Err(e) = schedule::order(self) {
            self.task_mut(id)?.preds = old;
            return Err(e);
        }
        Ok(())
    }
}

/// A day count in range: 0 to [`MAX_DAYS`].
pub fn check_days(n: i64) -> Result<i64, ModelError> {
    if (0..=MAX_DAYS).contains(&n) { Ok(n) } else { Err(ModelError::Invalid(format!("{n} is not between 0 and {MAX_DAYS} days"))) }
}

/// A duration or lag size in working minutes in range: 0 to [`MAX_MINUTES`].
pub fn check_minutes(n: i64) -> Result<i64, ModelError> {
    if (0..=MAX_MINUTES).contains(&n) {
        Ok(n)
    } else {
        Err(ModelError::Invalid(format!("{n} minutes is not between 0 and {MAX_MINUTES} minutes ({MAX_DAYS} days of 24 hours)")))
    }
}

/// Working minutes as days of `minutes_per_day`, to at most two decimal places: `3`, `1.5`,
/// `-2`, `2.13`. Integer arithmetic, rounding half away from zero.
pub fn days_text(minutes: i64, minutes_per_day: i64) -> String {
    let day = i128::from(minutes_per_day.max(1));
    let m = i128::from(minutes);
    let hundredths = (m.abs() * 200 + day) / (2 * day);
    let sign = if m < 0 && hundredths > 0 { "-" } else { "" };
    let (whole, frac) = (hundredths / 100, hundredths % 100);
    match frac {
        0 => format!("{sign}{whole}"),
        f if f % 10 == 0 => format!("{sign}{whole}.{}", f / 10),
        f => format!("{sign}{whole}.{f:02}"),
    }
}

/// Predecessors by Unique ID: `3, 5SS+2, 7FF-1, 8FS+4h`, each lag in the unit it was typed in
/// at the project's `options` (to two decimals; days without a suffix).
pub fn preds_text(preds: &[Link], options: &Options) -> String {
    preds.iter().map(|l| link_text(l.pred, l, options)).collect::<Vec<_>>().join(", ")
}

/// One link, its predecessor written as `pred`.
fn link_text(pred: impl fmt::Display, l: &Link, options: &Options) -> String {
    let kind = match l.kind {
        LinkKind::FS if l.lag == 0 => String::new(),
        k => format!("{k:?}"),
    };
    format!("{pred}{kind}{}", lag_text(l.lag, l.lag_unit, options))
}

/// Reads the Predecessors text by Unique ID, as commands take it: `3, 5ss+2, 7FF-1d, 8FS+4h`. A
/// lag is a number with an optional sign and unit ([`parse_lag`]): days when it has none.
pub fn parse_preds(text: &str, options: &Options) -> Result<Vec<Link>, ModelError> {
    let bad = |p: &str| ModelError::Invalid(format!("not a predecessor: {}", p.chars().take(40).collect::<String>()));
    let mut out = Vec::new();
    for part in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let digits = part.find(|c: char| !c.is_ascii_digit()).unwrap_or(part.len());
        let (id, rest) = (part.get(..digits).unwrap_or(""), part.get(digits..).unwrap_or("").trim_start());
        let pred = TaskId(id.parse().map_err(|_| bad(part))?);
        let kind = match rest.get(..2).map(str::to_ascii_uppercase).as_deref() {
            Some("FS") => Some(LinkKind::FS),
            Some("SS") => Some(LinkKind::SS),
            Some("FF") => Some(LinkKind::FF),
            Some("SF") => Some(LinkKind::SF),
            _ => None,
        };
        let lag = if kind.is_some() { rest.get(2..).unwrap_or("") } else { rest }.trim();
        let (lag, lag_unit) = if lag.is_empty() { (0, DurationUnit::Days) } else { parse_lag(lag, options)? };
        out.push(Link { pred, kind: kind.unwrap_or_default(), lag, lag_unit });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A day of duration at the default 8 hours.
    const D: i64 = 480;

    /// The default options: 8 hours a day, 40 a week, 20 days a month.
    const O: Options = Options {
        minutes_per_day: 480,
        minutes_per_week: 2400,
        days_per_month: 20,
        task_type: crate::TaskType::FixedUnits,
        effort_driven: false,
        units_format: crate::UnitsFormat::Percentage,
    };

    fn link(pred: u32, kind: LinkKind, lag: i64, lag_unit: DurationUnit) -> Link {
        Link { pred: TaskId(pred), kind, lag, lag_unit }
    }

    #[test]
    fn predecessor_text_round_trips() {
        let links = parse_preds("3, 5ss+2, 7FF-1d;9", &O).unwrap();
        assert_eq!(links[1].lag, 2 * D, "lags are working minutes");
        assert_eq!(preds_text(&links, &O), "3, 5SS+2, 7FF-1, 9");
        assert!(parse_preds("x", &O).is_err());
        assert!(parse_preds("1FS+x", &O).is_err());
        assert!(parse_preds("1d", &O).is_err(), "a unit needs a number");
        assert_eq!(parse_preds("1FS+999999999999999999999", &O).unwrap()[0].lag, MAX_MINUTES, "clamped, as a duration is");
        assert_eq!(parse_preds("", &O).unwrap(), vec![]);
        assert!(parse_preds("1FS+3ed", &O).unwrap_err().to_string().contains("elapsed lags aren't supported yet"));
        assert!(parse_preds("1SS+50%", &O).unwrap_err().to_string().contains("percent lags aren't supported yet"));
        // Part-day lags keep the unit typed.
        let part = parse_preds("2SS+4h, 3FS-1.5d, 4FF+30m, 5SF+1w, 6+1mo, 7 FS + 2 hrs", &O).unwrap();
        assert_eq!(
            part,
            vec![
                link(2, LinkKind::SS, 240, DurationUnit::Hours),
                link(3, LinkKind::FS, -720, DurationUnit::Days),
                link(4, LinkKind::FF, 30, DurationUnit::Minutes),
                link(5, LinkKind::SF, 2400, DurationUnit::Weeks),
                link(6, LinkKind::FS, 9600, DurationUnit::Months),
                link(7, LinkKind::FS, 120, DurationUnit::Hours),
            ]
        );
        assert_eq!(preds_text(&part, &O), "2SS+4h, 3FS-1.5, 4FF+30m, 5SF+1w, 6FS+1mo, 7FS+2h");
        assert_eq!(parse_preds(&preds_text(&part, &O), &O).unwrap(), part);
        // Days show at the hours per day in force: 960 minutes is 2 days at 8 hours, 2.13 at 7.5.
        let odd = vec![link(2, LinkKind::SS, 960, DurationUnit::Days)];
        assert_eq!(preds_text(&odd, &Options { minutes_per_day: 450, ..O }), "2SS+2.13");
        assert_eq!(preds_text(&odd, &O), "2SS+2");
    }

    #[test]
    fn row_ids_become_unique_ids_with_lags_as_typed() {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let a = p.add_task("a", D).unwrap();
        for n in ["b", "c"] {
            p.add_task(n, D).unwrap();
        }
        p.delete_tasks(&[a]).unwrap();
        // Rows 1 and 2 are Unique IDs 2 and 3.
        assert_eq!(p.preds_rows_to_ids(" 1SS+1.33d; 2fs-4h ").unwrap(), "2SS+1.33d, 3fs-4h");
        assert_eq!(p.preds_rows_to_ids("").unwrap(), "");
        assert!(p.preds_rows_to_ids("3").is_err());
        assert!(p.preds_rows_to_ids("1FS+2x").is_err());
    }

    #[test]
    fn days_text_shows_up_to_two_decimals() {
        for (m, day, text) in
            [(0, D, "0"), (3 * D, D, "3"), (720, D, "1.5"), (-960, D, "-2"), (1, D, "0"), (-1, D, "0"), (960, 450, "2.13"), (5, 1, "5")]
        {
            assert_eq!(days_text(m, day), text, "{m} at {day}");
        }
        assert_eq!(days_text(i64::MIN, 0), "-9223372036854775808");
    }

    #[test]
    fn set_preds_refuses_loops_and_keeps_the_old_links() {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let a = p.add_task("a", D).unwrap();
        let b = p.add_task("b", D).unwrap();
        p.set_preds(b, parse_preds("1", &O).unwrap()).unwrap();
        assert!(matches!(p.set_preds(a, parse_preds("2", &O).unwrap()), Err(ModelError::Cycle(_))));
        assert!(p.task(a).unwrap().preds.is_empty());
        assert!(p.set_preds(a, parse_preds("1", &O).unwrap()).is_err());
        assert!(p.set_preds(a, parse_preds("42", &O).unwrap()).is_err());
    }

    #[test]
    fn predecessors_column_uses_row_ids_and_links_keep_unique_ids() {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let a = p.add_task("a", D).unwrap();
        let b = p.add_task("b", D).unwrap();
        let c = p.add_task("c", D).unwrap();
        // Rows 1, 2 after a is deleted: b is row 1, c is row 2.
        p.delete_tasks(&[a]).unwrap();
        assert_eq!((p.row_of(b), p.row_of(c), p.row_of(a)), (Some(1), Some(2), None));
        let links = p.parse_preds_rows("1SS+2").unwrap();
        assert_eq!(links, vec![Link { pred: b, kind: LinkKind::SS, lag: 2 * D, lag_unit: DurationUnit::Days }]);
        p.set_preds(c, links).unwrap();
        assert_eq!(p.preds_text_rows(&p.task(c).unwrap().preds), "1SS+2");
        assert_eq!(preds_text(&p.task(c).unwrap().preds, &O), "2SS+2", "stored by Unique ID");
        // Moving rows renumbers the column; the link still points at b.
        p.tasks.swap(0, 1);
        assert_eq!(p.preds_text_rows(&p.task(c).unwrap().preds), "2SS+2");
        assert_eq!(p.task(c).unwrap().preds[0].pred, b);
        assert!(p.parse_preds_rows("3").is_err());
        assert!(p.parse_preds_rows("0").is_err());
        assert_eq!(p.preds_text_rows(&[link(99, LinkKind::FF, -D, DurationUnit::Days)]), "?FF-1");
        // A deleted Unique ID is never handed out again.
        assert_eq!(p.add_task("d", D).unwrap(), TaskId(4));
    }

    #[test]
    fn minutes_between_counts_working_time_and_honours_exceptions() {
        let mut c = Calendar::default();
        let d = |s: &str| s.parse::<Date>().unwrap();
        // Mon 5 .. Sun 18 January 2026: ten working days of 8 hours.
        assert_eq!(c.minutes_between(d("2026-01-05"), d("2026-01-19")), 10 * D);
        assert_eq!(c.minutes_between(d("2026-01-10"), d("2026-01-12")), 0);
        assert_eq!(c.minutes_between(d("2026-01-12"), d("2026-01-05")), 0);
        c.add_exception(d("2026-01-07"), false).unwrap();
        c.add_exception(d("2026-01-10"), false).unwrap(); // a Saturday: not worked anyway
        assert_eq!(c.minutes_between(d("2026-01-05"), d("2026-01-19")), 9 * D);
        c.add_exception(d("2026-01-17"), true).unwrap(); // an overtime Saturday
        c.add_exception(d("2026-01-14"), true).unwrap(); // a Wednesday: worked anyway
        assert_eq!(c.minutes_between(d("2026-01-05"), d("2026-01-19")), 10 * D);
        // Working times count: a 4-hour Friday, a Saturday worked 9:00–13:00.
        c.set_weekday_times(4, vec![parse_working_time("8:00-12:00").unwrap()]).unwrap();
        c.set_exception(d("2026-01-17"), vec![parse_working_time("9:00-13:00").unwrap()]).unwrap();
        assert_eq!(c.minutes_between(d("2026-01-05"), d("2026-01-19")), 7 * D + 3 * 240);
        // Agrees with a day-by-day sum, and the walk's table with both.
        let from = d("2025-12-29");
        let table = c.working_days(from, 40 * D, 1_000).unwrap();
        let mut sum = 0;
        for k in 0..60 {
            let day = from.add_days(k).unwrap();
            assert_eq!(c.minutes_between(from, day), sum, "{day}");
            if let Some((_, before)) = table.iter().find(|(t, _)| *t == day) {
                assert_eq!(*before, sum);
            }
            assert_eq!(table.iter().any(|(t, _)| *t == day), c.minutes_on(day) > 0 && sum <= 40 * D, "{day}");
            sum += c.minutes_on(day);
        }
        assert_eq!(c.working_days(from, 40 * D, 3), None, "more working days than allowed");
        assert!(c.minutes_between(d("0001-01-01"), d("9999-12-31")) > 2_000_000 * D / 2);
    }

    #[test]
    fn a_week_with_no_working_day_is_refused_and_exceptions_replace_each_other() {
        let mut c = Calendar::default();
        let d = |s: &str| s.parse::<Date>().unwrap();
        assert!(c.set_working([false; 7]).is_err());
        assert_eq!(c, Calendar::default());
        c.set_working([false, false, false, false, false, true, true]).unwrap();
        assert!(c.is_workday(d("2026-01-10")) && !c.is_workday(d("2026-01-12")));
        c.add_exception(d("2026-01-12"), true).unwrap();
        c.add_exception(d("2026-01-12"), false).unwrap();
        assert_eq!(c.exceptions.len(), 1);
        assert!(!c.is_workday(d("2026-01-12")));
        c.remove_exception(d("2026-01-12")).unwrap();
        assert!(c.remove_exception(d("2026-01-12")).is_err());
        // A damaged file's week with no working day: only working exceptions are worked, and
        // asking for more than there are is None, not a walk to year 9999.
        c.working = [false; 7];
        c.add_exception(d("2026-01-14"), true).unwrap();
        assert_eq!(c.working_days(d("2026-01-01"), D - 1, 10), Some(vec![(d("2026-01-14"), 0)]));
        assert_eq!(c.working_days(d("2026-01-01"), D, 10), None);
        assert_eq!(c.minutes_between(d("2026-01-01"), d("2026-02-01")), D);
    }

    #[test]
    fn working_times_per_weekday_and_exception_agree_with_the_working_days() {
        let mut c = Calendar::default();
        let d = |s: &str| s.parse::<Date>().unwrap();
        let t = |s: &str| parse_working_time(s).unwrap();
        // Default: Mon–Fri 8:00–12:00 and 13:00–17:00, eight hours; the weekend has none.
        assert_eq!(c.weekday_times(0), &DEFAULT_WORKING_TIMES);
        assert_eq!(minutes_of(c.weekday_times(4)), 480);
        assert!(c.weekday_times(5).is_empty() && c.weekday_times(6).is_empty() && c.weekday_times(7).is_empty());
        // A short Friday; a Saturday given times becomes a working day; a day with none is off.
        c.set_weekday_times(4, vec![t("8:00-12:00")]).unwrap();
        c.set_weekday_times(5, vec![t("9:00-13:00")]).unwrap();
        c.set_weekday_times(2, vec![]).unwrap();
        assert_eq!(c.working, [true, true, false, true, true, true, false]);
        assert_eq!(c.times_on(d("2026-01-09")), &[t("8:00-12:00")]);
        assert!(c.times_on(d("2026-01-07")).is_empty() && !c.is_workday(d("2026-01-07")));
        // Refused: overlapping, out of order, and a week left with no working day.
        let before = c.clone();
        assert!(c.set_weekday_times(0, vec![t("8:00-12:00"), t("11:00-17:00")]).is_err());
        assert!(c.set_weekday_times(0, vec![t("13:00-17:00"), t("8:00-12:00")]).is_err());
        let mut one = Calendar::default();
        one.set_working([true, false, false, false, false, false, false]).unwrap();
        assert!(one.set_weekday_times(0, vec![]).is_err());
        assert_eq!(c, before);
        // Unticking a weekday drops its times; ticking it back gives the default.
        c.set_working([true, true, true, true, false, true, false]).unwrap();
        c.set_working([true, true, true, true, true, true, false]).unwrap();
        assert_eq!(c.weekday_times(4), &DEFAULT_WORKING_TIMES);
        assert_eq!(c.weekday_times(5), &[t("9:00-13:00")], "unchanged days keep their times");
        // A worked Saturday with its own hours; a holiday has none; removing goes back to the weekday.
        c.set_exception(d("2026-01-17"), vec![t("9:00-13:00")]).unwrap();
        c.set_exception(d("2026-01-12"), vec![]).unwrap();
        c.add_exception(d("2026-01-18"), true).unwrap();
        assert_eq!(c.times_on(d("2026-01-17")), &[t("9:00-13:00")]);
        assert_eq!(c.times_on(d("2026-01-18")), &DEFAULT_WORKING_TIMES);
        assert_eq!(c.exceptions.get(&d("2026-01-12")), Some(&false));
        assert!(c.times_on(d("2026-01-12")).is_empty());
        assert!(c.set_exception(d("2026-01-19"), vec![t("9:00-13:00"), t("12:00-14:00")]).is_err());
        c.remove_exception(d("2026-01-17")).unwrap();
        c.add_exception(d("2026-01-17"), true).unwrap();
        assert_eq!(c.times_on(d("2026-01-17")), &DEFAULT_WORKING_TIMES, "the old times went with the old exception");
    }

    #[test]
    fn constraints_from_codes() {
        let d = Date::from_ymd(2026, 1, 12);
        assert_eq!(Constraint::from_code("asap", None).unwrap(), Constraint::Asap);
        assert_eq!(Constraint::from_code("ASAP", d).unwrap(), Constraint::Asap);
        for code in Constraint::CODES {
            let c = Constraint::from_code(code, d).unwrap();
            assert_eq!(c.code(), code);
            assert_eq!(c.date(), if code == "ASAP" { None } else { d });
        }
        assert!(Constraint::from_code("SNET", None).unwrap_err().to_string().contains("Start No Earlier Than needs a date"));
        assert!(Constraint::from_code("ALAP", d).is_err());
    }

    proptest::proptest! {
        #[test]
        fn parse_preds_never_panics(s in "\\PC{0,40}") {
            let _ = parse_preds(&s, &O);
            let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
            p.add_task("a", D).unwrap();
            if let Ok(links) = p.parse_preds_rows(&s) {
                let _ = p.preds_text_rows(&links);
            }
        }
    }
}
