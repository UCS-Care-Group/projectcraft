//! The outline: each task's level, and the summary tasks it makes. A **summary task** is a row
//! followed by a deeper row; its subtasks are the rows below it at deeper levels, up to the next
//! row at its own level or shallower. Worked out from row order, never stored.
//!
//! Outline rule: the first row is level 1, and no row is more than one level deeper than the row
//! above it. Indenting or outdenting a row moves its whole subtree with it, as in Microsoft Project.

use crate::{ModelError, Project, TaskId, schedule};

/// Deepest outline level. ponytail: a sanity bound, far beyond any real plan.
pub const MAX_OUTLINE_LEVEL: u32 = 100;

/// Message for an outline change that would make a summary and one of its own subtasks linked.
pub const CIRCULAR_OUTLINE: &str = "This outline change would create a circular relationship.";

/// The outline worked out from row order, with each row's level kept to the outline rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    /// Level of each row: 1 is the top.
    pub levels: Vec<u32>,
}

impl Outline {
    /// The outline of a project. Stored levels that break the rule (only a damaged or hand-edited
    /// file could hold them) are read as the nearest level the rule allows.
    pub fn of(p: &Project) -> Outline {
        Outline { levels: legal_levels(p.tasks.iter().map(|t| t.outline_level)) }
    }

    pub fn level(&self, row: usize) -> u32 {
        self.levels.get(row).copied().unwrap_or(1)
    }

    /// Whether the row (0-based) is a summary task: the row after it is deeper.
    pub fn is_summary(&self, row: usize) -> bool {
        match (self.levels.get(row), row.checked_add(1).and_then(|r| self.levels.get(r))) {
            (Some(me), Some(next)) => next > me,
            _ => false,
        }
    }

    /// One past the last row of the row's subtree (the row itself and its subtasks).
    pub fn subtree_end(&self, row: usize) -> usize {
        let Some(&me) = self.levels.get(row) else { return self.levels.len() };
        let mut end = row + 1;
        while self.levels.get(end).is_some_and(|l| *l > me) {
            end += 1;
        }
        end
    }

    /// Each row's direct subtasks (empty for a row that isn't a summary).
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut out = vec![Vec::new(); self.levels.len()];
        // The open summaries above the current row, innermost last.
        let mut open: Vec<usize> = Vec::new();
        for (row, level) in self.levels.iter().enumerate() {
            while open.last().is_some_and(|&s| self.level(s) >= *level) {
                open.pop();
            }
            if let Some(kids) = open.last().and_then(|&s| out.get_mut(s)) {
                kids.push(row);
            }
            open.push(row);
        }
        out
    }
}

/// Levels kept to the outline rule: at least 1, at most one deeper than the row above, at most
/// [`MAX_OUTLINE_LEVEL`].
fn legal_levels(stored: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut prev = 0u32;
    stored
        .map(|l| {
            prev = l.clamp(1, prev.saturating_add(1).min(MAX_OUTLINE_LEVEL));
            prev
        })
        .collect()
}

/// Why a task with links or a constraint can't become a summary yet.
fn summary_hold_not_yet(name: &str) -> ModelError {
    ModelError::Invalid(format!("{name} has predecessors or a constraint, which summary tasks don't support yet; remove them first (coming with #8)"))
}

impl Project {
    /// Rewrites every stored level to what [`Outline::of`] reads, so the file holds a legal outline.
    pub fn normalize_outline(&mut self) {
        let levels = Outline::of(self).levels;
        for (t, l) in self.tasks.iter_mut().zip(levels) {
            t.outline_level = l;
        }
    }

    /// Whether the task is a summary task now.
    pub fn is_summary(&self, id: TaskId) -> bool {
        self.tasks.iter().position(|t| t.id == id).is_some_and(|row| Outline::of(self).is_summary(row))
    }

    /// Indents each task, with its subtasks, one level. All or nothing.
    pub fn indent(&mut self, ids: &[TaskId]) -> Result<(), ModelError> {
        self.shift_outline(ids, true)
    }

    /// Outdents each task, with its subtasks, one level. All or nothing.
    pub fn outdent(&mut self, ids: &[TaskId]) -> Result<(), ModelError> {
        self.shift_outline(ids, false)
    }

    fn shift_outline(&mut self, ids: &[TaskId], indent: bool) -> Result<(), ModelError> {
        let before = Outline::of(self);
        let mut moved = vec![false; self.tasks.len()];
        let mut picked = Vec::new();
        for &id in ids.iter().take(crate::MAX_TASKS) {
            let row = self.tasks.iter().position(|t| t.id == id).ok_or(ModelError::NoTask(id))?;
            for m in moved.iter_mut().take(before.subtree_end(row)).skip(row) {
                *m = true;
            }
            picked.push(row);
        }
        let levels: Vec<u32> = before
            .levels
            .iter()
            .zip(&moved)
            .map(|(&l, &m)| match (m, indent) {
                (false, _) => l,
                (true, true) => l.saturating_add(1),
                (true, false) => l.saturating_sub(1),
            })
            .collect();
        // The outline rule, row by row; name the first task the change breaks it on.
        let mut prev = 0u32;
        for (row, &l) in levels.iter().enumerate() {
            let name = || self.tasks.get(row).map_or_else(String::new, |t| t.name.clone());
            if l < 1 {
                return Err(ModelError::Invalid(format!("{} is already at the top level", name())));
            }
            if l > prev.saturating_add(1) || l > MAX_OUTLINE_LEVEL {
                let why =
                    if row == 0 { "the first row is always at the top level" } else { "a task can be at most one level below the row above it" };
                return Err(ModelError::Invalid(format!("{} can't be indented: {why}", name())));
            }
            prev = l;
        }
        let after = Outline { levels };
        // ponytail: a summary's own links and constraint (the hold on its subtasks) come with #8.
        for (row, t) in self.tasks.iter().enumerate() {
            if after.is_summary(row) && !before.is_summary(row) && (!t.preds.is_empty() || !t.constraint.is_asap()) {
                return Err(summary_hold_not_yet(&t.name));
            }
        }
        let old: Vec<u32> = self.tasks.iter().map(|t| t.outline_level).collect();
        for (t, l) in self.tasks.iter_mut().zip(&after.levels) {
            t.outline_level = *l;
        }
        // A link between a summary and one of its own subtasks is a loop through the outline.
        if schedule::order(self).is_err() {
            for (t, l) in self.tasks.iter_mut().zip(old) {
                t.outline_level = l;
            }
            return Err(ModelError::Invalid(CIRCULAR_OUTLINE.into()));
        }
        Ok(())
    }

    /// Refuses predecessors on a summary task until summaries hold their subtasks (#8).
    pub(crate) fn check_summary_links(&self, id: TaskId, adding: bool) -> Result<(), ModelError> {
        if adding && self.is_summary(id) {
            return Err(ModelError::Invalid("Predecessors on summary tasks aren't supported yet (coming with #8)".into()));
        }
        Ok(())
    }

    /// Refuses a constraint on a summary task until summaries hold their subtasks (#8).
    pub fn check_summary_constraint(&self, id: TaskId, constraint: crate::Constraint) -> Result<(), ModelError> {
        if !constraint.is_asap() && self.is_summary(id) {
            return Err(ModelError::Invalid("Constraints on summary tasks aren't supported yet (coming with #8)".into()));
        }
        Ok(())
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

    fn levels(p: &Project) -> Vec<u32> {
        p.tasks.iter().map(|t| t.outline_level).collect()
    }

    #[test]
    fn summaries_and_subtrees_come_from_row_order() {
        let (p, ids) = project(&[1, 2, 3, 2, 1, 2]);
        let o = Outline::of(&p);
        let summaries: Vec<bool> = (0..6).map(|r| o.is_summary(r)).collect();
        assert_eq!(summaries, [true, true, false, false, true, false]);
        assert_eq!((o.subtree_end(0), o.subtree_end(1), o.subtree_end(2), o.subtree_end(4), o.subtree_end(5)), (4, 3, 3, 6, 6));
        assert_eq!(o.children(), vec![vec![1, 3], vec![2], vec![], vec![], vec![5], vec![]]);
        assert!(p.is_summary(ids[0]) && !p.is_summary(ids[2]) && !p.is_summary(TaskId(99)));
        assert_eq!(o.subtree_end(99), 6);
    }

    #[test]
    fn illegal_stored_levels_read_as_the_nearest_legal_ones() {
        let (mut p, _) = project(&[3, 0, 5, 2, 200]);
        assert_eq!(Outline::of(&p).levels, [1, 1, 2, 2, 3]);
        p.normalize_outline();
        assert_eq!(levels(&p), [1, 1, 2, 2, 3]);
    }

    #[test]
    fn indent_keeps_the_outline_rule_and_moves_the_subtree() {
        let (mut p, ids) = project(&[1, 1, 2, 1]);
        let e = p.indent(&[ids[0]]).unwrap_err().to_string();
        assert!(e.contains("T1 can't be indented: the first row"), "{e}");
        p.indent(&[ids[1]]).unwrap();
        assert_eq!(levels(&p), [1, 2, 3, 1], "T3 moves with its summary T2");
        let e = p.indent(&[ids[2]]).unwrap_err().to_string();
        assert!(e.contains("at most one level below"), "{e}");
        assert_eq!(levels(&p), [1, 2, 3, 1], "a refused indent changes nothing");
        // Several at once, a subtask named alongside its summary moves once.
        p.indent(&[ids[3]]).unwrap();
        assert_eq!(levels(&p), [1, 2, 3, 2]);
        p.outdent(&[ids[1], ids[2]]).unwrap();
        assert_eq!(levels(&p), [1, 1, 2, 2]);
        assert!(p.indent(&[TaskId(42)]).is_err());
        p.indent(&[]).unwrap();
    }

    #[test]
    fn outdent_moves_the_subtree_and_adopts_the_rows_below() {
        let (mut p, ids) = project(&[1, 2, 3, 2]);
        let e = p.outdent(&[ids[0]]).unwrap_err().to_string();
        assert!(e.contains("already at the top level"), "{e}");
        p.outdent(&[ids[1]]).unwrap();
        // T2 (with T3) comes up a level; T4, below it, becomes its subtask, as in Project.
        assert_eq!(levels(&p), [1, 1, 2, 2]);
        assert!(p.is_summary(ids[1]) && !p.is_summary(ids[0]));
    }

    #[test]
    fn indenting_under_a_linked_predecessor_is_circular() {
        // Worked example 10's shape: T2 follows T1; indenting T2 under T1 would link a summary to its own subtask.
        let (mut p, ids) = project(&[1, 1]);
        p.set_preds(ids[1], parse_preds("1").unwrap()).unwrap();
        assert_eq!(p.indent(&[ids[1]]).unwrap_err().to_string(), CIRCULAR_OUTLINE);
        assert_eq!(levels(&p), [1, 1]);
    }

    #[test]
    fn a_task_with_links_or_a_constraint_cannot_become_a_summary_yet() {
        let (mut p, ids) = project(&[1, 1, 1]);
        p.set_preds(ids[1], parse_preds("1").unwrap()).unwrap();
        let e = p.indent(&[ids[2]]).unwrap_err().to_string();
        assert!(e.contains("T2 has predecessors"), "{e}");
        // A summary's subtasks may link anywhere outside it; a summary may be a predecessor.
        let (mut p, ids) = project(&[1, 2, 1]);
        p.set_preds(ids[2], parse_preds("1").unwrap()).unwrap();
        assert!(p.set_preds(ids[0], parse_preds("3").unwrap()).unwrap_err().to_string().contains("aren't supported yet"));
        p.set_preds(ids[0], vec![]).unwrap();
        // A subtask linked to its own summary is a loop.
        assert!(p.set_preds(ids[1], parse_preds("1").unwrap()).is_err());
        assert!(p.check_summary_constraint(ids[0], crate::Constraint::StartNoEarlierThan(p.start)).is_err());
        assert!(p.check_summary_constraint(ids[0], crate::Constraint::Asap).is_ok());
        assert!(p.check_summary_constraint(ids[1], crate::Constraint::StartNoEarlierThan(p.start)).is_ok());
    }

    proptest::proptest! {
        #[test]
        fn outline_changes_never_panic_and_keep_the_rule(stored in proptest::collection::vec(0u32..6, 0..12), picks in proptest::collection::vec((0u32..14, proptest::bool::ANY), 0..8)) {
            let (mut p, _) = project(&stored);
            p.normalize_outline();
            for (id, indent) in picks {
                let ids = [TaskId(id)];
                let _ = if indent { p.indent(&ids) } else { p.outdent(&ids) };
                let now = levels(&p);
                proptest::prop_assert_eq!(&Outline::of(&p).levels, &now);
            }
        }
    }
}
