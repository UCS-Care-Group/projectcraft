//! Critical-path scheduling: a forward pass for the early dates, a backward pass for the late
//! dates, slack as the difference. Times are offsets in working minutes from the first working
//! minute on or after the project start, counted on the calendar's working times; at the end each
//! is turned into a [`Moment`] (a date and a minute of the day) through the calendar. Durations,
//! lags and slack are working minutes. Integer arithmetic only.
//!
//! Constraints: Start No Earlier Than raises the early start to its date, Finish No Earlier Than
//! the early finish (ES = max(ES, CD), EF = max(EF, CD)). A constraint date is a day: a start type
//! means the start of that day's working time, a finish type its end. On a non-working day that is
//! the start of the next working day for a start type and the end of the previous one for a finish
//! type.
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
//! A milestone is a moment, not a day: at the end of a working day (after a finish-to-start link,
//! on its predecessor's finish date) or at the start of one (constrained to start on a date, on
//! that date). The end of one working day and the start of the next are the same working minute
//! offset; which side it is decides the moment. See [`moments`].
//!
//! A deadline is not a constraint: it never moves a task, but caps its late finish
//! (LF = min(LF, deadline)), so slack counts from the deadline when that is earlier, and a finish
//! after it gives negative slack that the late dates carry back to the predecessors, as Microsoft's
//! Deadline field and "Show slack" pages describe. A deadline is the end of its day's working
//! time; on a non-working day, the end of the previous working day. The task is flagged ([`TaskDates::deadline_missed`]) when it finishes
//! after the deadline. On a summary, the cap reaches every subtask.
//!
//! Summary tasks ([`Outline`]) are rolled up, not scheduled: a summary starts at its earliest
//! subtask's start and finishes at its latest subtask's finish, and its duration is the working
//! time between. Each subtask comes before its summary in [`order`], so a summary's dates are
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
//! A finish-type link or constraint on a summary (FF, SF, Finish No Earlier/Later Than, Must
//! Finish On) is about the summary's finish, which its subtasks set, so the hold is searched for:
//! the earliest hold at which the summary finishes on or after the date (for Finish No Later Than,
//! the latest at which it finishes by it), trying a hold by scheduling the summary's subtasks again
//! from it. A later hold never makes the finish earlier, and moves it at most a minute per minute, so a
//! few tries land on the date exactly, even when a subtask's own link or constraint makes the span
//! depend on the hold. The search is bounded (see `HOLD_STEPS`).
//!
//! ponytail: schedules forward from the project start, with no actual progress and no resource
//! levelling; each is its own step on the roadmap. A Must Start On before the project start is
//! held at the project start.

use std::collections::{BTreeMap, HashMap};

use crate::{Constraint, Date, LinkKind, MAX_MINUTES, ModelError, Outline, Project, Task, TaskId};

/// A point in time on the calendar: a date and a minute of that day (0 is midnight, 1440 the
/// midnight after). A task starts at the start of a working minute and finishes at the end of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Moment {
    pub date: Date,
    pub minute: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskDates {
    /// The day of [`TaskDates::start_at`].
    pub start: Date,
    /// The last working day of the task, the day of [`TaskDates::finish_at`]; the start for a milestone.
    pub finish: Date,
    /// When it starts: the start of its first working minute (a milestone: its moment).
    pub start_at: Moment,
    /// When it finishes: the end of its last working minute (a milestone: its moment).
    pub finish_at: Moment,
    pub late_start: Date,
    pub late_finish: Date,
    /// Total slack in working minutes.
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
    /// Each summary task's duration in working minutes, rolled up from its subtasks.
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

/// Each constraint and deadline date's offsets, worked out once from the calendar: the passes
/// (and the finish-type hold searches, which run them many times) only look them up.
struct Days {
    /// The project start: a date before it is offset 0.
    start: Date,
    /// Date on or after the start → (start of its working time, end of its working time).
    at: HashMap<Date, (i64, i64)>,
}

impl Days {
    fn of(p: &Project) -> Days {
        let mut at = HashMap::new();
        for cd in p.tasks.iter().flat_map(|t| t.constraint.date().into_iter().chain(t.deadline)).filter(|cd| *cd >= p.start) {
            at.entry(cd).or_insert_with(|| {
                let start = p.calendar.minutes_between(p.start, cd);
                (start, start + p.calendar.minutes_on(cd))
            });
        }
        Days { start: p.start, at }
    }
}

/// The offset of the start of `cd`'s working time (on a day off, the next working day's): where a
/// task constrained to start on `cd` begins. Before the project start, 0.
fn start_offset(o: &Days, cd: Date) -> i64 {
    if cd <= o.start { 0 } else { o.at.get(&cd).map_or(0, |x| x.0) }
}

/// The offset of the end of `cd`'s working time (on a day off, the previous working day's): where
/// a task constrained to finish on `cd` ends. Before the project start, 0.
fn finish_offset(o: &Days, cd: Date) -> i64 {
    if cd < o.start { 0 } else { o.at.get(&cd).map_or(0, |x| x.1) }
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
        let lag = l.lag.clamp(-MAX_MINUTES, MAX_MINUTES);
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
fn constrain(o: &Days, c: Constraint, start: i64, why: Driver, dur: i64) -> (i64, i64, Driver, bool) {
    // Where the links (and a No Earlier Than date) put it; then what a hard constraint makes of it.
    let natural = match c {
        Constraint::StartNoEarlierThan(cd) => start.max(start_offset(o, cd)),
        Constraint::FinishNoEarlierThan(cd) => start.max(finish_offset(o, cd) - dur),
        _ => start,
    };
    let must = matches!(c, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_));
    let held = match c {
        Constraint::StartNoLaterThan(cd) => natural.min(start_offset(o, cd)),
        Constraint::FinishNoLaterThan(cd) => natural.min(finish_offset(o, cd) - dur),
        Constraint::MustStartOn(cd) => start_offset(o, cd),
        Constraint::MustFinishOn(cd) => finish_offset(o, cd) - dur,
        _ => natural,
    }
    .max(0);
    let why = if natural > start || held != natural || must { Driver::Constraint } else { why };
    // Must Start/Finish On set the early dates too (ES = CD); the No Later Than types leave them.
    let soft_early = if must { held } else { natural };
    (held, soft_early, why, start > held)
}

/// Bounds on the search for a finish-type hold (below), so a damaged file can't make the pass
/// spin. Each summary's hold is searched for in the pass itself until `HOLD_STEPS` scheduling
/// steps are taken in all; a summary inside one being searched for is searched for again at each
/// try, so nested searches stop `HOLD_DEPTH` deep or after `HOLD_NESTED_STEPS` steps. A step counts
/// one plus the task's links. Past a bound a summary keeps its start-type hold, the same way on
/// every run. The bounds keep the worst file a full-size plan can hold to a few hundred
/// milliseconds a reschedule in release (`cargo xtask perf` times it).
const HOLD_DEPTH: u32 = 8;
const HOLD_NESTED_STEPS: u64 = 2_000_000;
const HOLD_STEPS: u64 = 8_000_000;
/// Jumps straight to the earliest hold the finish allows, before halving what range is left.
const HOLD_JUMPS: u32 = 8;

/// The forward pass: each node in [`order`] in turn, and each summary's hold.
struct Pass<'a> {
    p: &'a Project,
    o: &'a Days,
    tree: &'a Tree,
    order: &'a [usize],
    /// Where each node sits in `order`.
    at: Vec<usize>,
    index: HashMap<TaskId, usize>,
    dur: &'a [i64],
    f: Forward,
    /// Each summary's hold, and the summary whose own link or constraint set it (none: the
    /// project start).
    hold: Vec<i64>,
    source: Vec<Option<usize>>,
    /// Scheduling steps taken, finish-type hold searches included.
    steps: u64,
}

/// One forward pass.
fn forward(p: &Project, o: &Days, tree: &Tree, order: &[usize], dur: &[i64]) -> Forward {
    let n = p.tasks.len();
    let mut at = vec![0usize; 2 * n];
    for (k, &node) in order.iter().enumerate() {
        if let Some(x) = at.get_mut(node) {
            *x = k;
        }
    }
    // Offsets stay small: at most MAX_TASKS chains of MAX_MINUTES-bounded steps fit in i64.
    let f = Forward { es: vec![0; n], ef: vec![0; n], early: vec![0; n], conflict: vec![false; n], driver: vec![Driver::ProjectStart; n] };
    let index = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut pass = Pass { p, o, tree, order, at, index, dur, f, hold: vec![0; n], source: vec![None; n], steps: 0 };
    pass.run(0, order.len(), 0);
    pass.f
}

impl Pass<'_> {
    /// The nodes at `from..to` in `order`.
    fn run(&mut self, from: usize, to: usize, depth: u32) {
        for k in from..to {
            let Some(&node) = self.order.get(k) else { break };
            let row = node.checked_sub(self.p.tasks.len()).unwrap_or(node);
            let links = self.p.tasks.get(row).map_or(0, |t| t.preds.len());
            self.steps = self.steps.saturating_add(1).saturating_add(u64::try_from(links).unwrap_or(u64::MAX));
            self.step(node, depth);
        }
    }

    fn id(&self, i: usize) -> TaskId {
        self.p.tasks.get(i).map_or(TaskId(0), |t| t.id)
    }

    /// A subtask's start from its summary's hold, if that is later than `start`.
    fn held_by(&self, i: usize, start: i64) -> Option<(i64, Option<usize>)> {
        let up = self.tree.parent(i)?;
        let h = self.hold.get(up).copied().unwrap_or(0);
        (h > start).then(|| (h, self.source.get(up).copied().flatten()))
    }

    fn step(&mut self, node: usize, depth: u32) {
        let (p, o, tree) = (self.p, self.o, self.tree);
        if let Some(s) = node.checked_sub(p.tasks.len()) {
            self.summary_hold(s, depth);
            return;
        }
        let i = node;
        let Some(t) = p.tasks.get(i) else { return };
        if let Some(k) = tree.summary(i) {
            let f = &mut self.f;
            roll_up(k, &mut f.es, &mut f.ef, i);
            let (s, e) = (f.es.get(i).copied().unwrap_or(0), f.ef.get(i).copied().unwrap_or(0));
            // The subtasks' own links can still take the summary past a No Later Than or Must date.
            let missed = match t.constraint {
                Constraint::StartNoLaterThan(cd) => s > start_offset(o, cd),
                Constraint::FinishNoLaterThan(cd) => e > finish_offset(o, cd),
                Constraint::MustStartOn(cd) => s > start_offset(o, cd),
                Constraint::MustFinishOn(cd) => e > finish_offset(o, cd),
                _ => false,
            };
            if let (Some(a), Some(c)) = (f.early.get_mut(i), f.conflict.get_mut(i)) {
                *a = s;
                *c |= missed;
            }
            return;
        }
        let d = self.dur.get(i).copied().unwrap_or(0);
        let (mut start, mut why) = link_start(t, &self.index, &self.f.es, &self.f.ef, d);
        if let Some((h, src)) = self.held_by(i, start) {
            start = h;
            why = src.map_or(Driver::ProjectStart, |u| Driver::ParentSummary(self.id(u)));
        }
        let (held, soft_early, why, conflict) = constrain(self.o, t.constraint, start, why, d);
        let f = &mut self.f;
        if let (Some(a), Some(b), Some(e), Some(c), Some(w)) =
            (f.es.get_mut(i), f.ef.get_mut(i), f.early.get_mut(i), f.conflict.get_mut(i), f.driver.get_mut(i))
        {
            *a = held;
            *b = held + d;
            *e = soft_early;
            *c = conflict;
            *w = why;
        }
    }

    /// A summary's hold: its start-type links, at least its own summary's hold, then its
    /// finish-type links, then its constraint, per Microsoft's definitions. A finish-type link or
    /// constraint holds the subtasks just as much as lets the summary finish on (or by) its date.
    fn summary_hold(&mut self, s: usize, depth: u32) {
        let (p, o) = (self.p, self.o);
        let Some(t) = p.tasks.get(s).filter(|_| self.tree.summary(s).is_some()) else { return };
        // FS and SS: the earliest the subtasks may start. FF and SF: the earliest the summary may finish.
        let (mut start, mut why) = (0i64, Driver::ProjectStart);
        let mut finish: Option<(i64, Driver)> = None;
        for l in &t.preds {
            let Some(&j) = self.index.get(&l.pred) else { continue };
            let (pes, pef) = (self.f.es.get(j).copied().unwrap_or(0), self.f.ef.get(j).copied().unwrap_or(0));
            let lag = l.lag.clamp(-MAX_MINUTES, MAX_MINUTES);
            match l.kind {
                LinkKind::FS | LinkKind::SS => {
                    let x = if l.kind == LinkKind::FS { pef } else { pes } + lag;
                    if x > start || (x == start && why == Driver::ProjectStart) {
                        (start, why) = (x, Driver::Link(l.pred));
                    }
                }
                LinkKind::FF | LinkKind::SF => {
                    let x = if l.kind == LinkKind::FF { pef } else { pes } + lag;
                    if finish.is_none_or(|(f, _)| x > f) {
                        finish = Some((x, Driver::Link(l.pred)));
                    }
                }
            }
        }
        let mut src = Some(s);
        if let Some((h, up_src)) = self.held_by(s, start) {
            (start, src) = (h, up_src);
            why = up_src.map_or(Driver::ProjectStart, |u| Driver::ParentSummary(self.id(u)));
        }
        if let Some((target, w)) = finish {
            let h = self.earliest_hold(s, start, target, depth);
            if h > start {
                (start, why) = (h, w);
            }
        }
        // Where the links put it (and a No Earlier Than date); then what a hard constraint makes of it.
        let natural = match t.constraint {
            Constraint::StartNoEarlierThan(cd) => start.max(start_offset(o, cd)),
            Constraint::FinishNoEarlierThan(cd) => self.earliest_hold(s, start, finish_offset(o, cd), depth),
            _ => start,
        };
        let must = matches!(t.constraint, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_));
        let held = match t.constraint {
            Constraint::StartNoLaterThan(cd) => natural.min(start_offset(o, cd)),
            // The latest hold that finishes by the date: one before the earliest that finishes after it.
            Constraint::FinishNoLaterThan(cd) => {
                natural.min(self.earliest_hold(s, 0, finish_offset(o, cd).saturating_add(1), depth).saturating_sub(1))
            }
            Constraint::MustStartOn(cd) => start_offset(o, cd),
            Constraint::MustFinishOn(cd) => self.earliest_hold(s, 0, finish_offset(o, cd), depth),
            _ => natural,
        }
        .max(0);
        let why = if natural > start || held != natural || must { Driver::Constraint } else { why };
        let conflict = start > held;
        if why == Driver::ProjectStart {
            src = None;
        } else if matches!(why, Driver::Link(_) | Driver::Constraint) {
            src = Some(s);
        }
        let f = &mut self.f;
        if let (Some(h), Some(sr), Some(w), Some(c)) = (self.hold.get_mut(s), self.source.get_mut(s), f.driver.get_mut(s), f.conflict.get_mut(s)) {
            *h = held;
            *sr = src;
            *w = why;
            *c = conflict;
        }
    }

    /// Where summary `s` finishes (its early finish offset) if its hold is `h`: the pass from its
    /// hold to its roll-up, run again. Every node there is worked out again by the pass itself
    /// before anything reads it, so nothing is left behind. None once the bounds are spent.
    fn finish_with_hold(&mut self, s: usize, h: i64, depth: u32) -> Option<i64> {
        let limit = if depth == 0 { HOLD_STEPS } else { HOLD_NESTED_STEPS };
        if depth >= HOLD_DEPTH || self.steps >= limit {
            return None;
        }
        let n = self.p.tasks.len();
        let (from, to) = (self.at.get(n + s)?.saturating_add(1), self.at.get(s)?.saturating_add(1));
        *self.hold.get_mut(s)? = h;
        self.run(from, to, depth + 1);
        self.f.ef.get(s).copied()
    }

    /// The earliest hold from `lo` on at which summary `s` finishes at `target` or later. The
    /// finish never moves earlier as the hold moves later, and at most a minute per minute, so
    /// `lo + (target − finish)` is never past the answer: jump there, then halve what is left.
    fn earliest_hold(&mut self, s: usize, mut lo: i64, target: i64, depth: u32) -> i64 {
        for _ in 0..HOLD_JUMPS {
            let Some(f) = self.finish_with_hold(s, lo, depth) else { return lo };
            if f >= target {
                return lo;
            }
            lo = lo.saturating_add(target.saturating_sub(f)).min(lo.max(target));
        }
        // From `target` on, any subtask that follows the hold finishes after it.
        let mut hi = lo.max(target);
        match self.finish_with_hold(s, hi, depth) {
            Some(f) if f >= target => {}
            _ => return hi,
        }
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.finish_with_hold(s, mid, depth) {
                Some(f) if f >= target => hi = mid,
                Some(_) => lo = mid + 1,
                None => return hi,
            }
        }
        hi
    }
}

/// A point in working time: a working-minute offset, and whether it is the start of the working
/// minute there (`true`) or the end of the one before (`false`). Both are the same working time,
/// but across a break (lunch, a night, a weekend) the start of the next working time is later on
/// the clock, so it wins a tie in `max` and loses one in `min`. A milestone shows the moment its
/// point is.
type Point = (i64, bool);

/// Each row's start and finish as points, for the moments a milestone (and a summary over one)
/// shows; a task with duration starts at the start of a working minute and finishes at the end of
/// one. A link from a finish or a finish-type constraint puts a milestone at the end of working
/// time, a link from a start or a start-type constraint at the start of it, so a milestone after a
/// task shows the task's finish and one constrained to start on a date shows that date, as in
/// Microsoft Project. The offsets are the forward pass's: only the side is worked out here.
fn moments(p: &Project, o: &Days, tree: &Tree, order: &[usize], es: &[i64], ef: &[i64], d: &impl Fn(usize) -> i64) -> (Vec<Point>, Vec<Point>) {
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut start: Vec<Point> = es.iter().map(|&s| (s, true)).collect();
    let mut finish: Vec<Point> = ef.iter().map(|&f| (f, false)).collect();
    let mut hold: Vec<Point> = vec![(0, true); n];
    // A start worked back from a finish is the start of a day for a task with duration.
    let back = |o: i64, side: bool, dur: i64| (o, side || dur > 0);
    for &node in order {
        let (i, is_hold) = node.checked_sub(n).map_or((node, false), |s| (s, true));
        let Some(t) = p.tasks.get(i) else { continue };
        let kids = tree.summary(i);
        if let (Some(k), false) = (kids, is_hold) {
            // Roll-up: the earliest start and the latest finish of the subtasks.
            let s = k.iter().filter_map(|&c| start.get(c).copied()).min();
            let f = k.iter().filter_map(|&c| finish.get(c).copied()).max();
            if let (Some(s), Some(a)) = (s.filter(|s| Some(&s.0) == es.get(i)), start.get_mut(i)) {
                *a = s;
            }
            if let (Some(f), Some(b)) = (f.filter(|f| Some(&f.0) == ef.get(i)), finish.get_mut(i)) {
                *b = f;
            }
            continue;
        }
        if is_hold && kids.is_none() || !is_hold && d(i) > 0 {
            continue;
        }
        let dur = d(i);
        let mut m: Point = (0, true);
        for l in &t.preds {
            let Some(&j) = index.get(&l.pred) else { continue };
            let (Some(&ps), Some(&pf)) = (start.get(j), finish.get(j)) else { continue };
            let lag = l.lag.clamp(-MAX_MINUTES, MAX_MINUTES);
            m = m.max(match l.kind {
                LinkKind::FS => (pf.0 + lag, pf.1),
                LinkKind::SS => (ps.0 + lag, ps.1),
                LinkKind::FF => back(pf.0 + lag - dur, pf.1, dur),
                LinkKind::SF => back(ps.0 + lag - dur, ps.1, dur),
            });
        }
        if let Some(&h) = tree.parent(i).and_then(|up| hold.get(up)) {
            m = m.max(h);
        }
        let on = |cd: Date| (start_offset(o, cd), true);
        let by = |cd: Date| back(finish_offset(o, cd) - dur, false, dur);
        m = match t.constraint {
            Constraint::StartNoEarlierThan(cd) => m.max(on(cd)),
            Constraint::FinishNoEarlierThan(cd) => m.max(by(cd)),
            Constraint::StartNoLaterThan(cd) => m.min(on(cd)),
            Constraint::FinishNoLaterThan(cd) => m.min(by(cd)),
            Constraint::MustStartOn(cd) => on(cd),
            Constraint::MustFinishOn(cd) => by(cd),
            _ => m,
        };
        if m.0 < 0 {
            m = (0, true);
        }
        if is_hold {
            if let Some(h) = hold.get_mut(i) {
                *h = m;
            }
            continue;
        }
        // A milestone: where the forward pass put it, on the side of the day worked out here.
        let at = es.get(i).copied().unwrap_or(0);
        let m = if m.0 == at { m } else { (at, false) };
        if let (Some(a), Some(b)) = (start.get_mut(i), finish.get_mut(i)) {
            (*a, *b) = (m, m);
        }
    }
    (start, finish)
}

pub fn schedule(p: &Project) -> Result<Schedule, ModelError> {
    let tree = Tree::of(p);
    let order = order_with(p, &tree)?;
    let o = Days::of(p);
    let o = &o;
    let n = p.tasks.len();
    let index: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let summary = |i: usize| tree.summary(i);
    let dur: Vec<i64> = p.tasks.iter().map(|t| t.duration.clamp(0, MAX_MINUTES)).collect();

    // Forward pass.
    let Forward { es, ef, early, conflict, driver } = forward(p, o, &tree, &order, &dur);
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
            Constraint::StartNoLaterThan(cd) => start_offset(o, cd) + d(i),
            Constraint::FinishNoLaterThan(cd) => finish_offset(o, cd),
            Constraint::MustStartOn(_) | Constraint::MustFinishOn(_) => ef.get(i).copied().unwrap_or(end),
            _ => end,
        };
        // A deadline caps the late finish too (LF = min(LF, deadline)) and never moves the task.
        let deadline = t.deadline.map_or(end, |dl| finish_offset(o, dl));
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
    let clock = Clock { p, days: p.calendar.working_days(p.start, end, MAX_CALENDAR_DAYS).ok_or_else(bad)? };
    let day = |off: i64| clock.date(off).ok_or_else(bad);
    // A task occupying [s, f) working minutes ends in minute f-1. A milestone at offset s is the
    // end of minute s-1 (it sits on its predecessor's finish) or the start of minute s (a
    // start-type constraint's date), as in Microsoft Project: see `moments`.
    let last = |s: i64, f: i64| day(if f > s { f - 1 } else { s - 1 });
    let (start_m, finish_m) = moments(p, o, &tree, &order, &es, &ef, &d);
    let mut tasks = HashMap::with_capacity(n);
    let mut finish_all = last(0, end)?;
    for (i, t) in p.tasks.iter().enumerate() {
        let (s, f, lfi) = (es.get(i).copied().unwrap_or(0), ef.get(i).copied().unwrap_or(0), lf.get(i).copied().unwrap_or(end));
        let ls = ls.get(i).copied().unwrap_or(lfi - d(i));
        // Roll-up: a summary takes the tighter of its start and finish slack.
        let slack = if summary(i).is_some() { (ls - s).min(lfi - f) } else { ls - early.get(i).copied().unwrap_or(s) };
        let first = |o: i64| if d(i) == 0 { day(o - 1) } else { day(o) };
        let (sm, fm) = (start_m.get(i).copied().unwrap_or((s, d(i) > 0)), finish_m.get(i).copied().unwrap_or((f, false)));
        let (start_at, finish_at) = (clock.moment(sm).ok_or_else(bad)?, clock.moment(fm).ok_or_else(bad)?);
        let finish = finish_at.date;
        finish_all = finish_all.max(finish);
        // A milestone at the start of its day has its late dates there too when they are its
        // early ones (no slack) or a start-type constraint's date holds them.
        let late_on_day = d(i) == 0
            && summary(i).is_none()
            && ((lfi == s && sm.1) || matches!(t.constraint, Constraint::StartNoLaterThan(cd) if lfi == start_offset(o, cd)));
        let (late_start, late_finish) = if late_on_day { (day(lfi)?, day(lfi)?) } else { (first(ls)?, last(ls, lfi)?) };
        tasks.insert(
            t.id,
            TaskDates {
                start: start_at.date,
                finish,
                start_at,
                finish_at,
                late_start,
                late_finish,
                slack,
                critical: slack <= 0,
                constraint_conflict: conflict.get(i).copied().unwrap_or(false),
                deadline_missed: t.deadline.is_some_and(|dl| finish > dl),
                driver: driver.get(i).copied().unwrap_or(Driver::ProjectStart),
            },
        );
    }
    let rolled_up = (0..n).filter(|&i| summary(i).is_some()).filter_map(|i| p.tasks.get(i).map(|t| (t.id, d(i)))).collect();
    Ok(Schedule { tasks, finish: finish_all, rolled_up })
}

/// Most working days a schedule spans: past year 9999 from any start, so only a calendar of a
/// few working minutes a day can reach it. Bounds the calendar walk.
const MAX_CALENDAR_DAYS: usize = 4_000_000;

/// Working-minute offsets turned into dates and moments: the working days from the project start,
/// each with the working minutes before it.
struct Clock<'a> {
    p: &'a Project,
    days: Vec<(Date, i64)>,
}

impl Clock<'_> {
    /// The working day minute `off` falls in, and the minutes of that day's working time before it.
    /// An offset before the project start is its first working minute.
    fn find(&self, off: i64) -> Option<(Date, i64)> {
        let off = off.max(0);
        let k = self.days.partition_point(|&(_, before)| before <= off).checked_sub(1)?;
        self.days.get(k).map(|&(d, before)| (d, off - before))
    }

    fn date(&self, off: i64) -> Option<Date> {
        self.find(off).map(|(d, _)| d)
    }

    /// The moment a point is: the start of its working minute, or the end of the minute before.
    fn moment(&self, at: Point) -> Option<Moment> {
        let end = !at.1 && at.0 > 0;
        let (date, mut into) = self.find(if end { at.0 - 1 } else { at.0 })?;
        for t in self.p.calendar.times_on(date) {
            let len = i64::from(t.minutes());
            if into < len {
                let minute = i64::from(t.from) + into + i64::from(end);
                return Some(Moment { date, minute: u16::try_from(minute).ok()? });
            }
            into -= len;
        }
        None
    }
}

/// The late finish each of a task's predecessors may have, given the task's late start and finish.
fn bound_preds(t: &Task, index: &HashMap<TaskId, usize>, lf: &mut [i64], my_ls: i64, my_lf: i64, d: &impl Fn(usize) -> i64) {
    for l in &t.preds {
        let Some(&j) = index.get(&l.pred) else { continue };
        let lag = l.lag.clamp(-MAX_MINUTES, MAX_MINUTES);
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

    /// A day of duration at the default 8 hours.
    const D: i64 = 480;

    fn date(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[test]
    fn critical_path_over_a_weekend() {
        // Monday 2026-01-05. A(3) -> B(2), A -> C(1), B,C -> D(0 milestone).
        let mut p = Project::new("t", date("2026-01-05"));
        let a = p.add_task("A", 3 * D).unwrap();
        let b = p.add_task("B", 2 * D).unwrap();
        let c = p.add_task("C", D).unwrap();
        let m = p.add_task("D", 0).unwrap();
        p.set_preds(b, parse_preds("1", D).unwrap()).unwrap();
        p.set_preds(c, parse_preds("1", D).unwrap()).unwrap();
        p.set_preds(m, parse_preds("2,3", D).unwrap()).unwrap();
        let s = schedule(&p).unwrap();
        let t = |id| s.tasks[&id];
        assert_eq!((t(a).start, t(a).finish), (date("2026-01-05"), date("2026-01-07")));
        // B: Thu, Fri.
        assert_eq!((t(b).start, t(b).finish), (date("2026-01-08"), date("2026-01-09")));
        assert!(t(a).critical && t(b).critical && t(m).critical);
        assert_eq!(t(c).slack, D);
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
        let a = p.add_task("A", 4 * D).unwrap();
        let b = p.add_task("B", 2 * D).unwrap();
        p.set_preds(b, parse_preds("1SS+1", D).unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].start, date("2026-01-06"));
        p.set_preds(b, parse_preds("1FF", D).unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].finish, schedule(&p).unwrap().tasks[&a].finish);
        p.set_preds(b, parse_preds("1FS-2", D).unwrap()).unwrap();
        assert_eq!(schedule(&p).unwrap().tasks[&b].start, date("2026-01-07"));
    }

    /// A summary over A (3d, Mon–Wed) and B (2d, after A, Thu–Fri), then C after the summary.
    fn phase() -> (Project, [TaskId; 4]) {
        let mut p = Project::new("t", date("2026-01-05"));
        let s = p.add_task("Phase", 99 * D).unwrap();
        let a = p.add_task("A", 3 * D).unwrap();
        let b = p.add_task("B", 2 * D).unwrap();
        let c = p.add_task("C", D).unwrap();
        p.indent(&[a, b]).unwrap();
        p.set_preds(b, parse_preds("2", D).unwrap()).unwrap();
        p.set_preds(c, parse_preds("1", D).unwrap()).unwrap();
        (p, [s, a, b, c])
    }

    #[test]
    fn a_summary_rolls_up_its_subtasks_and_ignores_its_own_duration() {
        let (p, [s, a, b, c]) = phase();
        let sch = schedule(&p).unwrap();
        let t = |id| sch.tasks[&id];
        assert_eq!((t(s).start, t(s).finish), (date("2026-01-05"), date("2026-01-09")), "earliest start to latest finish");
        assert_eq!(sch.rolled_up.get(&s), Some(&(5 * D)), "working time between, not its own 99");
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
        let phase = p.add_task("Phase", D).unwrap();
        let a = p.add_task("A", 3 * D).unwrap();
        let sub = p.add_task("Sub", D).unwrap();
        let b = p.add_task("B", D).unwrap();
        let dd = p.add_task("D", D).unwrap();
        let e = p.add_task("E", 5 * D).unwrap();
        p.indent(&[a, sub, b, dd]).unwrap();
        p.indent(&[b, dd]).unwrap();
        p.set_preds(dd, parse_preds("4", D).unwrap()).unwrap();
        let sch = schedule(&p).unwrap();
        let t = |id| sch.tasks[&id];
        assert_eq!((t(sub).start, t(sub).finish), (date("2026-01-05"), date("2026-01-06")));
        assert_eq!((t(phase).start, t(phase).finish), (date("2026-01-05"), date("2026-01-07")));
        assert_eq!((sch.rolled_up[&sub], sch.rolled_up[&phase]), (2 * D, 3 * D));
        assert!(t(e).critical);
        assert_eq!((t(a).slack, t(sub).slack, t(phase).slack), (2 * D, 3 * D, 2 * D));
        assert_eq!((t(phase).late_start, t(phase).late_finish), (date("2026-01-07"), date("2026-01-09")));
    }

    #[test]
    fn a_summary_of_milestones_has_no_duration() {
        let mut p = Project::new("t", date("2026-01-05"));
        let s = p.add_task("Gates", 4 * D).unwrap();
        let m = p.add_task("Gate", 0).unwrap();
        p.indent(&[m]).unwrap();
        let sch = schedule(&p).unwrap();
        assert_eq!(sch.rolled_up[&s], 0);
        assert_eq!(sch.tasks[&s].start, sch.tasks[&m].start);
    }

    fn at(d: &str, h: u16, m: u16) -> Moment {
        Moment { date: date(d), minute: h * 60 + m }
    }

    /// Starts and finishes are moments on the working times: a task starts at the start of its
    /// first working minute and finishes at the end of its last; a milestone after it sits at
    /// that end (17:00), one constrained to start on a date at the start of the day (8:00).
    #[test]
    fn starts_and_finishes_are_moments_on_the_working_times() {
        let mut p = Project::new("t", date("2026-01-05"));
        let a = p.add_task("A", 3 * D).unwrap();
        let m = p.add_task("M", 0).unwrap();
        let k = p.add_task("K", 0).unwrap();
        p.set_preds(m, parse_preds("1", D).unwrap()).unwrap();
        p.task_mut(k).unwrap().constraint = Constraint::StartNoEarlierThan(date("2026-01-12"));
        let s = schedule(&p).unwrap();
        assert_eq!((s.tasks[&a].start_at, s.tasks[&a].finish_at), (at("2026-01-05", 8, 0), at("2026-01-07", 17, 0)));
        assert_eq!((s.tasks[&m].start_at, s.tasks[&m].finish_at), (at("2026-01-07", 17, 0), at("2026-01-07", 17, 0)));
        assert_eq!((s.tasks[&k].start_at, s.tasks[&k].finish_at), (at("2026-01-12", 8, 0), at("2026-01-12", 8, 0)));
    }

    /// Part-day durations and lags count working minutes across the lunch break and the night.
    #[test]
    fn part_days_count_working_minutes() {
        let mut p = Project::new("t", date("2026-01-05"));
        let a = p.add_task("A", 240).unwrap();
        let b = p.add_task("B", D).unwrap();
        let c = p.add_task("C", 90).unwrap();
        p.set_preds(b, parse_preds("1", D).unwrap()).unwrap();
        p.set_preds(c, parse_preds("2FS+30m", D).unwrap()).unwrap();
        let s = schedule(&p).unwrap();
        // A Mon 8:00–12:00; B from 13:00, a working day, to Tue 12:00; C 30 minutes later, 13:30–15:00.
        assert_eq!(s.tasks[&a].finish_at, at("2026-01-05", 12, 0));
        assert_eq!((s.tasks[&b].start_at, s.tasks[&b].finish_at), (at("2026-01-05", 13, 0), at("2026-01-06", 12, 0)));
        assert_eq!((s.tasks[&c].start_at, s.tasks[&c].finish_at), (at("2026-01-06", 13, 30), at("2026-01-06", 15, 0)));
        assert_eq!(s.tasks[&a].slack, 0);
    }

    /// A short Friday (8:00–12:00) holds half a day of work: a 2-day task from Thursday works
    /// Thursday, Friday morning and Monday morning. Constraint dates on days off land on the
    /// working time either side: Finish No Later Than a Saturday is the end of Friday.
    #[test]
    fn working_times_set_how_much_a_day_holds() {
        let mut p = Project::new("t", date("2026-01-08"));
        p.calendar.set_weekday_times(4, vec![crate::parse_working_time("8:00-12:00").unwrap()]).unwrap();
        let a = p.add_task("A", 2 * D).unwrap();
        let b = p.add_task("B", D).unwrap();
        p.task_mut(b).unwrap().constraint = Constraint::FinishNoLaterThan(date("2026-01-10"));
        let s = schedule(&p).unwrap();
        assert_eq!((s.tasks[&a].start_at, s.tasks[&a].finish_at), (at("2026-01-08", 8, 0), at("2026-01-12", 12, 0)));
        assert_eq!(s.tasks[&b].late_finish, date("2026-01-09"));
        // B must finish by Friday 12:00, so it may start as late as Thursday 12:00: 4 hours of slack.
        assert_eq!(s.tasks[&b].slack, 240);
    }

    #[test]
    fn a_calendar_with_no_working_days_is_an_error() {
        let mut p = Project::new("t", date("2026-01-05"));
        p.add_task("A", D).unwrap();
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
                let id = p.add_task(format!("T{i}"), (i % 4) as i64 * D).unwrap();
                p.task_mut(id).unwrap().outline_level = *l;
            }
            let ids: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
            let kinds = [LinkKind::FS, LinkKind::SS, LinkKind::FF, LinkKind::SF];
            for (succ, pred, kind, lag) in links {
                if let Some(&id) = ids.get(succ) {
                    p.task_mut(id).unwrap().preds.push(crate::Link { pred: TaskId(pred), kind: kinds[usize::from(kind)], lag: lag * D });
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

    /// The last working day on or before `d`: where a finish-type date lands.
    fn workday_back(p: &Project, d: Date) -> Date {
        let mut d = d;
        for _ in 0..7 {
            if p.calendar.is_workday(d) {
                break;
            }
            d = d.add_days(-1).unwrap();
        }
        d
    }

    /// Nested summaries 40 deep, each Finish No Earlier Than a later date than the one inside it,
    /// over a subtask fixed by Must Start On and one linked after it: the searches stay within
    /// their bounds, the outermost lands on its date, and the dates come out the same twice.
    #[test]
    fn deeply_nested_finish_holds_are_bounded_and_land_on_their_dates() {
        let mut p = Project::new("t", date("2026-01-05"));
        let depth = 40u32;
        for l in 0..depth {
            let id = p.add_task(format!("S{l}"), D).unwrap();
            let t = p.task_mut(id).unwrap();
            t.outline_level = l + 1;
            t.constraint = Constraint::FinishNoEarlierThan(date("2026-03-02").add_days(i64::from(depth - l)).unwrap());
        }
        let a = p.add_task("A", 2 * D).unwrap();
        let b = p.add_task("B", 3 * D).unwrap();
        for id in [a, b] {
            p.task_mut(id).unwrap().outline_level = depth + 1;
        }
        p.task_mut(a).unwrap().constraint = Constraint::MustStartOn(date("2026-01-05"));
        p.set_preds(b, parse_preds(&a.0.to_string(), D).unwrap()).unwrap();
        let first = schedule(&p).unwrap();
        assert_eq!(first, schedule(&p).unwrap());
        let outer = p.tasks[0].id;
        let cd = date("2026-03-02").add_days(i64::from(depth)).unwrap();
        assert_eq!(p.tasks[0].constraint, Constraint::FinishNoEarlierThan(cd));
        assert_eq!(first.tasks[&outer].finish, workday_back(&p, cd));
    }

    proptest::proptest! {
        /// Finish-type holds on summaries (Finish No Earlier Than, finish-to-finish and
        /// start-to-finish links into them) over subtasks that are As Soon As Possible or Start No
        /// Earlier Than: every such summary finishes on or after its date, never panics, and gives
        /// the same dates twice.
        #[test]
        fn finish_holds_on_summaries_are_met_and_deterministic(
            levels in proptest::collection::vec(0u32..5, 1..12),
            links in proptest::collection::vec((0usize..12, 0u32..14, 0u8..4, -3i64..4), 0..14),
            constraints in proptest::collection::vec((0usize..12, 0u8..2, 0i64..40), 0..6),
            durations in proptest::collection::vec(0i64..6, 12),
        ) {
            let mut p = Project::new("t", date("2026-01-05"));
            for (i, l) in levels.iter().enumerate() {
                let id = p.add_task(format!("T{i}"), durations[i] * D).unwrap();
                p.task_mut(id).unwrap().outline_level = *l;
            }
            let ids: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
            let kinds = [LinkKind::FS, LinkKind::SS, LinkKind::FF, LinkKind::SF];
            for (succ, pred, kind, lag) in links {
                if let Some(&id) = ids.get(succ) {
                    p.task_mut(id).unwrap().preds.push(crate::Link { pred: TaskId(pred), kind: kinds[usize::from(kind)], lag: lag * D });
                }
            }
            for (row, ty, day) in constraints {
                let d = date("2026-01-01").add_days(day).unwrap();
                if let Some(&id) = ids.get(row) {
                    p.task_mut(id).unwrap().constraint =
                        if ty == 0 { Constraint::StartNoEarlierThan(d) } else { Constraint::FinishNoEarlierThan(d) };
                }
            }
            let first = schedule(&p);
            proptest::prop_assert_eq!(&first, &schedule(&p));
            let Ok(sch) = first else { return Ok(()) };
            for t in p.tasks.iter().filter(|t| p.is_summary(t.id)) {
                let finish = sch.tasks[&t.id].finish;
                if let Constraint::FinishNoEarlierThan(cd) = t.constraint
                    && cd >= p.start
                {
                    proptest::prop_assert!(finish >= workday_back(&p, cd), "{} finishes {finish:?} before {cd:?}", t.name);
                }
                for l in t.preds.iter().filter(|l| l.kind == LinkKind::FF && l.lag == 0) {
                    let Some(pd) = sch.tasks.get(&l.pred) else { continue };
                    proptest::prop_assert!(finish >= pd.finish, "{} finishes {finish:?} before its FF predecessor", t.name);
                }
            }
        }
    }
}
