//! ProjectCraft project model: a project's tasks, the links between them, its working calendar, and
//! the critical-path scheduler ([`schedule`]).
//!
//! Everything here is plain data with edit primitives; undo and commands live in the engine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod schedule;

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

pub use projectcraft_time::Date;
pub use schedule::{Schedule, TaskDates, schedule};

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

/// A task's identity: stable for the life of the project, shown in the ID column, never reused.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub name: String,
    /// Working days; 0 is a milestone.
    pub duration: i64,
    #[serde(default)]
    pub preds: Vec<Link>,
}

/// Which days are worked. ponytail: one project calendar; resource and task calendars come with resources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Monday first.
    pub working: [bool; 7],
    #[serde(default)]
    pub holidays: BTreeSet<Date>,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar { working: [true, true, true, true, true, false, false], holidays: BTreeSet::new() }
    }
}

impl Calendar {
    pub fn is_workday(&self, d: Date) -> bool {
        self.working.get(usize::from(d.weekday())).copied().unwrap_or(false) && !self.holidays.contains(&d)
    }

    /// The first `count` working days on or after `from`; `None` with no working weekday, or past year 9999.
    pub fn workdays(&self, from: Date, count: usize) -> Option<Vec<Date>> {
        if !self.working.contains(&true) {
            return None;
        }
        let (mut d, mut out) = (from, Vec::with_capacity(count));
        // Terminates: at least one weekday is worked, holidays are finite, and add_days stops at year 9999.
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
}

impl Project {
    pub fn new(name: impl Into<String>, start: Date) -> Self {
        Project { name: name.into(), start, calendar: Calendar::default(), tasks: Vec::new(), next_id: 1 }
    }

    pub fn task(&self, id: TaskId) -> Result<&Task, ModelError> {
        self.tasks.iter().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    pub fn task_mut(&mut self, id: TaskId) -> Result<&mut Task, ModelError> {
        self.tasks.iter_mut().find(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    /// Adds a task at the end and returns its id.
    pub fn add_task(&mut self, name: impl Into<String>, duration: i64) -> Result<TaskId, ModelError> {
        if self.tasks.len() >= MAX_TASKS {
            return Err(ModelError::Invalid(format!("a project holds at most {MAX_TASKS} tasks")));
        }
        let id = TaskId(self.next_id.max(1));
        self.next_id = id.0.checked_add(1).ok_or_else(|| ModelError::Invalid("out of task ids".into()))?;
        self.tasks.push(Task { id, name: name.into(), duration: check_days(duration)?, preds: Vec::new() });
        Ok(id)
    }

    /// Removes a task and every link to it.
    pub fn delete_task(&mut self, id: TaskId) -> Result<(), ModelError> {
        self.task(id)?;
        self.tasks.retain(|t| t.id != id);
        for t in &mut self.tasks {
            t.preds.retain(|l| l.pred != id);
        }
        Ok(())
    }

    /// Replaces a task's predecessors, refusing links to missing tasks, to itself, or that close a loop.
    pub fn set_preds(&mut self, id: TaskId, preds: Vec<Link>) -> Result<(), ModelError> {
        for l in &preds {
            self.task(l.pred)?;
            if l.pred == id {
                return Err(ModelError::Cycle(id));
            }
            check_days(l.lag.abs())?;
        }
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

/// Predecessors as the Predecessors column writes them: `3, 5SS+2, 7FF-1`.
pub fn preds_text(preds: &[Link]) -> String {
    let one = |l: &Link| {
        let kind = match l.kind {
            LinkKind::FS if l.lag == 0 => String::new(),
            k => format!("{k:?}"),
        };
        let lag = if l.lag == 0 { String::new() } else { format!("{:+}", l.lag) };
        format!("{}{kind}{lag}", l.pred)
    };
    preds.iter().map(one).collect::<Vec<_>>().join(", ")
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

    proptest::proptest! {
        #[test]
        fn parse_preds_never_panics(s in "\\PC{0,40}") {
            let _ = parse_preds(&s);
        }
    }
}
