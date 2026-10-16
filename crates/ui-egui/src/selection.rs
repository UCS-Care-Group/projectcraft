//! The task sheet's row selection and its cursor (the row whose cell has the focus). Rows are
//! held by Unique ID, so the selection stays on the same tasks when rows renumber. Plain data: the
//! sheet feeds it clicks and keys, and turns it into command params.

use std::collections::BTreeSet;

use projectcraft_engine::model::TaskId;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    rows: BTreeSet<TaskId>,
    /// Where a Shift-click or Shift+arrow range starts.
    anchor: Option<TaskId>,
    /// The row with the focused cell.
    pub cursor: Option<TaskId>,
}

impl Selection {
    pub fn is_selected(&self, id: TaskId) -> bool {
        self.rows.contains(&id)
    }

    /// Selects one row and puts the cursor on it.
    pub fn select_only(&mut self, id: TaskId) {
        self.rows = BTreeSet::from([id]);
        self.anchor = Some(id);
        self.cursor = Some(id);
    }

    /// A click on a row: alone, Shift (a range from the anchor) or Ctrl (add or remove one row).
    /// `order` is every row's Unique ID, top to bottom.
    pub fn click(&mut self, id: TaskId, order: &[TaskId], shift: bool, ctrl: bool) {
        if shift {
            let anchor = self.anchor.unwrap_or(id);
            self.rows = range(order, anchor, id);
            self.anchor = Some(anchor);
            self.cursor = Some(id);
        } else if ctrl {
            if !self.rows.remove(&id) {
                self.rows.insert(id);
            }
            self.anchor = Some(id);
            self.cursor = Some(id);
        } else {
            self.select_only(id);
        }
    }

    /// Moves the cursor `by` rows (negative is up), stopping at the first and last row. With
    /// `extend` the selection grows from the anchor, as Shift+arrow; otherwise it follows the cursor.
    pub fn step(&mut self, order: &[TaskId], by: isize, extend: bool) {
        let here = self.cursor.and_then(|c| order.iter().position(|t| *t == c));
        let to = match here {
            Some(i) => i.saturating_add_signed(by).min(order.len().saturating_sub(1)),
            None => 0,
        };
        let Some(&id) = order.get(to) else { return };
        if extend && here.is_some() {
            self.click(id, order, true, false);
        } else {
            self.select_only(id);
        }
    }

    /// The selected rows' Unique IDs, top to bottom; the cursor's row when none is selected.
    pub fn ids(&self, order: &[TaskId]) -> Vec<TaskId> {
        let picked: Vec<TaskId> = order.iter().copied().filter(|t| self.rows.contains(t)).collect();
        if picked.is_empty() { self.cursor.into_iter().filter(|c| order.contains(c)).collect() } else { picked }
    }

    /// Drops rows that no longer exist (after a delete or an undo). A cursor whose row went moves
    /// to the row now at `fallback` (0-based), or the last row.
    pub fn prune(&mut self, order: &[TaskId], fallback: Option<usize>) {
        self.rows.retain(|t| order.contains(t));
        if self.anchor.is_some_and(|a| !order.contains(&a)) {
            self.anchor = None;
        }
        if let Some(c) = self.cursor
            && !order.contains(&c)
        {
            match fallback.and_then(|i| order.get(i).or(order.last())) {
                Some(&id) => self.select_only(id),
                None => *self = Selection::default(),
            }
        }
    }
}

/// The rows from `a` to `b`, either way round.
fn range(order: &[TaskId], a: TaskId, b: TaskId) -> BTreeSet<TaskId> {
    let at = |id| order.iter().position(|t| *t == id);
    match (at(a), at(b)) {
        (Some(i), Some(j)) => order.get(i.min(j)..=i.max(j)).unwrap_or_default().iter().copied().collect(),
        _ => BTreeSet::from([b]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<TaskId> {
        [4, 1, 7, 2, 9].map(TaskId).to_vec()
    }

    #[test]
    fn click_shift_click_and_ctrl_click() {
        let o = order();
        let mut s = Selection::default();
        s.click(TaskId(1), &o, false, false);
        assert_eq!(s.ids(&o), [TaskId(1)]);
        s.click(TaskId(2), &o, true, false);
        assert_eq!(s.ids(&o), [1, 7, 2].map(TaskId), "rows, not Unique IDs, make the range");
        s.click(TaskId(7), &o, false, true);
        assert_eq!(s.ids(&o), [1, 2].map(TaskId));
        s.click(TaskId(9), &o, false, true);
        assert_eq!(s.ids(&o), [1, 2, 9].map(TaskId));
        assert_eq!(s.cursor, Some(TaskId(9)));
        // Shift-click upwards from the last anchor.
        s.click(TaskId(4), &o, true, false);
        assert_eq!(s.ids(&o), o);
        s.click(TaskId(7), &o, false, false);
        assert_eq!(s.ids(&o), [TaskId(7)]);
    }

    #[test]
    fn arrows_move_and_extend_and_stop_at_the_ends() {
        let o = order();
        let mut s = Selection::default();
        s.step(&o, 1, false);
        assert_eq!(s.cursor, Some(TaskId(4)), "no cursor yet: the first row");
        s.step(&o, -1, false);
        assert_eq!(s.cursor, Some(TaskId(4)));
        s.step(&o, 1, true);
        s.step(&o, 1, true);
        assert_eq!(s.ids(&o), [4, 1, 7].map(TaskId));
        s.step(&o, -1, true);
        assert_eq!(s.ids(&o), [4, 1].map(TaskId));
        s.step(&o, 10, false);
        assert_eq!(s.ids(&o), [TaskId(9)]);
        assert!(s.is_selected(TaskId(9)) && !s.is_selected(TaskId(4)));
        Selection::default().step(&[], 1, false);
    }

    #[test]
    fn deleted_rows_leave_the_selection_and_the_cursor_moves_to_the_same_row() {
        let mut s = Selection::default();
        let o = order();
        s.click(TaskId(7), &o, false, false);
        s.click(TaskId(2), &o, true, false);
        let left = [4, 1, 9].map(TaskId);
        s.prune(&left, Some(2));
        assert_eq!(s.ids(&left), [TaskId(9)]);
        s.prune(&[TaskId(4)], Some(2));
        assert_eq!(s.ids(&[TaskId(4)]), [TaskId(4)], "past the end: the last row");
        s.prune(&[], Some(0));
        assert_eq!(s, Selection::default());
        // Only the cursor: it is the selection.
        let s = Selection { cursor: Some(TaskId(1)), ..Selection::default() };
        assert_eq!(s.ids(&o), [TaskId(1)]);
        assert!(s.ids(&left[..1]).is_empty());
    }
}
