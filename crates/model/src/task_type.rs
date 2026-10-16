//! Task types and effort-driven scheduling, as Microsoft Project documents them in "Change the
//! task type for more accurate scheduling"
//! (<https://support.microsoft.com/en-us/project/change-the-task-type-for-more-accurate-scheduling>)
//! and "Change the effort driven setting for task types"
//! (<https://support.microsoft.com/en-us/office/change-the-effort-driven-setting-for-task-types-18efd7c7-d146-4b06-bdbe-b6a11564bdf3>).
//!
//! Each work assignment keeps its **units**, its **work** and its **duration** (how long it works
//! from the task's start: work ÷ units); the task's duration is the longest of its assignments'.
//! Work = Duration × Units, and the task type says which of the three stays when another changes:
//!
//! | Type | Units edited | Duration edited | Work edited |
//! |---|---|---|---|
//! | Fixed Units | duration recalculated | work recalculated | duration recalculated |
//! | Fixed Work | duration recalculated | units recalculated | duration recalculated |
//! | Fixed Duration | work recalculated | work recalculated | units recalculated |
//!
//! **Effort-driven**: once a task has work, adding or removing a work resource keeps the total
//! work, shared among the work assignments in proportion to their units; a Fixed Units or Fixed
//! Work task's duration changes, a Fixed Duration task's units do. The first assignment sets the
//! work (duration × units). Not effort-driven, a new assignment adds work and a removed one takes
//! its work away. New tasks are not effort-driven unless the project's options say so; a Fixed Work
//! task always is. A summary task is never effort-driven and its assignments follow its rolled-up
//! duration, as a Fixed Duration task's do.
//!
//! Materials and costs take no part: a material's quantity follows the task's duration
//! ([`crate::assignment_work`]), a cost has none. Integer arithmetic only: units in hundredths of a
//! percent, work and durations in working minutes, rounded to the nearest (a half away from zero).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{FULL_TIME, MAX_MINUTES, MAX_UNITS, ModelError, Project, TaskId, Units, check_minutes, resource::pick};

/// Most work one assignment holds, in working minutes: the longest duration at the highest units.
pub const MAX_WORK: i64 = MAX_MINUTES * (MAX_UNITS / FULL_TIME) as i64;

/// Which of duration, units and work a task keeps when another changes, as Microsoft Project's
/// Type field. Files without one have Fixed Units, Project's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskType {
    /// The units stay: changing duration changes work, changing work changes duration.
    #[default]
    FixedUnits,
    /// The duration stays: changing units or work changes the other.
    FixedDuration,
    /// The work stays: changing duration changes units, changing units changes duration. Always
    /// effort-driven.
    FixedWork,
}

impl TaskType {
    pub const ALL: [TaskType; 3] = [TaskType::FixedUnits, TaskType::FixedDuration, TaskType::FixedWork];
    pub const NAMES: [&'static str; 3] = ["Fixed Units", "Fixed Duration", "Fixed Work"];

    pub fn name(self) -> &'static str {
        match self {
            TaskType::FixedUnits => "Fixed Units",
            TaskType::FixedDuration => "Fixed Duration",
            TaskType::FixedWork => "Fixed Work",
        }
    }

    /// From its name or the start of it, any case, with or without "Fixed": `Fixed Work`, `work`,
    /// `fixed d`, `u`.
    pub fn from_name(text: &str) -> Result<TaskType, ModelError> {
        let t = text.trim();
        let lower = t.to_lowercase();
        let rest = lower.strip_prefix("fixed").map(str::trim_start).unwrap_or(&lower);
        let bare = |k: TaskType| k.name().get(6..).unwrap_or("");
        match TaskType::ALL.into_iter().filter(|k| !rest.is_empty() && bare(*k).to_lowercase().starts_with(rest)).collect::<Vec<_>>()[..] {
            [one] => Ok(one),
            _ => pick(t, &TaskType::ALL, TaskType::name, "task type"),
        }
    }

    pub fn is_fixed_units(&self) -> bool {
        *self == TaskType::FixedUnits
    }
}

/// `amount × num ÷ den`, rounded to the nearest (a half away from zero), in i128 so nothing
/// overflows; `None` past `i64` or when `den` is 0.
fn ratio(amount: i64, num: i64, den: i64) -> Option<i64> {
    if den == 0 {
        return None;
    }
    let (n, d) = (i128::from(amount) * i128::from(num), i128::from(den));
    let q = (2 * n.abs() + d.abs()) / (2 * d.abs());
    let q = if (n < 0) != (d < 0) { -q } else { q };
    i64::try_from(q).ok()
}

/// Work of `units` over `duration` working minutes: duration × units.
pub(crate) fn work_of(duration: i64, units: u32) -> i64 {
    ratio(duration, i64::from(units), i64::from(FULL_TIME)).unwrap_or(MAX_WORK).clamp(0, MAX_WORK)
}

/// How long `work` takes at `units`: work ÷ units. Refused past [`MAX_MINUTES`]; `None` at 0%.
fn duration_of(work: i64, units: u32) -> Result<Option<i64>, ModelError> {
    match ratio(work, i64::from(FULL_TIME), i64::from(units)) {
        None => Ok(None),
        Some(d) if (0..=MAX_MINUTES).contains(&d) => Ok(Some(d)),
        Some(_) => Err(ModelError::Invalid(format!("that would make the task longer than {} days", crate::MAX_DAYS))),
    }
}

/// The units that do `work` in `duration`: work ÷ duration. Refused past [`MAX_UNITS`]; `None`
/// over no duration.
fn units_of(work: i64, duration: i64) -> Result<Option<u32>, ModelError> {
    match ratio(work, i64::from(FULL_TIME), duration) {
        None => Ok(None),
        Some(u) => u32::try_from(u)
            .ok()
            .filter(|u| *u <= MAX_UNITS)
            .map(Some)
            .ok_or_else(|| ModelError::Invalid(format!("that would need units above {}%", MAX_UNITS / crate::UNITS_PER_PERCENT))),
    }
}

/// `total` split in proportion to `weights` (equally when they are all 0), each share rounded so
/// the shares add up to `total` exactly.
fn split(total: i64, weights: &[i64]) -> Vec<i64> {
    let sum: i64 = weights.iter().map(|w| w.max(&0)).fold(0i64, |a, w| a.saturating_add(*w));
    let equal = vec![1i64; weights.len()];
    let (weights, sum) = if sum > 0 { (weights.to_vec(), sum) } else { (equal, i64::try_from(weights.len()).unwrap_or(i64::MAX)) };
    let mut cum = 0i64;
    let mut before = 0i64;
    weights
        .iter()
        .map(|w| {
            cum = cum.saturating_add((*w).max(0));
            let upto = ratio(total, cum, sum).unwrap_or(total);
            let share = upto - before;
            before = upto;
            share
        })
        .collect()
}

impl Project {
    /// Whether a task keeps its work as resources come and go: its Effort Driven flag (always for
    /// Fixed Work), never for a summary.
    pub fn is_effort_driven(&self, id: TaskId) -> bool {
        !self.is_summary(id) && self.task(id).is_ok_and(|t| t.effort_driven || t.task_type == TaskType::FixedWork)
    }

    /// The type the scheduler gives a task: its own, or Fixed Duration for a summary (its
    /// duration is rolled up, so its assignments' work follows it).
    pub fn task_type_of(&self, id: TaskId) -> TaskType {
        if self.is_summary(id) { TaskType::FixedDuration } else { self.task(id).map_or(TaskType::FixedUnits, |t| t.task_type) }
    }

    /// Positions in `assignments` of a task's work assignments.
    fn work_rows(&self, task: TaskId) -> Vec<usize> {
        self.assignments.iter().enumerate().filter(|(_, a)| a.task == task && matches!(a.units, Units::Work(_))).map(|(i, _)| i).collect()
    }

    /// A task's work: its work assignments' work, added up.
    pub fn task_work(&self, task: TaskId) -> i64 {
        self.work_rows(task).iter().filter_map(|&i| self.assignments.get(i)).fold(0i64, |a, x| a.saturating_add(x.work))
    }

    /// Sets a task's type and Effort Driven flag (either may be left as it is). A Fixed Work task
    /// is always effort-driven: making one sets the flag, and clearing it on one is refused. A
    /// summary's type is not its own to set. Nothing is recalculated, as in Project.
    pub fn set_task_type(&mut self, id: TaskId, task_type: Option<TaskType>, effort_driven: Option<bool>) -> Result<(), ModelError> {
        if self.is_summary(id) {
            return Err(ModelError::Invalid("a summary task's type and effort-driven setting follow its subtasks and can't be set".into()));
        }
        let t = self.task(id)?;
        let ty = task_type.unwrap_or(t.task_type);
        let ed = match (ty, effort_driven) {
            (TaskType::FixedWork, Some(false)) => {
                return Err(ModelError::Invalid("a Fixed Work task is always effort-driven".into()));
            }
            (TaskType::FixedWork, _) => true,
            (_, Some(e)) => e,
            (_, None) => t.effort_driven,
        };
        let t = self.task_mut(id)?;
        t.task_type = ty;
        t.effort_driven = ed;
        Ok(())
    }

    /// Sets a task's duration (working minutes, shown in `unit`): Fixed Units and Fixed Duration
    /// keep the units and recalculate the work; Fixed Work keeps the work and recalculates the
    /// units. Every work assignment then lasts the new duration. A summary's is rolled up.
    pub fn set_duration(&mut self, id: TaskId, duration: i64, unit: crate::DurationUnit) -> Result<(), ModelError> {
        if self.is_summary(id) {
            return Err(ModelError::Invalid("a summary task's duration is rolled up from its subtasks and can't be set".into()));
        }
        let duration = check_minutes(duration)?;
        let ty = self.task_type_of(id);
        let mut p = self.clone();
        for i in p.work_rows(id) {
            let Some(a) = p.assignments.get_mut(i) else { continue };
            let Units::Work(u) = a.units else { continue };
            if ty == TaskType::FixedWork {
                match units_of(a.work, duration)? {
                    Some(units) => a.units = Units::Work(units),
                    None if a.work > 0 => {
                        return Err(ModelError::Invalid("a Fixed Work task with work can't have no duration; change its work or type first".into()));
                    }
                    None => {}
                }
            } else {
                a.work = work_of(duration, u);
            }
            a.duration = duration;
        }
        let t = p.task_mut(id)?;
        t.duration = duration;
        t.duration_unit = unit;
        *self = p;
        Ok(())
    }

    /// Sets a task's work (working minutes), shared among its work assignments in proportion to
    /// their work (their units when it has none): Fixed Units and Fixed Work keep the units and
    /// recalculate the duration; Fixed Duration keeps the duration and recalculates the units.
    /// Refused on a summary (its work is rolled up) and on a task with no work resource.
    pub fn set_task_work(&mut self, id: TaskId, work: i64) -> Result<(), ModelError> {
        if self.is_summary(id) {
            return Err(ModelError::Invalid("a summary task's work is rolled up from its subtasks and can't be set".into()));
        }
        if !(0..=MAX_WORK).contains(&work) {
            return Err(ModelError::Invalid(format!("work must be between 0 and {} hours", MAX_WORK / 60)));
        }
        let rows = self.work_rows(id);
        if rows.is_empty() {
            return Err(ModelError::Invalid("assign a work resource before setting the work: work is what the task's resources do".into()));
        }
        let ty = self.task_type_of(id);
        let mut p = self.clone();
        let current: Vec<i64> = rows.iter().filter_map(|&i| p.assignments.get(i)).map(|a| a.work).collect();
        let weights = if current.iter().any(|w| *w > 0) {
            current
        } else {
            rows.iter().filter_map(|&i| p.assignments.get(i)).map(|a| if let Units::Work(u) = a.units { i64::from(u) } else { 0 }).collect()
        };
        for (&i, share) in rows.iter().zip(split(work, &weights)) {
            let Some(a) = p.assignments.get_mut(i) else { continue };
            let Units::Work(u) = a.units else { continue };
            a.work = share;
            if ty == TaskType::FixedDuration {
                match units_of(share, a.duration)? {
                    Some(units) => a.units = Units::Work(units),
                    None if share > 0 => return Err(ModelError::Invalid("a Fixed Duration task with no duration can't take work".into())),
                    None => {}
                }
            } else {
                match duration_of(share, u)? {
                    Some(d) => a.duration = d,
                    None if share > 0 => return Err(ModelError::Invalid("an assignment at 0% can't do work; give it units first".into())),
                    None => {}
                }
            }
        }
        p.settle(id);
        *self = p;
        Ok(())
    }

    /// A work assignment's new units: Fixed Units and Fixed Work keep its work and recalculate
    /// its duration (the task's is the longest); Fixed Duration keeps its duration and
    /// recalculates its work. At 0% a Fixed Units assignment does no work; a Fixed Work one can't.
    pub(crate) fn set_work_units(&mut self, i: usize, units: u32) -> Result<(), ModelError> {
        let Some(task) = self.assignments.get(i).map(|a| a.task) else { return Ok(()) };
        let ty = self.task_type_of(task);
        let Some(a) = self.assignments.get_mut(i) else { return Ok(()) };
        if ty == TaskType::FixedDuration {
            a.work = work_of(a.duration, units);
        } else {
            match duration_of(a.work, units)? {
                Some(d) => a.duration = d,
                None if ty == TaskType::FixedWork && a.work > 0 => {
                    return Err(ModelError::Invalid("a Fixed Work task keeps its work, so its units can't be 0%".into()));
                }
                None => a.work = 0,
            }
        }
        a.units = Units::Work(units);
        self.settle(task);
        Ok(())
    }

    /// Fills a new work assignment (at row `i`): it works the task's duration at its units. When
    /// the task already had work (`before`) and is effort-driven, that work is shared again among
    /// all its work assignments.
    pub(crate) fn fill_new_work(&mut self, i: usize, before: i64) -> Result<(), ModelError> {
        let Some(task) = self.assignments.get(i).map(|a| a.task) else { return Ok(()) };
        let duration = self.task(task)?.duration;
        if let Some(a) = self.assignments.get_mut(i)
            && let Units::Work(u) = a.units
        {
            a.duration = duration;
            a.work = work_of(duration, u);
        }
        if before > 0 && self.is_effort_driven(task) { self.share_work(task, before) } else { Ok(()) }
    }

    /// After a work assignment of `task` was removed, with `before` the work the task had: an
    /// effort-driven task shares it among the work assignments left; otherwise the duration is
    /// the longest left. With none left the duration stays, and Fixed Units and Fixed Duration
    /// tasks lose their work, as Project's "Remove or replace a resource assignment" says.
    pub(crate) fn after_work_removed(&mut self, task: TaskId, before: i64) -> Result<(), ModelError> {
        if before > 0 && self.is_effort_driven(task) && !self.work_rows(task).is_empty() {
            self.share_work(task, before)
        } else {
            self.settle(task);
            Ok(())
        }
    }

    /// Effort-driven: `total` work shared among a task's work assignments in proportion to their
    /// units. Fixed Units and Fixed Work: one duration for all, the total over the units added up.
    /// Fixed Duration: the duration stays and the units are recalculated. Nothing changes when the
    /// units add up to 0%.
    fn share_work(&mut self, task: TaskId, total: i64) -> Result<(), ModelError> {
        let rows = self.work_rows(task);
        let units: Vec<i64> =
            rows.iter().filter_map(|&i| self.assignments.get(i)).map(|a| if let Units::Work(u) = a.units { i64::from(u) } else { 0 }).collect();
        let sum = units.iter().sum::<i64>();
        if sum <= 0 {
            self.settle(task);
            return Ok(());
        }
        let ty = self.task_type_of(task);
        let duration = self.task(task)?.duration;
        let span = ratio(total, i64::from(FULL_TIME), sum).unwrap_or(i64::MAX);
        if ty != TaskType::FixedDuration && !(0..=MAX_MINUTES).contains(&span) {
            return Err(ModelError::Invalid(format!("that would make the task longer than {} days", crate::MAX_DAYS)));
        }
        for (&i, share) in rows.iter().zip(split(total, &units)) {
            let new_units = if ty == TaskType::FixedDuration { units_of(share, duration)? } else { None };
            let Some(a) = self.assignments.get_mut(i) else { continue };
            a.work = share;
            if ty == TaskType::FixedDuration {
                a.duration = duration;
                if let Some(u) = new_units {
                    a.units = Units::Work(u);
                }
            } else {
                a.duration = span;
            }
        }
        self.settle(task);
        Ok(())
    }

    /// A task's duration as its work assignments set it: the longest of theirs. A summary's is
    /// rolled up, and a task with no work assignment keeps its own.
    pub(crate) fn settle(&mut self, task: TaskId) {
        if self.is_summary(task) {
            return;
        }
        let longest = self.work_rows(task).iter().filter_map(|&i| self.assignments.get(i)).map(|a| a.duration).max();
        if let (Some(d), Ok(t)) = (longest, self.task_mut(task)) {
            t.duration = d;
        }
    }

    /// Settles each of these tasks ([`Project::settle`]): after assignments were dropped with a
    /// resource.
    /// One pass over the assignments and one over the tasks, however many there are.
    pub(crate) fn settle_all(&mut self, tasks: &BTreeSet<TaskId>) {
        let mut longest: std::collections::BTreeMap<TaskId, i64> = std::collections::BTreeMap::new();
        for a in self.assignments.iter().filter(|a| tasks.contains(&a.task) && matches!(a.units, Units::Work(_))) {
            let d = longest.entry(a.task).or_insert(0);
            *d = (*d).max(a.duration);
        }
        let outline = crate::Outline::of(self);
        for (row, t) in self.tasks.iter_mut().enumerate() {
            if let Some(d) = longest.get(&t.id).filter(|_| !outline.is_summary(row)) {
                t.duration = *d;
            }
        }
    }

    /// Each summary's rolled-up duration, and its work assignments' work over it: the engine
    /// stores them after every schedule, so a summary that stops being one keeps its duration.
    /// One pass over the tasks and one over the assignments.
    pub fn set_rolled_up(&mut self, rolled_up: &std::collections::BTreeMap<TaskId, i64>) {
        if rolled_up.is_empty() {
            return;
        }
        for t in &mut self.tasks {
            if let Some(d) = rolled_up.get(&t.id) {
                t.duration = (*d).clamp(0, MAX_MINUTES);
            }
        }
        for a in &mut self.assignments {
            if let (Some(d), Units::Work(u)) = (rolled_up.get(&a.task), a.units) {
                a.duration = (*d).clamp(0, MAX_MINUTES);
                a.work = work_of(a.duration, u);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_types_read_from_their_names() {
        for (text, ty) in [
            ("Fixed Units", TaskType::FixedUnits),
            ("fixed duration", TaskType::FixedDuration),
            ("FIXED WORK", TaskType::FixedWork),
            ("work", TaskType::FixedWork),
            ("u", TaskType::FixedUnits),
            ("Fixed D", TaskType::FixedDuration),
            (" dur ", TaskType::FixedDuration),
        ] {
            assert_eq!(TaskType::from_name(text).unwrap(), ty, "{text}");
        }
        for bad in ["", "fixed", "f", "x", "Fixed Cost"] {
            assert!(TaskType::from_name(bad).is_err(), "{bad}");
        }
        for ty in TaskType::ALL {
            assert_eq!(TaskType::from_name(ty.name()).unwrap(), ty);
        }
    }

    #[test]
    fn ratios_round_half_away_from_zero_and_never_overflow() {
        assert_eq!(ratio(5, 1, 2), Some(3));
        assert_eq!(ratio(-5, 1, 2), Some(-3));
        assert_eq!(ratio(1, 1, 0), None);
        assert_eq!(ratio(i64::MAX, i64::MAX, 1), None);
        assert_eq!(work_of(4800, FULL_TIME / 2), 2400);
        assert_eq!(duration_of(4800, 0).unwrap(), None);
        assert!(duration_of(MAX_WORK, 1).is_err());
        assert!(units_of(MAX_WORK, 1).is_err());
        assert_eq!(units_of(4800, 3840).unwrap(), Some(12_500));
    }

    #[test]
    fn splits_add_up_exactly() {
        assert_eq!(split(100, &[1, 1, 1]), vec![33, 34, 33]);
        assert_eq!(split(100, &[0, 0]), vec![50, 50]);
        assert_eq!(split(4800, &[10_000, 10_000]), vec![2400, 2400]);
        assert_eq!(split(7, &[]), Vec::<i64>::new());
        for (total, w) in [(1_000_003i64, vec![3, 7, 11]), (MAX_WORK, vec![1, i64::from(MAX_UNITS)])] {
            assert_eq!(split(total, &w).iter().sum::<i64>(), total);
        }
    }

    proptest::proptest! {
        #[test]
        fn task_type_names_never_panic(s in "\\PC{0,20}") {
            let _ = TaskType::from_name(&s);
        }

        #[test]
        fn splits_always_add_up(total in 0i64..MAX_WORK, w in proptest::collection::vec(0i64..2_000_000, 1..8)) {
            let s = split(total, &w);
            proptest::prop_assert_eq!(s.iter().sum::<i64>(), total);
            proptest::prop_assert!(s.iter().all(|x| *x >= 0));
        }
    }
}
