//! Critical-path scheduling: a forward pass for the early dates, a backward pass for the late
//! dates, slack as the difference. Times are working-day offsets from the project start, turned
//! into dates through the calendar at the end.
//!
//! Constraints: Start No Earlier Than raises the early start to its date, Finish No Earlier Than
//! the early finish (ES = max(ES, CD), EF = max(EF, CD)). A constraint date on a non-working day
//! counts as the next working day for a start type and the previous one for a finish type.
//!
//! ponytail: schedules forward from the project start, with no hard constraints (#12), no actual
//! progress and no resource levelling; each is its own step on the roadmap.

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
    for &i in &order {
        let Some(t) = p.tasks.get(i) else { continue };
        let mut start = 0i64;
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
            start = start.max(s);
        }
        match t.constraint {
            Constraint::StartNoEarlierThan(cd) => start = start.max(start_offset(p, cd)),
            Constraint::FinishNoEarlierThan(cd) => start = start.max(finish_offset(p, cd) - d(i)),
            // Hard types are honoured from #12; ASAP has nothing to add.
            _ => {}
        }
        if let (Some(a), Some(b)) = (es.get_mut(i), ef.get_mut(i)) {
            *a = start;
            *b = start + d(i);
        }
    }
    let end = ef.iter().copied().max().unwrap_or(0);

    // Backward pass.
    let mut lf = vec![end; n];
    for &i in order.iter().rev() {
        let Some(t) = p.tasks.get(i) else { continue };
        let my_lf = lf.get(i).copied().unwrap_or(end);
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
        let slack = ls - s;
        let first = |o: i64| if d(i) == 0 { day(o - 1) } else { day(o) };
        tasks.insert(
            t.id,
            TaskDates { start: first(s)?, finish: last(s, f)?, late_start: first(ls)?, late_finish: last(ls, lfi)?, slack, critical: slack <= 0 },
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
