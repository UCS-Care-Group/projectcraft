//! Critical-path scheduling: a forward pass for the early dates, a backward pass for the late
//! dates, slack as the difference. Times are working-day offsets from the project start, turned
//! into dates through the calendar at the end.
//!
//! Constraints: Start No Earlier Than raises the early start to its date, Finish No Earlier Than
//! the early finish (ES = max(ES, CD), EF = max(EF, CD)). A constraint date on a non-working day
//! counts as the next working day for a start type and the previous one for a finish type.
//!
//! Hard constraints win over links, Project's default "Tasks will always honor their constraint
//! dates". Per Microsoft's definitions
//! (https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints):
//! Start No Later Than sets LS = CD if CD < LS, Finish No Later Than LF = CD if CD < LF; Must Start
//! On sets ES, LS and the scheduled start to CD, Must Finish On EF, LF and the scheduled finish.
//! A task whose links would take it past an SNLT or FNLT date is held at the date; its early start
//! stays where the links put it, so its slack (LS − ES) goes negative. The late dates carry the cap
//! back to its predecessors, so their slack goes negative too. Either way the task is flagged
//! ([`TaskDates::constraint_conflict`]) on every run.
//!
//! ponytail: schedules forward from the project start, with no actual progress and no resource
//! levelling; each is its own step on the roadmap. A Must Start On before the project start is
//! held at the project start.

use std::collections::HashMap;

use crate::{Constraint, Date, LinkKind, ModelError, Project, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskDates {
    pub start: Date,
    /// The last working day of the task; the start for a milestone.
    pub finish: Date,
    pub late_start: Date,
    pub late_finish: Date,
    /// Total slack in working days.
    pub slack: i64,
    pub critical: bool,
    /// A link can't be honoured because a hard constraint holds the task earlier: show a warning.
    pub constraint_conflict: bool,
    /// What set the start.
    pub driver: Driver,
}

/// What drove a task's start: the scheduler's answer to "why this date?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// Nothing later: it starts at the project start.
    ProjectStart,
    /// The link from this predecessor (by Unique ID); the first in the list when several tie.
    Link(TaskId),
    /// Its constraint: a Start/Finish No Earlier Than date later than the links, a No Later Than
    /// date holding it earlier, or a Must Start/Finish On date.
    Constraint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub tasks: HashMap<TaskId, TaskDates>,
    pub finish: Date,
}

/// Task indices in an order where every predecessor comes before its successors.
pub(crate) fn order(p: &Project) -> Result<Vec<usize>, ModelError> {
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut waiting: Vec<usize> = p.tasks.iter().map(|t| t.preds.iter().filter(|l| index.contains_key(&l.pred)).count()).collect();
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); p.tasks.len()];
    for (i, t) in p.tasks.iter().enumerate() {
        for l in &t.preds {
            if let Some(&j) = index.get(&l.pred)
                && let Some(s) = succs.get_mut(j)
            {
                s.push(i);
            }
        }
    }
    let mut ready: Vec<usize> = (0..p.tasks.len()).filter(|&i| waiting.get(i) == Some(&0)).rev().collect();
    let mut out = Vec::with_capacity(p.tasks.len());
    while let Some(i) = ready.pop() {
        out.push(i);
        for &s in succs.get(i).into_iter().flatten() {
            if let Some(w) = waiting.get_mut(s) {
                *w -= 1;
                if *w == 0 {
                    ready.push(s);
                }
            }
        }
    }
    if out.len() == p.tasks.len() {
        return Ok(out);
    }
    let stuck = (0..p.tasks.len()).find(|i| waiting.get(*i).is_some_and(|w| *w > 0)).and_then(|i| p.tasks.get(i)).map_or(TaskId(0), |t| t.id);
    Err(ModelError::Cycle(stuck))
}

/// The offset of the first working day on or after `cd`: where a task constrained to start
/// there begins. Before the project start, 0.
fn start_offset(p: &Project, cd: Date) -> i64 {
    p.calendar.workdays_between(p.start, cd)
}

/// The offset just after the last working day on or before `cd`: where a task constrained to
/// finish there ends. Before the project start, 0.
fn finish_offset(p: &Project, cd: Date) -> i64 {
    if cd < p.start {
        return 0;
    }
    p.calendar.workdays_between(p.start, cd) + i64::from(p.calendar.is_workday(cd))
}

pub fn schedule(p: &Project) -> Result<Schedule, ModelError> {
    let order = order(p)?;
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let dur: Vec<i64> = p.tasks.iter().map(|t| t.duration.clamp(0, crate::MAX_DAYS)).collect();
    let d = |i: usize| dur.get(i).copied().unwrap_or(0);

    // Forward pass. Offsets stay small: at most MAX_TASKS chains of MAX_DAYS-bounded steps fit in i64.
    let (mut es, mut ef) = (vec![0i64; n], vec![0i64; n]);
    // Early start for slack: where the links put the task before a hard constraint held it.
    let mut early = vec![0i64; n];
    let mut conflict = vec![false; n];
    let mut driver = vec![Driver::ProjectStart; n];
    for &i in &order {
        let Some(t) = p.tasks.get(i) else { continue };
        let mut start = 0i64;
        let mut why = Driver::ProjectStart;
        for l in &t.preds {
            let Some(&j) = index.get(&l.pred) else { continue };
            let (pes, pef) = (es.get(j).copied().unwrap_or(0), ef.get(j).copied().unwrap_or(0));
            let lag = l.lag.clamp(-crate::MAX_DAYS, crate::MAX_DAYS);
            let s = match l.kind {
                LinkKind::FS => pef + lag,
                LinkKind::SS => pes + lag,
                LinkKind::FF => pef + lag - d(i),
                LinkKind::SF => pes + lag - d(i),
            };
            if s > start || (s == start && why == Driver::ProjectStart) {
                (start, why) = (s, Driver::Link(l.pred));
            }
        }
        // Where the links (and a No Earlier Than date) put it; then what a hard constraint makes of it.
        let natural = match t.constraint {
            Constraint::StartNoEarlierThan(cd) => start.max(start_offset(p, cd)),
            Constraint::FinishNoEarlierThan(cd) => start.max(finish_offset(p, cd) - d(i)),
            _ => start,
        };
        let held = match t.constraint {
            Constraint::StartNoLaterThan(cd) => natural.min(start_offset(p, cd)),
            Constraint::FinishNoLaterThan(cd) => natural.min(finish_offset(p, cd) - d(i)),
            Constraint::MustStartOn(cd) => start_offset(p, cd),
            Constraint::MustFinishOn(cd) => finish_offset(p, cd) - d(i),
            _ => natural,
        }
        .max(0);
        if natural > start || held != natural || matches!(t.constraint, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_)) {
            why = Driver::Constraint;
        }
        // Must Start/Finish On set the early dates too (ES = CD); the No Later Than types leave them.
        let soft_early = if matches!(t.constraint, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_)) { held } else { natural };
        if let (Some(a), Some(b), Some(e), Some(c), Some(w)) =
            (es.get_mut(i), ef.get_mut(i), early.get_mut(i), conflict.get_mut(i), driver.get_mut(i))
        {
            *a = held;
            *b = held + d(i);
            *e = soft_early;
            *c = start > held;
            *w = why;
        }
    }
    let end = ef.iter().copied().max().unwrap_or(0);

    // Backward pass.
    let mut lf = vec![end; n];
    for &i in order.iter().rev() {
        let Some(t) = p.tasks.get(i) else { continue };
        // A hard constraint caps the late finish: LS = CD (SNLT, MSO), LF = CD (FNLT, MFO).
        let cap = match t.constraint {
            Constraint::StartNoLaterThan(cd) => start_offset(p, cd) + d(i),
            Constraint::FinishNoLaterThan(cd) => finish_offset(p, cd),
            Constraint::MustStartOn(_) | Constraint::MustFinishOn(_) => ef.get(i).copied().unwrap_or(end),
            _ => end,
        };
        let my_lf = lf.get(i).copied().unwrap_or(end).min(cap);
        if let Some(x) = lf.get_mut(i) {
            *x = my_lf;
        }
        let my_ls = my_lf - d(i);
        for l in &t.preds {
            let Some(&j) = index.get(&l.pred) else { continue };
            let lag = l.lag.clamp(-crate::MAX_DAYS, crate::MAX_DAYS);
            let bound = match l.kind {
                LinkKind::FS => my_ls - lag,
                LinkKind::SS => my_ls - lag + d(j),
                LinkKind::FF => my_lf - lag,
                LinkKind::SF => my_lf - lag + d(j),
            };
            if let Some(x) = lf.get_mut(j) {
                *x = (*x).min(bound);
            }
        }
    }

    let bad = || ModelError::Invalid("the schedule runs off the calendar (no working days, or past year 9999)".into());
    // Every offset used is in 0..=end; one walk of the calendar serves them all.
    let len = usize::try_from(end.saturating_add(1)).ok().filter(|&l| l <= 4_000_000).ok_or_else(bad)?;
    let days = p.calendar.workdays(p.start, len).ok_or_else(bad)?;
    let day = |off: i64| usize::try_from(off.max(0)).ok().and_then(|o| days.get(o).copied()).ok_or_else(bad);
    // A task occupying [s, f) working days ends on day f-1. A milestone at offset s is the end of
    // day s-1, so it sits on its predecessor's finish date, as in Microsoft Project.
    let last = |s: i64, f: i64| day(if f > s { f - 1 } else { s - 1 });
    let mut tasks = HashMap::with_capacity(n);
    for (i, t) in p.tasks.iter().enumerate() {
        let (s, f, lfi) = (es.get(i).copied().unwrap_or(0), ef.get(i).copied().unwrap_or(0), lf.get(i).copied().unwrap_or(end));
        let ls = lfi - d(i);
        let slack = ls - early.get(i).copied().unwrap_or(s);
        let first = |o: i64| if d(i) == 0 { day(o - 1) } else { day(o) };
        tasks.insert(
            t.id,
            TaskDates {
                start: first(s)?,
                finish: last(s, f)?,
                late_start: first(ls)?,
                late_finish: last(ls, lfi)?,
                slack,
                critical: slack <= 0,
                constraint_conflict: conflict.get(i).copied().unwrap_or(false),
                driver: driver.get(i).copied().unwrap_or(Driver::ProjectStart),
            },
        );
    }
    Ok(Schedule { tasks, finish: last(0, end)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_preds;

    fn date(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[test]
    fn critical_path_over_a_weekend() {
        // Monday 2026-01-05. A(3) -> B(2), A -> C(1), B,C -> D(0 milestone).
        let mut p = Project::new("t", date("2026-01-05"));
        let a = p.add_task("A", 3).unwrap();
        let b = p.add_task("B", 2).unwrap();
        let c = p.add_task("C", 1).unwrap();
        let m = p.add_task("D", 0).unwrap();
        p.set_preds(b, parse_preds("1").unwrap()).unwrap();
        p.set_preds(c, parse_preds("1").unwrap()).unwrap();
        p.set_preds(m, parse_preds("2,3").unwrap()).unwrap();
        let s = schedule(&p).unwrap();
        let t = |id| s.tasks[&id];
        assert_eq!((t(a).start, t(a).finish), (date("2026-01-05"), date("2026-01-07")));
        // B: Thu, Fri.
        assert_eq!((t(b).start, t(b).finish), (date("2026-01-08"), date("2026-01-09")));
        assert!(t(a).critical && t(b).critical && t(m).critical);
        assert_eq!(t(c).slack, 1);
        assert!(!t(c).critical);
        // The milestone sits on its predecessor's finish, Friday; one at the project start, on the start.
        assert_eq!((t(m).start, t(m).finish), (date("2026-01-09"), date("2026-01-09")));
        let first = p.add_task("Kick-off", 0).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&first].start, date("2026-01-05"));
        assert_eq!(s.finish, date("2026-01-09"));
    }

    #[test]
    fn link_kinds_and_lags() {
        let mut p = Project::new("t", date("2026-01-05"));
        let a = p.add_task("A", 4).unwrap();
        let b = p.add_task("B", 2).unwrap();
        p.set_preds(b, parse_preds("1SS+1").unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].start, date("2026-01-06"));
        p.set_preds(b, parse_preds("1FF").unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].finish, schedule(&p).unwrap().tasks[&a].finish);
        p.set_preds(b, parse_preds("1FS-2").unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].start, date("2026-01-07"));
    }

    #[test]
    fn a_calendar_with_no_working_days_is_an_error() {
        let mut p = Project::new("t", date("2026-01-05"));
        p.add_task("A", 1).unwrap();
        p.calendar.working = [false; 7];
        assert!(schedule(&p).is_err());
    }
}
