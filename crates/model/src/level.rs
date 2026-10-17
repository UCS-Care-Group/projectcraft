//! Resource levelling: Level All resolves overallocation by delaying tasks, as Microsoft
//! Project's "Level resource assignments" with its defaults
//! (<https://support.microsoft.com/en-US/project/distribute-project-work-evenly-level-resource-assignments>):
//! overallocations are looked for **day by day**, the **Standard** levelling order decides which
//! task is delayed, levelling values are **cleared before levelling**, levelling may delay the
//! project finish, and it never splits a task.
//!
//! A delay is stored on the task as its **Leveling Delay** ([`crate::Task::leveling_delay`]),
//! working minutes of the project calendar after its links and constraint, and the scheduler adds
//! it: dates still come only from [`crate::schedule`]. Clear Leveling sets every delay back to
//! nothing.
//!
//! The method, one round at a time:
//! 1. Schedule the plan and work out each work resource's usage ([`crate::usage`]).
//! 2. Take the earliest day any work resource is overallocated (more work than its Max Units
//!    allow on its calendar that day).
//! 3. For each resource overallocated that day, go through the tasks it works on that day in the
//!    Standard order, keeping each whose work still fits; the rest are delayed to the start of
//!    the next working day of the project calendar. In the Standard order a task levelling can't
//!    move comes first, then one with successors (a predecessor) before one without, less total
//!    slack before more, an earlier start before a later one (slack and start as the plan had
//!    them before levelling, so the order never changes between rounds), a higher priority before
//!    a lower one, a constrained task before an As Soon As Possible one, and finally the higher
//!    row. The
//!    first task kept is always kept, even when its own work is more than the resource allows: a
//!    delay can't fix that.
//! 4. Repeat until no resource is overallocated on a day levelling can still do something about.
//!
//! Levelling never moves a summary task, a milestone, a task of priority [`MAX_PRIORITY`], or one
//! that must start or finish on a date. Material and cost resources are never overallocated, so
//! never levelled.
//!
//! Bounded and deterministic: a round always moves a task later, gives up on a task its delays
//! don't move, or gives up on a day; and the rounds are capped ([`MAX_LEVEL_ROUNDS`], fewer on a
//! large plan), so a plan that can't be levelled stops with a report of what is still
//! overallocated, the same way on every run. Nothing depends on hash order or the clock.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::usage::calendar_of;
use crate::{Calendar, Constraint, Date, FULL_TIME, MAX_MINUTES, ModelError, Moment, Project, ResourceId, ResourceType, TaskId, schedule, usage};

/// A new task's priority, as in Microsoft Project.
pub const DEFAULT_PRIORITY: u32 = 500;
/// The highest priority: levelling never delays a task with it.
pub const MAX_PRIORITY: u32 = 1000;
/// Most rounds of levelling (each a reschedule). A large plan gets fewer: see `LEVEL_BUDGET`.
pub const MAX_LEVEL_ROUNDS: usize = 5_000;
/// Rounds × (tasks + assignments) levelling may spend in all, so a large plan's levelling takes
/// a bounded time: a 200-task plan with 300 assignments gets the full [`MAX_LEVEL_ROUNDS`], one
/// at the task cap a few.
const LEVEL_BUDGET: u64 = 5_000_000;
/// Times in a row a task's delay may grow without moving it before levelling leaves it where it
/// last moved to (a No Later Than date holding it, say).
const MAX_STRIKES: u32 = 3;
/// Most days looked ahead for the project calendar's next working day.
const MAX_LOOK_AHEAD: i64 = 3_700;

/// A priority in range: 0 to [`MAX_PRIORITY`].
pub fn check_priority(n: i64) -> Result<u32, ModelError> {
    u32::try_from(n)
        .ok()
        .filter(|p| *p <= MAX_PRIORITY)
        .ok_or_else(|| ModelError::Invalid(format!("a priority is between 0 and {MAX_PRIORITY}, not {n}")))
}

/// What Level All did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LevelReport {
    /// The tasks it delayed, top to bottom.
    pub delayed: Vec<TaskId>,
    /// The work resources still overallocated afterwards, in Resource Sheet order, each with the
    /// first day it is: delaying tasks couldn't resolve it (one assignment alone over Max Units,
    /// tasks levelling may not move), or levelling stopped first.
    pub still_over: Vec<(ResourceId, Date)>,
    /// It stopped at its round cap before every overallocation it could resolve was resolved.
    pub stopped: bool,
    /// Rounds taken, each a reschedule.
    pub rounds: usize,
}

impl LevelReport {
    /// One line for the status bar or the command line.
    pub fn text(&self, p: &Project) -> String {
        let mut out = match self.delayed.len() {
            0 if self.still_over.is_empty() => "Nothing to level: no resource is overallocated.".to_owned(),
            0 => "Levelling delayed no task.".to_owned(),
            1 => "Levelling delayed 1 task.".to_owned(),
            n => format!("Levelling delayed {n} tasks."),
        };
        if self.stopped {
            out.push_str(&format!(" It stopped after {} rounds, the most it takes on a plan this size.", self.rounds));
        }
        if !self.still_over.is_empty() {
            let names: Vec<String> =
                self.still_over.iter().take(5).map(|(id, d)| format!("{} (from {d})", p.resource(*id).map_or("?", |r| r.name.as_str()))).collect();
            let more = self.still_over.len().saturating_sub(names.len());
            let more = if more > 0 { format!(" and {more} more") } else { String::new() };
            out.push_str(&format!(" Still overallocated: {}{more}.", names.join(", ")));
        }
        out
    }
}

/// A task's place in the Standard levelling order: the smaller is kept first.
type Key = (bool, bool, i64, Moment, Reverse<u32>, bool, usize);

/// What levelling knows of a task through the rounds.
#[derive(Default, Clone, Copy)]
struct Track {
    /// The delay it last moved with.
    good: i64,
    /// Rounds in a row its delay grew without moving it.
    strikes: u32,
    /// Levelling leaves it where it is from now on.
    frozen: bool,
}

impl Project {
    /// Clear Leveling: every task's levelling delay back to none.
    pub fn clear_leveling(&mut self) {
        for t in &mut self.tasks {
            t.leveling_delay = 0;
        }
    }

    /// Level All ([`level`]).
    pub fn level(&mut self) -> Result<LevelReport, ModelError> {
        level(self)
    }
}

/// Level All: clears every levelling delay, then delays tasks until no work resource is
/// overallocated on a day levelling can do something about (see the module). An error only when
/// the plan can't be scheduled or its usage worked out; the caller keeps the project as it was.
pub fn level(p: &mut Project) -> Result<LevelReport, ModelError> {
    p.clear_leveling();
    let size = u64::try_from(p.tasks.len().saturating_add(p.assignments.len())).unwrap_or(u64::MAX).max(1);
    let rounds = usize::try_from(LEVEL_BUDGET / size).unwrap_or(MAX_LEVEL_ROUNDS).clamp(1, MAX_LEVEL_ROUNDS);
    // Each work resource's calendar and Max Units: levelling changes neither.
    let off: Vec<&str> = p.resources_on_the_project_calendar();
    let room: BTreeMap<ResourceId, (u32, Calendar)> =
        p.resources.iter().filter(|r| r.kind == ResourceType::Work).map(|r| (r.id, (r.max_units, calendar_of(p, r, &off).into_owned()))).collect();
    let has_succs: BTreeSet<TaskId> = p.tasks.iter().flat_map(|t| t.preds.iter().map(|l| l.pred)).collect();
    let row: BTreeMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let outline = crate::Outline::of(p);
    let summaries: BTreeSet<TaskId> = p.tasks.iter().enumerate().filter(|(i, _)| outline.is_summary(*i)).map(|(_, t)| t.id).collect();
    let moves = Moves::of(p);
    let mut track: BTreeMap<TaskId, Track> = BTreeMap::new();
    // Days given up on, for a resource: nothing there levelling can move.
    let mut skip: BTreeSet<(Date, ResourceId)> = BTreeSet::new();
    // The tasks delayed last round, with where they were.
    let mut pending: Vec<(TaskId, Moment, Moment)> = Vec::new();
    // The Standard order's slack and start are the plan's before levelling, so the order stays
    // the same through the rounds: a task kept before another is never delayed for it later.
    let unlevelled = schedule(p)?;
    let mut report = LevelReport::default();
    let mut done = false;
    while report.rounds < rounds {
        report.rounds += 1;
        let s = schedule(p)?;
        settle(p, &row, &s, &mut pending, &mut track)?;
        let u = usage(p, &s)?;
        let movable = |id: TaskId, track: &BTreeMap<TaskId, Track>| {
            row.get(&id).and_then(|&i| p.tasks.get(i)).is_some_and(|t| {
                t.priority < MAX_PRIORITY
                    && t.duration > 0
                    && !summaries.contains(&id)
                    && !matches!(t.constraint, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_))
                    && t.leveling_delay < MAX_MINUTES
                    && !track.get(&id).is_some_and(|k| k.frozen)
            })
        };
        // The earliest day with something to delay, and what.
        let mut delay: BTreeMap<TaskId, Date> = BTreeMap::new();
        while delay.is_empty() {
            let first = u.resources.iter().filter_map(|r| r.over.iter().find(|d| !skip.contains(&(**d, r.id))).copied()).min();
            let Some(day) = first else {
                done = true;
                break;
            };
            let over: Vec<&crate::ResourceUsage> = u.resources.iter().filter(|r| r.over.contains(&day) && !skip.contains(&(day, r.id))).collect();
            for r in over {
                let Some((max_units, cal)) = room.get(&r.id) else { continue };
                let room = i128::from(*max_units) * i128::from(cal.minutes_on(day));
                // Its tasks that day, with their work, in the Standard order.
                let mut work: BTreeMap<TaskId, i64> = BTreeMap::new();
                for (task, days) in &r.assignments {
                    if let Some(w) = days.get(&day).filter(|w| **w > 0) {
                        let e = work.entry(*task).or_insert(0);
                        *e = e.saturating_add(*w);
                    }
                }
                let mut order: Vec<(Key, TaskId, i64)> = Vec::with_capacity(work.len());
                for (id, w) in work {
                    let (Some(t), Some(d)) = (row.get(&id).and_then(|&i| p.tasks.get(i)), unlevelled.tasks.get(&id)) else { continue };
                    let key = (
                        movable(id, &track),
                        !has_succs.contains(&id),
                        d.slack,
                        d.start_at,
                        Reverse(t.priority),
                        t.constraint.is_asap(),
                        row.get(&id).copied().unwrap_or(usize::MAX),
                    );
                    order.push((key, id, w));
                }
                order.sort_unstable();
                let rows: BTreeSet<usize> = order.iter().map(|(key, _, _)| key.6).collect();
                let (mut kept, mut any) = (0i128, false);
                let mut moved = false;
                for (key, id, w) in order {
                    if delay.contains_key(&id) {
                        // Already being delayed for another resource: its work goes from this day.
                        moved = true;
                        continue;
                    }
                    let fits = (kept + i128::from(w)) * i128::from(FULL_TIME) <= room;
                    // Delaying a task that the others that day follow would take them along: the
                    // overallocation would follow it for ever.
                    if fits || !any || !key.0 || moves.reaches(key.6, &rows) {
                        kept += i128::from(w);
                        any = true;
                    } else {
                        delay.insert(id, day);
                        moved = true;
                    }
                }
                if !moved {
                    skip.insert((day, r.id));
                }
            }
        }
        if done {
            break;
        }
        for (id, day) in delay {
            let Some(d) = s.tasks.get(&id) else { continue };
            let k = track.entry(id).or_default();
            match to_next_day(&p.calendar, d.start_at, day) {
                Some(more) => {
                    let t = task_mut(p, &row, id)?;
                    t.leveling_delay = t.leveling_delay.saturating_add(more).min(MAX_MINUTES);
                    pending.push((id, d.start_at, d.finish_at));
                }
                None => k.frozen = true,
            }
        }
    }
    report.stopped = !done;
    // Delays that didn't move their task: back to where they last moved it (the same dates).
    if !pending.is_empty() {
        let s = schedule(p)?;
        settle(p, &row, &s, &mut pending, &mut track)?;
    }
    for (id, k) in &track {
        let t = task_mut(p, &row, *id)?;
        if t.leveling_delay != k.good && (k.strikes > 0 || k.frozen) {
            t.leveling_delay = k.good;
        }
    }
    let s = schedule(p)?;
    let u = usage(p, &s)?;
    report.still_over = u.resources.iter().filter_map(|r| r.over.first().map(|d| (r.id, *d))).collect();
    report.delayed = p.tasks.iter().filter(|t| t.leveling_delay > 0).map(|t| t.id).collect();
    Ok(report)
}

/// A task by its row, found through `row` rather than a search of every task.
fn task_mut<'a>(p: &'a mut Project, row: &BTreeMap<TaskId, usize>, id: TaskId) -> Result<&'a mut crate::Task, ModelError> {
    row.get(&id).and_then(|&i| p.tasks.get_mut(i)).ok_or(ModelError::NoTask(id))
}

/// Most tasks looked through to see whether delaying one moves another; past it, it is taken to.
const MAX_REACH: usize = 20_000;

/// What delaying a task moves along with it: its successors by link (a summary successor's
/// subtasks too), and the successors of the summaries it is in. Worked out as needed, so a
/// hostile outline costs no more than the tasks looked through.
struct Moves {
    /// By row: its link successors.
    succ: Vec<Vec<usize>>,
    /// By row: its summary.
    parent: Vec<Option<usize>>,
    /// By row: the end of its subtree (one past its last subtask; itself plus one when none).
    end: Vec<usize>,
}

impl Moves {
    fn of(p: &Project) -> Moves {
        let outline = crate::Outline::of(p);
        let n = p.tasks.len();
        let index: BTreeMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, t) in p.tasks.iter().enumerate() {
            for l in &t.preds {
                if let Some(x) = index.get(&l.pred).and_then(|&j| succ.get_mut(j)) {
                    x.push(i);
                }
            }
        }
        let mut parent: Vec<Option<usize>> = vec![None; n];
        for (s, kids) in outline.children().iter().enumerate() {
            for &k in kids {
                if let Some(x) = parent.get_mut(k) {
                    *x = Some(s);
                }
            }
        }
        let end = (0..n).map(|i| outline.subtree_end(i).max(i + 1)).collect();
        Moves { succ, parent, end }
    }

    /// Whether delaying row `from` moves any of `rows` (other than itself). Taken to when the
    /// search runs past [`MAX_REACH`] tasks.
    fn reaches(&self, from: usize, rows: &BTreeSet<usize>) -> bool {
        let mut seen = BTreeSet::from([from]);
        let mut stack = vec![from];
        while let Some(i) = stack.pop() {
            // Its successors, its summaries' successors, and its own subtasks if a summary.
            let mut next: Vec<usize> = self.succ.get(i).cloned().unwrap_or_default();
            let mut up = self.parent.get(i).copied().flatten();
            for _ in 0..crate::MAX_OUTLINE_LEVEL {
                let Some(a) = up else { break };
                next.extend(self.succ.get(a).into_iter().flatten().copied());
                up = self.parent.get(a).copied().flatten();
            }
            next.extend(i + 1..self.end.get(i).copied().unwrap_or(i + 1));
            for j in next {
                if j != from && rows.contains(&j) {
                    return true;
                }
                if seen.insert(j) {
                    if seen.len() > MAX_REACH {
                        return true;
                    }
                    stack.push(j);
                }
            }
        }
        false
    }
}

/// After a round: each task delayed last round that moved keeps its delay; one that didn't gets
/// a strike, and after [`MAX_STRIKES`] goes back to the delay it last moved with and is left
/// there. A delay that doesn't move a task changes no date, so undoing it needs no reschedule.
fn settle(
    p: &mut Project,
    row: &BTreeMap<TaskId, usize>,
    s: &crate::Schedule,
    pending: &mut Vec<(TaskId, Moment, Moment)>,
    track: &mut BTreeMap<TaskId, Track>,
) -> Result<(), ModelError> {
    for (id, start, finish) in pending.drain(..) {
        let moved = s.tasks.get(&id).is_some_and(|d| d.start_at > start || d.finish_at > finish);
        let t = task_mut(p, row, id)?;
        let k = track.entry(id).or_default();
        if moved {
            k.good = t.leveling_delay;
            k.strikes = 0;
        } else {
            k.strikes += 1;
            if k.strikes >= MAX_STRIKES {
                t.leveling_delay = k.good;
                k.frozen = true;
            }
        }
    }
    Ok(())
}

/// Working minutes of `cal` from `start` to the start of its first working day after `day`:
/// how much more delay puts a task starting at `start` past `day`. `None` when there is no
/// working day soon after (a damaged calendar).
fn to_next_day(cal: &Calendar, start: Moment, day: Date) -> Option<i64> {
    let mut next = day.add_days(1)?;
    let mut looked = 0;
    while cal.minutes_on(next) == 0 {
        looked += 1;
        if looked > MAX_LOOK_AHEAD {
            return None;
        }
        next = next.add_days(1)?;
    }
    let before: i64 = cal.times_on(start.date).iter().map(|t| i64::from(t.to.min(start.minute).saturating_sub(t.from))).filter(|m| *m > 0).sum();
    let more = cal.minutes_between(start.date, next).saturating_sub(before);
    (more > 0).then_some(more)
}

/// Worked examples of Level All with Microsoft Project's default settings, as its page
/// "Level resource assignments" describes them
/// (<https://support.microsoft.com/en-US/project/distribute-project-work-evenly-level-resource-assignments>):
/// tasks are delayed, not split; overallocation is looked for day by day; the Standard order looks
/// at predecessors, slack, dates, priority and constraints; a lower priority is delayed first and
/// priority 1000 is never levelled; levelling values are cleared before levelling.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DurationUnit, Link, LinkKind, Units};

    const D: i64 = 480;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    /// A plan from Monday 5 January 2026 with one work resource, Ann, at 100%.
    fn plan() -> (Project, ResourceId) {
        let mut p = Project::new("t", d("2026-01-05"));
        let ann = p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
        (p, ann)
    }

    fn task(p: &mut Project, name: &str, days: i64, who: &[(ResourceId, u32)]) -> TaskId {
        let id = p.add_task(name, days * D).unwrap();
        for (r, percent) in who {
            p.assign(id, *r, Some(Units::Work(percent * FULL_TIME / 100))).unwrap();
        }
        id
    }

    fn link(p: &mut Project, succ: TaskId, pred: TaskId) {
        let mut preds = p.task(succ).unwrap().preds.clone();
        preds.push(Link { pred, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days });
        p.set_preds(succ, preds).unwrap();
    }

    /// A task's start and finish dates, as text.
    fn dates(p: &Project, id: TaskId) -> (String, String) {
        let s = schedule(p).unwrap();
        let t = s.tasks[&id];
        (t.start.to_string(), t.finish.to_string())
    }

    fn over(p: &Project) -> bool {
        let s = schedule(p).unwrap();
        usage(p, &s).unwrap().resources.iter().any(|r| r.is_overallocated())
    }

    #[test]
    fn the_lower_row_is_delayed_when_all_else_ties_and_clear_leveling_puts_it_back() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 2, &[(ann, 100)]);
        let before = p.clone();
        assert!(over(&p));
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![b]);
        assert!(r.still_over.is_empty() && !r.stopped);
        assert_eq!(dates(&p, a), ("2026-01-05".into(), "2026-01-06".into()));
        // B waits for A: two days of delay, Wednesday and Thursday.
        assert_eq!(dates(&p, b), ("2026-01-07".into(), "2026-01-08".into()));
        assert_eq!(p.task(b).unwrap().leveling_delay, 2 * D);
        assert!(!over(&p));
        assert_eq!(schedule(&p).unwrap().tasks[&b].driver, crate::Driver::LevelingDelay);
        // Levelling again gives the same: it clears first.
        let again = p.clone();
        assert_eq!(level(&mut p).unwrap(), r);
        assert_eq!(p, again);
        p.clear_leveling();
        assert_eq!(p, before);
    }

    #[test]
    fn less_slack_is_kept_before_an_earlier_row() {
        let (mut p, ann) = plan();
        // A has two days of slack; B is the critical path.
        let a = task(&mut p, "A", 1, &[(ann, 100)]);
        let b = task(&mut p, "B", 3, &[(ann, 100)]);
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![a]);
        assert_eq!(dates(&p, b), ("2026-01-05".into(), "2026-01-07".into()));
        assert_eq!(dates(&p, a), ("2026-01-08".into(), "2026-01-08".into()));
    }

    #[test]
    fn a_lower_priority_is_delayed_first_and_1000_never() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 2, &[(ann, 100)]);
        p.task_mut(a).unwrap().priority = 400;
        assert_eq!(level(&mut p).unwrap().delayed, vec![a]);
        assert_eq!(dates(&p, a), ("2026-01-07".into(), "2026-01-08".into()));
        // 1000 on the lower priority: it is never delayed, so the other one is.
        p.task_mut(a).unwrap().priority = MAX_PRIORITY;
        assert_eq!(level(&mut p).unwrap().delayed, vec![b]);
        // Both at 1000: nothing moves, and the report says Ann is still overallocated.
        p.task_mut(b).unwrap().priority = MAX_PRIORITY;
        let r = level(&mut p).unwrap();
        assert!(r.delayed.is_empty());
        assert_eq!(r.still_over, vec![(ann, d("2026-01-05"))]);
        assert_eq!(r.text(&p), "Levelling delayed no task. Still overallocated: Ann (from 2026-01-05).");
    }

    #[test]
    fn a_task_with_successors_is_kept_and_its_successors_move_with_a_delayed_one() {
        let (mut p, ann) = plan();
        let bob = p.add_resource(Some("Bob"), ResourceType::Work, None).unwrap();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 2, &[(ann, 100)]);
        let c = task(&mut p, "C", 1, &[(bob, 100)]);
        link(&mut p, c, a);
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![b]);
        // When A is the one delayed (B can't move), C goes with it.
        p.task_mut(b).unwrap().priority = MAX_PRIORITY;
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![a]);
        assert_eq!(dates(&p, a), ("2026-01-07".into(), "2026-01-08".into()));
        assert_eq!(dates(&p, c), ("2026-01-09".into(), "2026-01-09".into()));
    }

    #[test]
    fn work_within_max_units_is_left_alone() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 2, &[(ann, 50)]);
        task(&mut p, "B", 2, &[(ann, 50)]);
        let before = p.clone();
        let r = level(&mut p).unwrap();
        assert_eq!(r, LevelReport { rounds: 1, ..LevelReport::default() });
        assert_eq!(r.text(&p), "Nothing to level: no resource is overallocated.");
        assert_eq!(p, before);
        // At 50% Max Units, two half-time tasks are one too many.
        p.resource_mut(ann).unwrap().max_units = FULL_TIME / 2;
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed.len(), 1);
        assert!(!over(&p));
        assert_eq!(dates(&p, a), ("2026-01-05".into(), "2026-01-06".into()));
    }

    #[test]
    fn one_assignment_over_max_units_can_not_be_levelled_and_says_so() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 3, &[(ann, 200)]);
        let r = level(&mut p).unwrap();
        assert!(r.delayed.is_empty() && !r.stopped);
        assert_eq!(r.still_over, vec![(ann, d("2026-01-05"))]);
        assert_eq!(dates(&p, a).0, "2026-01-05");
        // With a second task, that one waits until the first is done.
        let b = task(&mut p, "B", 1, &[(ann, 100)]);
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![b]);
        assert_eq!(dates(&p, b).0, "2026-01-08");
    }

    #[test]
    fn must_start_on_is_never_moved() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 2, &[(ann, 100)]);
        p.task_mut(a).unwrap().constraint = Constraint::MustStartOn(d("2026-01-05"));
        p.task_mut(b).unwrap().priority = 900;
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![b], "the Must Start On task stays, whatever the priorities");
        assert_eq!(dates(&p, a).0, "2026-01-05");
    }

    #[test]
    fn a_no_later_than_date_stops_a_delay_and_levelling_gives_up_on_it() {
        let (mut p, ann) = plan();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 2, &[(ann, 100)]);
        p.task_mut(a).unwrap().priority = MAX_PRIORITY;
        p.task_mut(b).unwrap().constraint = Constraint::StartNoLaterThan(d("2026-01-05"));
        let r = level(&mut p).unwrap();
        assert!(r.delayed.is_empty(), "a delay that can't move the task is taken back");
        assert_eq!(r.still_over, vec![(ann, d("2026-01-05"))]);
        assert!(!r.stopped);
    }

    #[test]
    fn a_resource_on_leave_is_levelled_on_its_own_calendar() {
        let (mut p, ann) = plan();
        // Ann is off on Tuesday 6 January: A works Monday and Wednesday.
        p.set_resource_exception(ann, d("2026-01-06"), vec![]).unwrap();
        let a = task(&mut p, "A", 2, &[(ann, 100)]);
        let b = task(&mut p, "B", 1, &[(ann, 100)]);
        p.task_mut(a).unwrap().priority = 900;
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![b]);
        assert!(!over(&p));
        assert_eq!(dates(&p, a), ("2026-01-05".into(), "2026-01-07".into()));
        assert_eq!(dates(&p, b), ("2026-01-08".into(), "2026-01-08".into()));
    }

    #[test]
    fn a_part_day_start_is_delayed_to_the_next_morning() {
        let (mut p, ann) = plan();
        // A finishes at 10:00 on Monday; B and C follow it, both Ann's: twelve hours on Monday.
        let a = p.add_task("A", 120).unwrap();
        let b = task(&mut p, "B", 1, &[(ann, 100)]);
        let c = task(&mut p, "C", 1, &[(ann, 100)]);
        link(&mut p, b, a);
        link(&mut p, c, a);
        let r = level(&mut p).unwrap();
        assert_eq!(r.delayed, vec![c]);
        let s = schedule(&p).unwrap();
        // Day by day: B works Monday and two hours of Tuesday, so C waits for Wednesday.
        assert_eq!(s.tasks[&c].start_at, Moment { date: d("2026-01-07"), minute: 8 * 60 });
        assert!(!over(&p));
    }

    proptest::proptest! {
        /// Plans with links, priorities, constraints, leave and overlapping assignments: levelling
        /// never panics or spins, levelling twice is the same, and Clear Leveling restores the plan.
        #[test]
        fn levelling_is_bounded_deterministic_and_clears(
            durations in proptest::collection::vec(0i64..2400, 1..10),
            preds in proptest::collection::vec((0usize..10, 0u32..11), 0..8),
            priorities in proptest::collection::vec((0usize..10, 0u32..1001), 0..6),
            constrained in proptest::collection::vec((0usize..10, 0usize..8, 0i64..20), 0..4),
            leave in proptest::collection::vec((0usize..3, 0i64..30), 0..6),
            max in proptest::collection::vec(1u32..30_000, 3),
            assigned in proptest::collection::vec((0usize..10, 0usize..3, 1u32..30_000), 0..14),
        ) {
            let mut p = Project::new("t", d("2026-01-05"));
            let ids: Vec<TaskId> = durations.iter().enumerate().map(|(i, m)| p.add_task(format!("T{i}"), *m).unwrap()).collect();
            for (succ, pred) in preds {
                if let Some(&id) = ids.get(succ) {
                    let _ = p.set_preds(id, vec![Link { pred: TaskId(pred), kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days }]);
                }
            }
            for (row, pr) in priorities {
                p.tasks[row % ids.len()].priority = pr;
            }
            for (row, code, day) in constrained {
                let c = Constraint::from_code(Constraint::CODES[code], d("2026-01-05").add_days(day)).unwrap();
                p.tasks[row % ids.len()].constraint = c;
            }
            let people: Vec<ResourceId> = ["Ann", "Bob", "Cat"].iter().map(|n| p.add_resource(Some(n), ResourceType::Work, None).unwrap()).collect();
            for (r, m) in people.iter().zip(&max) {
                p.resource_mut(*r).unwrap().max_units = *m;
            }
            for (r, day) in leave {
                let _ = p.set_resource_exception(people[r], d("2026-01-05").add_days(day).unwrap(), vec![]);
            }
            for (row, r, units) in assigned {
                if let Some(&id) = ids.get(row) {
                    let _ = p.assign(id, people[r], Some(Units::Work(units)));
                }
            }
            let before = p.clone();
            if schedule(&p).is_err() {
                return Ok(());
            }
            let r = level(&mut p).unwrap();
            let once = p.clone();
            proptest::prop_assert_eq!(&level(&mut p).unwrap(), &r);
            proptest::prop_assert_eq!(&p, &once);
            proptest::prop_assert!(r.rounds <= MAX_LEVEL_ROUNDS);
            // Every overallocation it leaves is reported.
            let s = schedule(&p).unwrap();
            let left: Vec<(ResourceId, Date)> = usage(&p, &s).unwrap().resources.iter().filter_map(|r| r.over.first().map(|d| (r.id, *d))).collect();
            proptest::prop_assert_eq!(&left, &r.still_over);
            for t in &p.tasks {
                proptest::prop_assert!((0..=MAX_MINUTES).contains(&t.leveling_delay));
                if t.priority == MAX_PRIORITY || p.is_summary(t.id) || t.duration == 0 {
                    proptest::prop_assert_eq!(t.leveling_delay, 0);
                }
            }
            p.clear_leveling();
            proptest::prop_assert_eq!(p, before);
        }
    }
}
