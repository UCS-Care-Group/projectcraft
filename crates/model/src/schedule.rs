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
//! A deadline is not a constraint: it never moves a task, but caps its late finish
//! (LF = min(LF, deadline)), so slack counts from the deadline when that is earlier, and a finish
//! after it gives negative slack that the late dates carry back to the predecessors, as Microsoft's
//! Deadline field and "Show slack" pages describe. A deadline on a non-working day counts as the
//! previous working day. The task is flagged ([`TaskDates::deadline_missed`]) when it finishes
//! after the deadline. On a summary, the cap reaches every subtask.
//!
//! Summary tasks ([`Outline`]) are rolled up, not scheduled: a summary starts at its earliest
//! subtask's start and finishes at its latest subtask's finish, and its duration is the working
//! days between. Each subtask comes before its summary in [`order`], so a summary's dates are
//! known before its successors are scheduled. The roll-up steps are marked "Roll-up".
//!
//! A summary's own links and constraint (Project allows both:
//! https://learn.microsoft.com/en-us/archive/technet-wiki/29569.ms-project-linking-andor-assigning-resources-to-summary-tasks)
//! give it a **hold**: the earliest any of its subtasks may start, worked out as for a task (links,
//! then the constraint) and never earlier than its own summary's hold. One forward pass does it:
//! the hold is a node of its own in [`order`], after the summary's predecessors and before its
//! subtasks. A subtask held later than its own links is driven by [`Driver::ParentSummary`]. In
//! the backward pass a summary's late start and finish (from its subtasks) bound its predecessors,
//! and a hard constraint on a summary caps the late finish of every subtask. A link between a
//! summary and one of its own subtasks is a loop through the outline, so [`order`] refuses it.
//!
//! ponytail: a finish-type link or constraint on a summary (FF, SF, Finish No Earlier/Later Than,
//! Must Finish On) needs the summary's duration before its subtasks are scheduled; it takes the
//! rolled-up duration from a first pass without those holds, so a hold that changes the duration
//! (a subtask's own link or constraint) can leave the finish a little off the date. Schedules
//! forward from the project start, with no actual progress and no resource levelling; each is its
//! own step on the roadmap. A Must Start On before the project start is held at the project start.

use std::collections::{BTreeMap, HashMap};

use crate::{Constraint, Date, LinkKind, ModelError, Outline, Project, Task, TaskId};

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
    /// It finishes after its deadline: show the missed-deadline indicator.
    pub deadline_missed: bool,
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
    /// The hold of a summary it sits in (by Unique ID): the summary whose own link or constraint
    /// allows no earlier start, the innermost when several tie.
    ParentSummary(TaskId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub tasks: HashMap<TaskId, TaskDates>,
    pub finish: Date,
    /// Each summary task's duration in working days, rolled up from its subtasks.
    pub rolled_up: BTreeMap<TaskId, i64>,
}

/// The outline as the scheduler uses it: each row's direct subtasks, its summary, and where its
/// subtree ends.
struct Tree {
    kids: Vec<Vec<usize>>,
    parent: Vec<Option<usize>>,
    outline: Outline,
}

impl Tree {
    fn of(p: &Project) -> Tree {
        let outline = Outline::of(p);
        let kids = outline.children();
        let mut parent = vec![None; kids.len()];
        for (s, ks) in kids.iter().enumerate() {
            for &k in ks {
                if let Some(x) = parent.get_mut(k) {
                    *x = Some(s);
                }
            }
        }
        Tree { kids, parent, outline }
    }

    fn summary(&self, i: usize) -> Option<&[usize]> {
        self.kids.get(i).map(Vec::as_slice).filter(|k| !k.is_empty())
    }

    fn parent(&self, i: usize) -> Option<usize> {
        self.parent.get(i).copied().flatten()
    }

    /// The rows below a summary that aren't summaries themselves, at any depth.
    fn leaves(&self, s: usize) -> impl Iterator<Item = usize> + '_ {
        (s + 1..self.outline.subtree_end(s)).filter(|&r| self.summary(r).is_none())
    }
}

/// Nodes in an order where every predecessor comes before its successors and every subtask before
/// its summary. Nodes `0..n` are the tasks (by row); node `n + s` is summary `s`'s hold. A
/// summary's predecessors come before its hold, its own summary's hold before its hold, and its
/// hold before each of its subtasks.
pub(crate) fn order(p: &Project) -> Result<Vec<usize>, ModelError> {
    order_with(p, &Tree::of(p))
}

fn order_with(p: &Project, tree: &Tree) -> Result<Vec<usize>, ModelError> {
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut waiting = vec![0usize; 2 * n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); 2 * n];
    let mut edge = |from: usize, to: usize| {
        if let (Some(w), Some(s)) = (waiting.get_mut(to), succs.get_mut(from)) {
            *w += 1;
            s.push(to);
        }
    };
    for (i, t) in p.tasks.iter().enumerate() {
        let kids = tree.summary(i);
        for l in &t.preds {
            if let Some(&j) = index.get(&l.pred) {
                // A summary's predecessors feed its hold.
                edge(j, if kids.is_some() { n + i } else { i });
            }
        }
        if let Some(kids) = kids {
            if let Some(up) = tree.parent(i) {
                edge(n + up, n + i);
            }
            for &k in kids {
                // Roll-up: a summary waits for its subtasks. Its hold comes before them.
                edge(k, i);
                edge(n + i, k);
            }
        }
    }
    let mut ready: Vec<usize> = (0..2 * n).filter(|&i| waiting.get(i) == Some(&0)).rev().collect();
    let mut out = Vec::with_capacity(2 * n);
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
    if out.len() == 2 * n {
        return Ok(out);
    }
    let stuck = (0..2 * n)
        .find(|i| waiting.get(*i).is_some_and(|w| *w > 0))
        .and_then(|i| p.tasks.get(i.checked_sub(n).unwrap_or(i)))
        .map_or(TaskId(0), |t| t.id);
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

/// The early dates from one forward pass, by row.
struct Forward {
    es: Vec<i64>,
    ef: Vec<i64>,
    /// Early start for slack: where the links put the task before a hard constraint held it.
    early: Vec<i64>,
    conflict: Vec<bool>,
    driver: Vec<Driver>,
}

/// Where a task's links put its start, given its duration `dur`, and which link did it.
fn link_start(t: &Task, index: &HashMap<TaskId, usize>, es: &[i64], ef: &[i64], dur: i64) -> (i64, Driver) {
    let mut start = 0i64;
    let mut why = Driver::ProjectStart;
    for l in &t.preds {
        let Some(&j) = index.get(&l.pred) else { continue };
        let (pes, pef) = (es.get(j).copied().unwrap_or(0), ef.get(j).copied().unwrap_or(0));
        let lag = l.lag.clamp(-crate::MAX_DAYS, crate::MAX_DAYS);
        let s = match l.kind {
            LinkKind::FS => pef + lag,
            LinkKind::SS => pes + lag,
            LinkKind::FF => pef + lag - dur,
            LinkKind::SF => pes + lag - dur,
        };
        if s > start || (s == start && why == Driver::ProjectStart) {
            (start, why) = (s, Driver::Link(l.pred));
        }
    }
    (start, why)
}

/// What a constraint makes of a start the links put at `start` (driven by `why`), for a task of
/// `dur` days: the start it is held at, its early start for slack, what drove it, and whether a
/// link can't be honoured.
fn constrain(p: &Project, c: Constraint, start: i64, why: Driver, dur: i64) -> (i64, i64, Driver, bool) {
    // Where the links (and a No Earlier Than date) put it; then what a hard constraint makes of it.
    let natural = match c {
        Constraint::StartNoEarlierThan(cd) => start.max(start_offset(p, cd)),
        Constraint::FinishNoEarlierThan(cd) => start.max(finish_offset(p, cd) - dur),
        _ => start,
    };
    let must = matches!(c, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_));
    let held = match c {
        Constraint::StartNoLaterThan(cd) => natural.min(start_offset(p, cd)),
        Constraint::FinishNoLaterThan(cd) => natural.min(finish_offset(p, cd) - dur),
        Constraint::MustStartOn(cd) => start_offset(p, cd),
        Constraint::MustFinishOn(cd) => finish_offset(p, cd) - dur,
        _ => natural,
    }
    .max(0);
    let why = if natural > start || held != natural || must { Driver::Constraint } else { why };
    // Must Start/Finish On set the early dates too (ES = CD); the No Later Than types leave them.
    let soft_early = if must { held } else { natural };
    (held, soft_early, why, start > held)
}

/// One forward pass. `sdur` is each summary's duration as its finish-type links and constraint
/// see it.
fn forward(p: &Project, tree: &Tree, order: &[usize], dur: &[i64], sdur: &[i64]) -> Forward {
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let d = |i: usize| dur.get(i).copied().unwrap_or(0);
    let id = |i: usize| p.tasks.get(i).map_or(TaskId(0), |t| t.id);
    // Offsets stay small: at most MAX_TASKS chains of MAX_DAYS-bounded steps fit in i64.
    let mut f = Forward { es: vec![0; n], ef: vec![0; n], early: vec![0; n], conflict: vec![false; n], driver: vec![Driver::ProjectStart; n] };
    // Each summary's hold, and the summary whose own link or constraint set it (none: the project start).
    let mut hold = vec![0i64; n];
    let mut source: Vec<Option<usize>> = vec![None; n];
    // A subtask's start from its summary's hold, if that is later than `start`.
    let held_by = |hold: &[i64], source: &[Option<usize>], i: usize, start: i64| {
        let up = tree.parent(i)?;
        let h = hold.get(up).copied().unwrap_or(0);
        (h > start).then(|| (h, source.get(up).copied().flatten()))
    };
    for &node in order {
        if let Some(s) = node.checked_sub(n) {
            // A summary's hold: its links, at least its own summary's hold, then its constraint.
            let Some(t) = p.tasks.get(s).filter(|_| tree.summary(s).is_some()) else { continue };
            let ds = sdur.get(s).copied().unwrap_or(0);
            let (mut start, mut why) = link_start(t, &index, &f.es, &f.ef, ds);
            let mut src = Some(s);
            if let Some((h, up_src)) = held_by(&hold, &source, s, start) {
                (start, src) = (h, up_src);
                why = up_src.map_or(Driver::ProjectStart, |u| Driver::ParentSummary(id(u)));
            }
            let (held, _, why, conflict) = constrain(p, t.constraint, start, why, ds);
            if why == Driver::ProjectStart {
                src = None;
            } else if matches!(why, Driver::Link(_) | Driver::Constraint) {
                src = Some(s);
            }
            if let (Some(h), Some(sr), Some(w), Some(c)) = (hold.get_mut(s), source.get_mut(s), f.driver.get_mut(s), f.conflict.get_mut(s)) {
                *h = held;
                *sr = src;
                *w = why;
                *c = conflict;
            }
            continue;
        }
        let i = node;
        let Some(t) = p.tasks.get(i) else { continue };
        if let Some(k) = tree.summary(i) {
            roll_up(k, &mut f.es, &mut f.ef, i);
            let (s, e) = (f.es.get(i).copied().unwrap_or(0), f.ef.get(i).copied().unwrap_or(0));
            // The subtasks' own links can still take the summary past a No Later Than or Must date.
            let missed = match t.constraint {
                Constraint::StartNoLaterThan(cd) => s > start_offset(p, cd),
                Constraint::FinishNoLaterThan(cd) => e > finish_offset(p, cd),
                Constraint::MustStartOn(cd) => s > start_offset(p, cd),
                Constraint::MustFinishOn(cd) => e > finish_offset(p, cd),
                _ => false,
            };
            if let (Some(a), Some(c)) = (f.early.get_mut(i), f.conflict.get_mut(i)) {
                *a = s;
                *c |= missed;
            }
            continue;
        }
        let (mut start, mut why) = link_start(t, &index, &f.es, &f.ef, d(i));
        if let Some((h, src)) = held_by(&hold, &source, i, start) {
            start = h;
            why = src.map_or(Driver::ProjectStart, |u| Driver::ParentSummary(id(u)));
        }
        let (held, soft_early, why, conflict) = constrain(p, t.constraint, start, why, d(i));
        if let (Some(a), Some(b), Some(e), Some(c), Some(w)) =
            (f.es.get_mut(i), f.ef.get_mut(i), f.early.get_mut(i), f.conflict.get_mut(i), f.driver.get_mut(i))
        {
            *a = held;
            *b = held + d(i);
            *e = soft_early;
            *c = conflict;
            *w = why;
        }
    }
    f
}

pub fn schedule(p: &Project) -> Result<Schedule, ModelError> {
    let tree = Tree::of(p);
    let order = order_with(p, &tree)?;
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let summary = |i: usize| tree.summary(i);
    let dur: Vec<i64> = p.tasks.iter().map(|t| t.duration.clamp(0, crate::MAX_DAYS)).collect();

    // Forward pass. A summary's finish-type link or constraint takes its duration from a first pass.
    let finish_type = |t: &Task| {
        matches!(t.constraint, Constraint::FinishNoEarlierThan(_) | Constraint::FinishNoLaterThan(_) | Constraint::MustFinishOn(_))
            || t.preds.iter().any(|l| matches!(l.kind, LinkKind::FF | LinkKind::SF))
    };
    let mut fw = forward(p, &tree, &order, &dur, &vec![0; n]);
    if p.tasks.iter().enumerate().any(|(i, t)| summary(i).is_some() && finish_type(t)) {
        let sdur: Vec<i64> = fw.ef.iter().zip(&fw.es).map(|(f, s)| f - s).collect();
        fw = forward(p, &tree, &order, &dur, &sdur);
    }
    let Forward { es, ef, early, conflict, driver } = fw;
    let end = ef.iter().copied().max().unwrap_or(0);
    // Roll-up: from here on a summary's duration is its rolled-up one.
    let dur: Vec<i64> =
        (0..n)
            .map(|i| {
                if summary(i).is_some() {
                    ef.get(i).copied().unwrap_or(0) - es.get(i).copied().unwrap_or(0)
                } else {
                    dur.get(i).copied().unwrap_or(0)
                }
            })
            .collect();
    let d = |i: usize| dur.get(i).copied().unwrap_or(0);

    // Backward pass.
    let mut lf = vec![end; n];
    for &node in order.iter().rev() {
        if let Some(s) = node.checked_sub(n) {
            // A summary's hold: every subtask is final here, so the summary's late start and
            // finish (rolled up from them) bound its predecessors.
            let Some(t) = p.tasks.get(s) else { continue };
            if t.preds.is_empty() || summary(s).is_none() {
                continue;
            }
            let my_lf = tree.leaves(s).filter_map(|r| lf.get(r).copied()).max().unwrap_or(end);
            let my_ls = tree.leaves(s).filter_map(|r| lf.get(r).map(|x| x - d(r))).min().unwrap_or(my_lf);
            bound_preds(t, &index, &mut lf, my_ls, my_lf, &d);
            continue;
        }
        let i = node;
        let Some(t) = p.tasks.get(i) else { continue };
        let my_lf = lf.get(i).copied().unwrap_or(end);
        // A hard constraint caps the late finish: LS = CD (SNLT, MSO), LF = CD (FNLT, MFO).
        let cap = match t.constraint {
            Constraint::StartNoLaterThan(cd) => start_offset(p, cd) + d(i),
            Constraint::FinishNoLaterThan(cd) => finish_offset(p, cd),
            Constraint::MustStartOn(_) | Constraint::MustFinishOn(_) => ef.get(i).copied().unwrap_or(end),
            _ => end,
        };
        // A deadline caps the late finish too (LF = min(LF, deadline)) and never moves the task.
        let deadline = t.deadline.map_or(end, |dl| finish_offset(p, dl));
        let my_lf = my_lf.min(cap).min(deadline);
        // Roll-up: a summary's successors, constraint and deadline hold back each of its subtasks.
        if let Some(k) = summary(i) {
            for &c in k {
                if let Some(x) = lf.get_mut(c) {
                    *x = (*x).min(my_lf);
                }
            }
            continue;
        }
        if let Some(x) = lf.get_mut(i) {
            *x = my_lf;
        }
        bound_preds(t, &index, &mut lf, my_lf - d(i), my_lf, &d);
    }

    let mut ls: Vec<i64> = (0..n).map(|i| lf.get(i).copied().unwrap_or(end) - d(i)).collect();
    // Roll-up of the late dates: inner summaries sit lower in the outline, so go bottom-up.
    for i in (0..n).rev() {
        if let Some(k) = summary(i) {
            roll_up(k, &mut ls, &mut lf, i);
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
        let ls = ls.get(i).copied().unwrap_or(lfi - d(i));
        // Roll-up: a summary takes the tighter of its start and finish slack.
        let slack = if summary(i).is_some() { (ls - s).min(lfi - f) } else { ls - early.get(i).copied().unwrap_or(s) };
        let first = |o: i64| if d(i) == 0 { day(o - 1) } else { day(o) };
        let finish = last(s, f)?;
        tasks.insert(
            t.id,
            TaskDates {
                start: first(s)?,
                finish,
                late_start: first(ls)?,
                late_finish: last(ls, lfi)?,
                slack,
                critical: slack <= 0,
                constraint_conflict: conflict.get(i).copied().unwrap_or(false),
                deadline_missed: t.deadline.is_some_and(|dl| finish > dl),
                driver: driver.get(i).copied().unwrap_or(Driver::ProjectStart),
            },
        );
    }
    let rolled_up = (0..n).filter(|&i| summary(i).is_some()).filter_map(|i| p.tasks.get(i).map(|t| (t.id, d(i)))).collect();
    Ok(Schedule { tasks, finish: last(0, end)?, rolled_up })
}

/// The late finish each of a task's predecessors may have, given the task's late start and finish.
fn bound_preds(t: &Task, index: &HashMap<TaskId, usize>, lf: &mut [i64], my_ls: i64, my_lf: i64, d: &impl Fn(usize) -> i64) {
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

/// Roll-up: a summary's start is its subtasks' earliest, its finish their latest. `start` and
/// `finish` are early or late offsets.
fn roll_up(kids: &[usize], start: &mut [i64], finish: &mut [i64], summary: usize) {
    let s = kids.iter().filter_map(|&k| start.get(k).copied()).min().unwrap_or(0);
    let f = kids.iter().filter_map(|&k| finish.get(k).copied()).max().unwrap_or(s);
    if let (Some(a), Some(b)) = (start.get_mut(summary), finish.get_mut(summary)) {
        *a = s;
        *b = f.max(s);
    }
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

    /// A summary over A (3d, Mon–Wed) and B (2d, after A, Thu–Fri), then C after the summary.
    fn phase() -> (Project, [TaskId; 4]) {
        let mut p = Project::new("t", date("2026-01-05"));
        let s = p.add_task("Phase", 99).unwrap();
        let a = p.add_task("A", 3).unwrap();
        let b = p.add_task("B", 2).unwrap();
        let c = p.add_task("C", 1).unwrap();
        p.indent(&[a, b]).unwrap();
        p.set_preds(b, parse_preds("2").unwrap()).unwrap();
        p.set_preds(c, parse_preds("1").unwrap()).unwrap();
        (p, [s, a, b, c])
    }

    #[test]
    fn a_summary_rolls_up_its_subtasks_and_ignores_its_own_duration() {
        let (p, [s, a, b, c]) = phase();
        let sch = schedule(&p).unwrap();
        let t = |id| sch.tasks[&id];
        assert_eq!((t(s).start, t(s).finish), (date("2026-01-05"), date("2026-01-09")), "earliest start to latest finish");
        assert_eq!(sch.rolled_up.get(&s), Some(&5), "working days between, not its own 99");
        assert_eq!(sch.rolled_up.len(), 1);
        // The summary as a predecessor: C follows the summary's finish, the following Monday.
        assert_eq!(t(c).start, date("2026-01-12"));
        assert!(t(s).critical && t(a).critical && t(b).critical && t(c).critical);
        assert_eq!(sch.finish, date("2026-01-12"));
    }

    #[test]
    fn summaries_nest_and_take_their_slack_from_their_subtasks() {
        // Phase { A(3); Sub { B(1); D(1) } }; E(5) alone sets the project finish.
        let mut p = Project::new("t", date("2026-01-05"));
        let phase = p.add_task("Phase", 1).unwrap();
        let a = p.add_task("A", 3).unwrap();
        let sub = p.add_task("Sub", 1).unwrap();
        let b = p.add_task("B", 1).unwrap();
        let dd = p.add_task("D", 1).unwrap();
        let e = p.add_task("E", 5).unwrap();
        p.indent(&[a, sub, b, dd]).unwrap();
        p.indent(&[b, dd]).unwrap();
        p.set_preds(dd, parse_preds("4").unwrap()).unwrap();
        let sch = schedule(&p).unwrap();
        let t = |id| sch.tasks[&id];
        assert_eq!((t(sub).start, t(sub).finish), (date("2026-01-05"), date("2026-01-06")));
        assert_eq!((t(phase).start, t(phase).finish), (date("2026-01-05"), date("2026-01-07")));
        assert_eq!((sch.rolled_up[&sub], sch.rolled_up[&phase]), (2, 3));
        assert!(t(e).critical);
        assert_eq!((t(a).slack, t(sub).slack, t(phase).slack), (2, 3, 2));
        assert_eq!((t(phase).late_start, t(phase).late_finish), (date("2026-01-07"), date("2026-01-09")));
    }

    #[test]
    fn a_summary_of_milestones_has_no_duration() {
        let mut p = Project::new("t", date("2026-01-05"));
        let s = p.add_task("Gates", 4).unwrap();
        let m = p.add_task("Gate", 0).unwrap();
        p.indent(&[m]).unwrap();
        let sch = schedule(&p).unwrap();
        assert_eq!(sch.rolled_up[&s], 0);
        assert_eq!(sch.tasks[&s].start, sch.tasks[&m].start);
    }

    #[test]
    fn a_calendar_with_no_working_days_is_an_error() {
        let mut p = Project::new("t", date("2026-01-05"));
        p.add_task("A", 1).unwrap();
        p.calendar.working = [false; 7];
        assert!(schedule(&p).is_err());
    }

    proptest::proptest! {
        /// Any outline with any links and constraints (as a damaged file could hold them, loops and
        /// summary-to-own-subtask links included) schedules or reports an error, never panics, and
        /// gives the same dates twice.
        #[test]
        fn summaries_with_links_and_constraints_never_panic(
            levels in proptest::collection::vec(0u32..5, 1..10),
            links in proptest::collection::vec((0usize..10, 0u32..12, 0u8..4, -3i64..4), 0..12),
            constraints in proptest::collection::vec((0usize..10, 0u8..7, 0i64..30), 0..5),
        ) {
            let mut p = Project::new("t", date("2026-01-05"));
            for (i, l) in levels.iter().enumerate() {
                let id = p.add_task(format!("T{i}"), (i % 4) as i64).unwrap();
                p.task_mut(id).unwrap().outline_level = *l;
            }
            let ids: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
            let kinds = [LinkKind::FS, LinkKind::SS, LinkKind::FF, LinkKind::SF];
            for (succ, pred, kind, lag) in links {
                if let Some(&id) = ids.get(succ) {
                    p.task_mut(id).unwrap().preds.push(crate::Link { pred: TaskId(pred), kind: kinds[usize::from(kind)], lag });
                }
            }
            for (row, ty, day) in constraints {
                let d = date("2026-01-01").add_days(day).unwrap();
                let c = [Constraint::Asap, Constraint::StartNoEarlierThan(d), Constraint::FinishNoEarlierThan(d), Constraint::StartNoLaterThan(d),
                    Constraint::FinishNoLaterThan(d), Constraint::MustStartOn(d), Constraint::MustFinishOn(d)][usize::from(ty)];
                if let Some(&id) = ids.get(row) {
                    p.task_mut(id).unwrap().constraint = c;
                }
            }
            let first = schedule(&p);
            proptest::prop_assert_eq!(&first, &schedule(&p));
        }
    }
}
