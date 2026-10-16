//! ProjectCraft project model: a project's tasks, the links between them, its working calendar, and
//! the critical-path scheduler ([`schedule`]).
//!
//! Everything here is plain data with edit primitives; undo and commands live in the engine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod duration;
mod outline;
mod resource;
mod rows;
mod schedule;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub use duration::{DAYS_PER_MONTH, DAYS_PER_WEEK, parse_duration};
pub use outline::{CIRCULAR_OUTLINE, MAX_OUTLINE_LEVEL, Outline};
pub use projectcraft_time::Date;
pub use resource::{
    AccrueAt, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS, FULL_TIME, MAX_MONEY, MAX_RESOURCE_TEXT, MAX_RESOURCES, MAX_UNITS, Resource,
    ResourceField, ResourceId, ResourceType, UNITS_PER_PERCENT, check_name, format_money, parse_money, parse_units, units_text,
};
pub use schedule::{Driver, Schedule, TaskDates, schedule};

/// Longest duration or lag accepted, in working days (about 400 years): keeps all arithmetic in range.
pub const MAX_DAYS: i64 = 100_000;
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
    /// Working days; negative is lead.
    #[serde(default)]
    pub lag: i64,
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
    /// Working days; 0 is a milestone.
    pub duration: i64,
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

/// Which days are worked: working weekdays, and **exceptions** (whole days made non-working, a
/// holiday, or working, an overtime Saturday), as Microsoft Project's Change Working Time.
/// An exception wins over its weekday.
///
/// Written to the file as `{"working": [Mon…Sun], "exceptions": [{"date": "2026-12-25",
/// "working": false}]}`. Files from before exceptions held `"holidays": ["2026-12-25"]`; those
/// load as non-working exceptions and are written back as exceptions.
///
/// ponytail: one project calendar of whole days; work weeks, hours, resource and task calendars
/// come later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "CalendarFile", from = "CalendarFile")]
pub struct Calendar {
    /// Monday first.
    pub working: [bool; 7],
    /// Exception days: `true` worked, `false` not worked, whatever the weekday says.
    pub exceptions: BTreeMap<Date, bool>,
}

/// The calendar as the file holds it.
#[derive(Serialize, Deserialize)]
struct CalendarFile {
    working: [bool; 7],
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
}

impl From<CalendarFile> for Calendar {
    fn from(f: CalendarFile) -> Calendar {
        let mut exceptions: BTreeMap<Date, bool> = f.holidays.into_iter().map(|d| (d, false)).collect();
        exceptions.extend(f.exceptions.into_iter().map(|e| (e.date, e.working)));
        Calendar { working: f.working, exceptions }
    }
}

impl From<Calendar> for CalendarFile {
    fn from(c: Calendar) -> CalendarFile {
        let exceptions = c.exceptions.into_iter().map(|(date, working)| ExceptionFile { date, working }).collect();
        CalendarFile { working: c.working, exceptions, holidays: BTreeSet::new() }
    }
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar { working: [true, true, true, true, true, false, false], exceptions: BTreeMap::new() }
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

    /// Sets the working weekdays (Monday first). Refuses a week with no working day, even when
    /// working exceptions exist: such a calendar would run out of days after its last exception.
    pub fn set_working(&mut self, working: [bool; 7]) -> Result<(), ModelError> {
        if !working.contains(&true) {
            return Err(ModelError::Invalid("a calendar needs at least one working weekday".into()));
        }
        self.working = working;
        Ok(())
    }

    /// Makes `date` a working (`true`) or non-working day, replacing any exception already on it.
    pub fn add_exception(&mut self, date: Date, working: bool) -> Result<(), ModelError> {
        if self.exceptions.len() >= MAX_EXCEPTIONS && !self.exceptions.contains_key(&date) {
            return Err(ModelError::Invalid(format!("a calendar holds at most {MAX_EXCEPTIONS} exceptions")));
        }
        self.exceptions.insert(date, working);
        Ok(())
    }

    /// Removes the exception on `date`; the day goes back to its weekday.
    pub fn remove_exception(&mut self, date: Date) -> Result<(), ModelError> {
        self.exceptions.remove(&date).map(|_| ()).ok_or_else(|| ModelError::Invalid(format!("there is no exception on {date}")))
    }

    /// Working days on or after `from` and before `to`; 0 when `to` is not after `from`. Whole
    /// weeks are counted arithmetically, then each exception in the range corrects its day, so
    /// any range in years 1–9999 is quick.
    pub fn workdays_between(&self, from: Date, to: Date) -> i64 {
        let n = to.days_since(from);
        if n <= 0 {
            return 0;
        }
        let per_week = self.working.iter().filter(|w| **w).count() as i64;
        let (weeks, rest) = (n / 7, n % 7);
        let tail = (0..rest).filter(|k| from.add_days(weeks * 7 + k).is_some_and(|d| self.works_weekday(d))).count() as i64;
        let correction: i64 = self.exceptions.range(from..to).map(|(d, w)| i64::from(*w) - i64::from(self.works_weekday(*d))).sum();
        weeks * per_week + tail + correction
    }

    /// The first `count` working days on or after `from`; `None` when the calendar runs out of
    /// working days, or past year 9999.
    pub fn workdays(&self, from: Date, count: usize) -> Option<Vec<Date>> {
        if !self.working.contains(&true) {
            // Only the working exceptions are worked (a damaged file; commands refuse this week).
            let out: Vec<Date> = self.exceptions.range(from..).filter(|(_, w)| **w).map(|(d, _)| *d).take(count).collect();
            return (out.len() == count).then_some(out);
        }
        let (mut d, mut out) = (from, Vec::with_capacity(count));
        // Terminates: at least one weekday is worked, exceptions are finite, and add_days stops at year 9999.
        while out.len() < count {
            if self.is_workday(d) {
                out.push(d);
            }
            d = d.add_days(1)?;
        }
        Some(out)
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
}

fn is_first_id(n: &u32) -> bool {
    *n <= 1
}

impl Project {
    pub fn new(name: impl Into<String>, start: Date) -> Self {
        Project { name: name.into(), start, calendar: Calendar::default(), tasks: Vec::new(), next_id: 1, resources: Vec::new(), next_resource_id: 1 }
    }

    pub fn task(&self, id: TaskId) -> Result<&Task, ModelError> {
        self.tasks.iter().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    pub fn task_mut(&mut self, id: TaskId) -> Result<&mut Task, ModelError> {
        self.tasks.iter_mut().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    /// Adds a task at the end, at the top level, and returns its Unique ID.
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

    /// Predecessors as the Predecessors column shows them: by row ID. A link to a missing task
    /// (only a damaged file could hold one) shows as `?`.
    pub fn preds_text_rows(&self, preds: &[Link]) -> String {
        let one = |l: &Link| match self.row_of(l.pred) {
            Some(row) => link_text(row, l),
            None => link_text('?', l),
        };
        preds.iter().map(one).collect::<Vec<_>>().join(", ")
    }

    /// Reads the Predecessors column: row IDs, returned as links by Unique ID.
    pub fn parse_preds_rows(&self, text: &str) -> Result<Vec<Link>, ModelError> {
        parse_preds(text)?
            .into_iter()
            .map(|l| {
                let pred = self.task_at_row(l.pred.0).ok_or_else(|| ModelError::Invalid(format!("no task with ID {}", l.pred)))?;
                Ok(Link { pred, ..l })
            })
            .collect()
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
            check_days(l.lag.abs())?;
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

pub fn check_days(n: i64) -> Result<i64, ModelError> {
    if (0..=MAX_DAYS).contains(&n) { Ok(n) } else { Err(ModelError::Invalid(format!("{n} is not between 0 and {MAX_DAYS} days"))) }
}

/// Predecessors by Unique ID, as commands take them: `3, 5SS+2, 7FF-1`.
pub fn preds_text(preds: &[Link]) -> String {
    preds.iter().map(|l| link_text(l.pred, l)).collect::<Vec<_>>().join(", ")
}

/// One link, its predecessor written as `pred`.
fn link_text(pred: impl fmt::Display, l: &Link) -> String {
    let kind = match l.kind {
        LinkKind::FS if l.lag == 0 => String::new(),
        k => format!("{k:?}"),
    };
    let lag = if l.lag == 0 { String::new() } else { format!("{:+}", l.lag) };
    format!("{pred}{kind}{lag}")
}

/// Reads what [`preds_text`] writes (an optional trailing `d` on a lag is accepted).
pub fn parse_preds(text: &str) -> Result<Vec<Link>, ModelError> {
    let bad = |p: &str| ModelError::Invalid(format!("not a predecessor: {}", p.chars().take(40).collect::<String>()));
    let mut out = Vec::new();
    for part in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let digits = part.find(|c: char| !c.is_ascii_digit()).unwrap_or(part.len());
        let (id, rest) = part.split_at(digits);
        let pred = TaskId(id.parse().map_err(|_| bad(part))?);
        let upper = rest.to_ascii_uppercase();
        let (kind, lag) = match upper.get(..2) {
            Some("FS") => (LinkKind::FS, &rest[2..]),
            Some("SS") => (LinkKind::SS, &rest[2..]),
            Some("FF") => (LinkKind::FF, &rest[2..]),
            Some("SF") => (LinkKind::SF, &rest[2..]),
            _ => (LinkKind::FS, rest),
        };
        let lag = lag.trim().trim_end_matches(['d', 'D']).trim();
        let lag = if lag.is_empty() { 0 } else { lag.trim_start_matches('+').parse::<i64>().map_err(|_| bad(part))? };
        check_days(lag.saturating_abs())?;
        out.push(Link { pred, kind, lag });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predecessor_text_round_trips() {
        let links = parse_preds("3, 5ss+2, 7FF-1d;9").unwrap();
        assert_eq!(preds_text(&links), "3, 5SS+2, 7FF-1, 9");
        assert!(parse_preds("x").is_err());
        assert!(parse_preds("1FS+999999999999999999999").is_err());
        assert_eq!(parse_preds("").unwrap(), vec![]);
    }

    #[test]
    fn set_preds_refuses_loops_and_keeps_the_old_links() {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let a = p.add_task("a", 1).unwrap();
        let b = p.add_task("b", 1).unwrap();
        p.set_preds(b, parse_preds("1").unwrap()).unwrap();
        assert!(matches!(p.set_preds(a, parse_preds("2").unwrap()), Err(ModelError::Cycle(_))));
        assert!(p.task(a).unwrap().preds.is_empty());
        assert!(p.set_preds(a, parse_preds("1").unwrap()).is_err());
        assert!(p.set_preds(a, parse_preds("42").unwrap()).is_err());
    }

    #[test]
    fn predecessors_column_uses_row_ids_and_links_keep_unique_ids() {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let a = p.add_task("a", 1).unwrap();
        let b = p.add_task("b", 1).unwrap();
        let c = p.add_task("c", 1).unwrap();
        // Rows 1, 2 after a is deleted: b is row 1, c is row 2.
        p.delete_tasks(&[a]).unwrap();
        assert_eq!((p.row_of(b), p.row_of(c), p.row_of(a)), (Some(1), Some(2), None));
        let links = p.parse_preds_rows("1SS+2").unwrap();
        assert_eq!(links, vec![Link { pred: b, kind: LinkKind::SS, lag: 2 }]);
        p.set_preds(c, links).unwrap();
        assert_eq!(p.preds_text_rows(&p.task(c).unwrap().preds), "1SS+2");
        assert_eq!(preds_text(&p.task(c).unwrap().preds), "2SS+2", "stored by Unique ID");
        // Moving rows renumbers the column; the link still points at b.
        p.tasks.swap(0, 1);
        assert_eq!(p.preds_text_rows(&p.task(c).unwrap().preds), "2SS+2");
        assert_eq!(p.task(c).unwrap().preds[0].pred, b);
        assert!(p.parse_preds_rows("3").is_err());
        assert!(p.parse_preds_rows("0").is_err());
        assert_eq!(p.preds_text_rows(&[Link { pred: TaskId(99), kind: LinkKind::FF, lag: -1 }]), "?FF-1");
        // A deleted Unique ID is never handed out again.
        assert_eq!(p.add_task("d", 1).unwrap(), TaskId(4));
    }

    #[test]
    fn workdays_between_counts_working_days_and_honours_exceptions() {
        let mut c = Calendar::default();
        let d = |s: &str| s.parse::<Date>().unwrap();
        // Mon 5 .. Sun 18 January 2026: ten working days.
        assert_eq!(c.workdays_between(d("2026-01-05"), d("2026-01-19")), 10);
        assert_eq!(c.workdays_between(d("2026-01-10"), d("2026-01-12")), 0);
        assert_eq!(c.workdays_between(d("2026-01-12"), d("2026-01-05")), 0);
        c.add_exception(d("2026-01-07"), false).unwrap();
        c.add_exception(d("2026-01-10"), false).unwrap(); // a Saturday: not worked anyway
        assert_eq!(c.workdays_between(d("2026-01-05"), d("2026-01-19")), 9);
        c.add_exception(d("2026-01-17"), true).unwrap(); // an overtime Saturday
        c.add_exception(d("2026-01-14"), true).unwrap(); // a Wednesday: worked anyway
        assert_eq!(c.workdays_between(d("2026-01-05"), d("2026-01-19")), 10);
        // Agrees with a day-by-day walk.
        let from = d("2025-12-29");
        let walk = c.workdays(from, 40).unwrap();
        for (i, day) in walk.iter().enumerate() {
            assert_eq!(c.workdays_between(from, *day), i as i64);
        }
        assert!(c.workdays_between(d("0001-01-01"), d("9999-12-31")) > 2_000_000);
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
        assert_eq!(c.workdays(d("2026-01-01"), 1), Some(vec![d("2026-01-14")]));
        assert_eq!(c.workdays(d("2026-01-01"), 2), None);
        assert_eq!(c.workdays_between(d("2026-01-01"), d("2026-02-01")), 1);
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
            let _ = parse_preds(&s);
            let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
            p.add_task("a", 1).unwrap();
            if let Ok(links) = p.parse_preds_rows(&s) {
                let _ = p.preds_text_rows(&links);
            }
        }
    }
}
