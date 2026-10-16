//! Inserting, moving and deleting rows. A summary task moves or deletes with its whole subtree, as
//! in Microsoft Project. Unique IDs never change and are never reused, so links (stored by Unique
//! ID) survive any reorder; only the ID column, the row number, renumbers.
//!
//! Each edit is all or nothing: a refused one leaves the project exactly as it was.

use std::collections::BTreeSet;

use crate::{
    CIRCULAR_OUTLINE, Constraint, DurationUnit, MAX_OUTLINE_LEVEL, MAX_TASKS, ModelError, Outline, Project, Task, TaskId, TaskType, check_minutes,
    schedule,
};

impl Project {
    /// Inserts a task above `before` at that row's outline level, or at the end at the last row's
    /// level, and returns its new Unique ID.
    pub fn insert_task(&mut self, before: Option<TaskId>, name: impl Into<String>, duration: i64) -> Result<TaskId, ModelError> {
        let outline = Outline::of(self);
        let (row, level) = match before {
            Some(b) => {
                let row = self.row_index(b)?;
                (row, outline.level(row))
            }
            None => (self.tasks.len(), outline.levels.last().copied().unwrap_or(1)),
        };
        let task = self.new_task(name.into(), duration, level)?;
        let id = task.id;
        self.tasks.insert(row, task);
        Ok(id)
    }

    /// Moves the tasks, each with its subtasks, above `to` (to the end if `None`), keeping their
    /// order. Each moved task takes the outline level of the row it goes above (the top level at
    /// the end) and its subtasks keep their depth under it, so no row that stays put becomes a
    /// summary. All or nothing.
    pub fn move_tasks(&mut self, ids: &[TaskId], to: Option<TaskId>) -> Result<(), ModelError> {
        let outline = Outline::of(self);
        let moving = self.subtree_rows(&outline, ids)?;
        if !moving.contains(&true) {
            return Ok(());
        }
        let target = match to {
            Some(t) => {
                let row = self.row_index(t)?;
                if moving.get(row).copied().unwrap_or(false) {
                    let name = self.tasks.get(row).map_or_else(String::new, |t| t.name.clone());
                    return Err(ModelError::Invalid(format!("a task can't move above itself or one of its own subtasks ({name})")));
                }
                Some((t, outline.level(row)))
            }
            None => None,
        };
        let old = self.tasks.clone();
        let goal = i64::from(target.map_or(1, |(_, l)| l));
        let (mut block, mut rest): (Vec<Task>, Vec<Task>) = (Vec::new(), Vec::new());
        // Each moved subtree's top row goes to `goal`; its subtasks keep their depth under it.
        let (mut subtree_end, mut shift) = (0, 0i64);
        for (row, ((mut t, &level), &m)) in std::mem::take(&mut self.tasks).into_iter().zip(&outline.levels).zip(&moving).enumerate() {
            if !m {
                t.outline_level = level;
                rest.push(t);
                continue;
            }
            if row >= subtree_end {
                subtree_end = outline.subtree_end(row);
                shift = goal - i64::from(level);
            }
            let shifted = i64::from(level).saturating_add(shift).clamp(1, i64::from(MAX_OUTLINE_LEVEL));
            t.outline_level = u32::try_from(shifted).unwrap_or(1);
            block.push(t);
        }
        let at = target.and_then(|(t, _)| rest.iter().position(|r| r.id == t)).unwrap_or(rest.len());
        rest.splice(at..at, block);
        self.tasks = rest;
        self.normalize_outline();
        // Moved rows can land under a summary they're linked with: a loop through the outline.
        if schedule::order(self).is_err() {
            self.tasks = old;
            return Err(ModelError::Invalid(CIRCULAR_OUTLINE.into()));
        }
        Ok(())
    }

    /// Deletes the tasks, each with its subtasks, and every link to them. All or nothing. The rows
    /// left keep their levels and their summaries: a deleted row's subtasks go with it, so no row
    /// below it gains a new summary (and with it a loop through the outline).
    pub fn delete_tasks(&mut self, ids: &[TaskId]) -> Result<(), ModelError> {
        let outline = Outline::of(self);
        let doomed = self.subtree_rows(&outline, ids)?;
        if !doomed.contains(&true) {
            return Ok(());
        }
        let gone: BTreeSet<TaskId> = self.tasks.iter().zip(&doomed).filter(|(_, d)| **d).map(|(t, _)| t.id).collect();
        self.tasks.retain(|t| !gone.contains(&t.id));
        self.assignments.retain(|a| !gone.contains(&a.task));
        for t in &mut self.tasks {
            t.preds.retain(|l| !gone.contains(&l.pred));
        }
        self.normalize_outline();
        Ok(())
    }

    /// A new task with the next Unique ID. Unique IDs only count up, so a deleted one never returns.
    pub(crate) fn new_task(&mut self, name: String, duration: i64, outline_level: u32) -> Result<Task, ModelError> {
        if self.tasks.len() >= MAX_TASKS {
            return Err(ModelError::Invalid(format!("a project holds at most {MAX_TASKS} tasks")));
        }
        let duration = check_minutes(duration)?;
        let id = TaskId(self.next_id.max(1));
        self.next_id = id.0.checked_add(1).ok_or_else(|| ModelError::Invalid("out of task ids".into()))?;
        Ok(Task {
            id,
            name,
            duration,
            duration_unit: DurationUnit::Days,
            preds: Vec::new(),
            constraint: Constraint::Asap,
            outline_level,
            deadline: None,
            fixed_cost: 0,
            task_type: self.options.task_type,
            effort_driven: self.options.effort_driven || self.options.task_type == TaskType::FixedWork,
        })
    }

    /// The 0-based row of a task.
    fn row_index(&self, id: TaskId) -> Result<usize, ModelError> {
        self.tasks.iter().position(|t| t.id == id).ok_or(ModelError::NoTask(id))
    }

    /// Marks the rows of each named task and its subtasks. Any unknown task refuses the lot.
    fn subtree_rows(&self, outline: &Outline, ids: &[TaskId]) -> Result<Vec<bool>, ModelError> {
        let mut marked = vec![false; self.tasks.len()];
        for &id in ids.iter().take(MAX_TASKS) {
            let row = self.row_index(id)?;
            for m in marked.iter_mut().take(outline.subtree_end(row)).skip(row) {
                *m = true;
            }
        }
        Ok(marked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Date, parse_preds};

    fn project(levels: &[u32]) -> (Project, Vec<TaskId>) {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let ids = levels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let id = p.add_task(format!("T{}", i + 1), 1).unwrap();
                p.task_mut(id).unwrap().outline_level = *l;
                id
            })
            .collect();
        (p, ids)
    }

    fn rows(p: &Project) -> Vec<(u32, u32)> {
        p.tasks.iter().map(|t| (t.id.0, t.outline_level)).collect()
    }

    #[test]
    fn a_move_that_links_a_summary_to_its_own_subtask_is_refused() {
        // T3 follows T1; moving it above T2 would make it T1's subtask.
        let (mut p, ids) = project(&[1, 2, 1]);
        p.set_preds(ids[2], parse_preds("1", &crate::Options::default()).unwrap()).unwrap();
        let before = p.clone();
        assert_eq!(p.move_tasks(&[ids[2]], Some(ids[1])).unwrap_err().to_string(), CIRCULAR_OUTLINE);
        assert_eq!(p, before);
    }

    #[test]
    fn tasks_picked_at_different_levels_each_land_at_the_target_level() {
        // T1 (top level) and T4 (a subtask of T3), moved to the end together: both land at the
        // top level, so T1 doesn't become T4's summary.
        let (mut p, ids) = project(&[1, 1, 1, 2]);
        p.move_tasks(&[ids[0], ids[3]], None).unwrap();
        assert_eq!(rows(&p), [(2, 1), (3, 1), (1, 1), (4, 1)]);
        assert!(!p.is_summary(ids[0]));
    }

    #[test]
    fn insert_and_move_keep_the_outline_rule() {
        let (mut p, ids) = project(&[1, 2, 3, 1]);
        let n = p.insert_task(Some(ids[2]), "x", 1).unwrap();
        assert_eq!(rows(&p), [(1, 1), (2, 2), (5, 3), (3, 3), (4, 1)]);
        // A deep subtree moved to the top keeps its shape.
        p.move_tasks(&[ids[1]], Some(ids[0])).unwrap();
        assert_eq!(rows(&p), [(2, 1), (5, 2), (3, 2), (1, 1), (4, 1)]);
        p.move_tasks(&[n], None).unwrap();
        assert_eq!(rows(&p), [(2, 1), (3, 2), (1, 1), (4, 1), (5, 1)]);
        assert!(p.insert_task(Some(TaskId(42)), "y", 1).is_err());
        assert!(p.insert_task(None, "y", -1).is_err());
        assert_eq!(p.next_id, 6, "a refused insert spends no Unique ID");
    }

    proptest::proptest! {
        #[test]
        fn row_edits_never_panic_and_keep_the_rule(
            stored in proptest::collection::vec(0u32..5, 0..10),
            edits in proptest::collection::vec((0u8..3, proptest::collection::vec(0u32..12, 0..3), proptest::option::of(0u32..12)), 0..8),
        ) {
            let (mut p, _) = project(&stored);
            p.normalize_outline();
            for (kind, ids, to) in edits {
                let ids: Vec<TaskId> = ids.into_iter().map(TaskId).collect();
                let to = to.map(TaskId);
                let before = p.clone();
                let r = match kind {
                    0 => p.insert_task(to, "n", 1).map(|_| ()),
                    1 => p.move_tasks(&ids, to),
                    _ => p.delete_tasks(&ids),
                };
                if r.is_err() {
                    proptest::prop_assert_eq!(&p, &before);
                }
                // No row becomes a summary it wasn't: moved tasks land at the level of the row below them.
                let summaries = |p: &Project| -> BTreeSet<TaskId> {
                    let o = Outline::of(p);
                    p.tasks.iter().enumerate().filter(|(row, _)| o.is_summary(*row)).map(|(_, t)| t.id).collect()
                };
                proptest::prop_assert!(summaries(&p).is_subset(&summaries(&before)));
                let now: Vec<u32> = p.tasks.iter().map(|t| t.outline_level).collect();
                proptest::prop_assert_eq!(&Outline::of(&p).levels, &now);
                let mut seen: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
                seen.sort();
                seen.dedup();
                proptest::prop_assert_eq!(seen.len(), p.tasks.len());
            }
        }
    }
}
