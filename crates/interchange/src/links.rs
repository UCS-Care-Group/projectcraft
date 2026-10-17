//! Links read from another program's file that ProjectCraft's editing commands would refuse: to
//! the task itself, repeated, between a summary and its own subtask ([`refused`]), or closing a
//! loop ([`drop_loops`]). An import drops each instead, with a report line, so the scheduler never
//! meets a loop.
//!
//! The graph is the scheduler's: each task's finish, and for each summary a hold that its
//! predecessors feed, that comes after its own summary's hold and before each of its subtasks,
//! which come before its finish. Links are kept in row order, each unless it closes a loop with
//! the outline and the links kept before it, as typing them in that order would. Only the links
//! inside a loop (one strongly connected part of the graph) are searched, within a budget; past it
//! the rest of those links are dropped, so a hostile file can't make the import run on.

use std::collections::{HashMap, HashSet};

use crate::model::{Outline, Project, TaskId};

/// Why a link read from a file can't join its task's links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The task is its own predecessor.
    SelfLink,
    /// The task already has this predecessor; the first link is kept.
    Repeated,
    /// Between a summary task and its own subtask.
    Summary,
}

/// Whether the link from row `from` (task `pred`) to row `to` is refused, given the predecessors
/// `to` has kept so far.
pub(crate) fn refused(outline: &Outline, kept: &HashSet<TaskId>, from: usize, to: usize, pred: TaskId) -> Option<Refused> {
    let within = |inner: usize, outer: usize| inner > outer && inner < outline.subtree_end(outer);
    if from == to {
        Some(Refused::SelfLink)
    } else if kept.contains(&pred) {
        Some(Refused::Repeated)
    } else if within(to, from) || within(from, to) {
        Some(Refused::Summary)
    } else {
        None
    }
}

/// Nodes visited, in all, looking for paths that would close a loop.
const SEARCH_BUDGET: usize = 5_000_000;

/// One link: the row of the task it belongs to, its place in that task's list, and its edge.
struct Edge {
    row: usize,
    index: usize,
    from: usize,
    to: usize,
}

/// Drops every link that closes a loop; returns the rows (0-based) of the tasks that lost one.
pub(crate) fn drop_loops(p: &mut Project) -> Vec<usize> {
    let n = p.tasks.len();
    let outline = Outline::of(p);
    let row_of: HashMap<TaskId, usize> = p.tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); 2 * n];
    let add = |adj: &mut Vec<Vec<usize>>, from: usize, to: usize| {
        if let Some(a) = adj.get_mut(from) {
            a.push(to);
        }
    };
    // The outline: each row's summary is the nearest row above with a lower level.
    let mut open: Vec<usize> = Vec::new();
    for (i, &level) in outline.levels.iter().enumerate() {
        while open.last().is_some_and(|&s| outline.levels.get(s).is_some_and(|&l| l >= level)) {
            open.pop();
        }
        if let Some(&up) = open.last() {
            add(&mut adj, i, up);
            add(&mut adj, n + up, i);
            if outline.is_summary(i) {
                add(&mut adj, n + up, n + i);
            }
        }
        open.push(i);
    }
    let mut links = Vec::new();
    for (row, t) in p.tasks.iter().enumerate() {
        let to = if outline.is_summary(row) { n + row } else { row };
        for (index, l) in t.preds.iter().enumerate() {
            if let Some(&from) = row_of.get(&l.pred) {
                links.push(Edge { row, index, from, to });
            }
        }
    }
    let mut all = adj.clone();
    for e in &links {
        add(&mut all, e.from, e.to);
    }
    let part = parts(&all);
    let same = |a: usize, b: usize| part.get(a).is_some() && part.get(a) == part.get(b);
    let (looping, kept): (Vec<Edge>, Vec<Edge>) = links.into_iter().partition(|e| same(e.from, e.to));
    if looping.is_empty() {
        return Vec::new();
    }
    for e in &kept {
        add(&mut adj, e.from, e.to);
    }
    let mut search = Search { seen: vec![0; 2 * n], round: 0, budget: SEARCH_BUDGET, stack: Vec::new() };
    let mut dropped: Vec<(usize, usize)> = Vec::new();
    for e in &looping {
        let within = part.get(e.from).copied().unwrap_or(usize::MAX);
        if search.reaches(&adj, &part, within, e.to, e.from) == Some(false) {
            add(&mut adj, e.from, e.to);
        } else {
            dropped.push((e.row, e.index));
        }
    }
    let mut rows: Vec<usize> = Vec::new();
    // Highest index first, so the indexes still to remove keep their places.
    for &(row, index) in dropped.iter().rev() {
        if let Some(t) = p.tasks.get_mut(row)
            && index < t.preds.len()
        {
            t.preds.remove(index);
        }
        if rows.last() != Some(&row) {
            rows.push(row);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// A depth-first search for paths, its marks reused from one search to the next.
struct Search {
    seen: Vec<u32>,
    round: u32,
    budget: usize,
    stack: Vec<usize>,
}

impl Search {
    /// Whether `to` is reached from `from` through nodes of part `within`; `None` once the budget
    /// is spent.
    fn reaches(&mut self, adj: &[Vec<usize>], part: &[usize], within: usize, from: usize, to: usize) -> Option<bool> {
        self.round = self.round.checked_add(1)?;
        self.stack.clear();
        self.stack.push(from);
        while let Some(node) = self.stack.pop() {
            if node == to {
                return Some(true);
            }
            self.budget = self.budget.checked_sub(1)?;
            match self.seen.get_mut(node) {
                Some(mark) if *mark != self.round => *mark = self.round,
                _ => continue,
            }
            for &next in adj.get(node).into_iter().flatten() {
                if part.get(next) == Some(&within) && self.seen.get(next).is_some_and(|&m| m != self.round) {
                    self.stack.push(next);
                }
            }
        }
        Some(false)
    }
}

/// Each node's strongly connected part (Kosaraju's two passes, without recursion).
fn parts(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, outs) in adj.iter().enumerate() {
        for &to in outs {
            if let Some(r) = rev.get_mut(to) {
                r.push(from);
            }
        }
    }
    // First pass: nodes in the order their searches finish.
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..n {
        if done.get(start) != Some(&false) {
            continue;
        }
        if let Some(d) = done.get_mut(start) {
            *d = true;
        }
        stack.push((start, 0));
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            match adj.get(node).and_then(|a| a.get(*next)).copied() {
                Some(child) => {
                    *next += 1;
                    if let Some(d) = done.get_mut(child).filter(|d| !**d) {
                        *d = true;
                        stack.push((child, 0));
                    }
                }
                None => {
                    order.push(node);
                    stack.pop();
                }
            }
        }
    }
    // Second pass: on the reversed graph, latest finish first.
    let mut part = vec![usize::MAX; n];
    let mut count = 0;
    let mut todo = Vec::new();
    for &start in order.iter().rev() {
        if part.get(start) != Some(&usize::MAX) {
            continue;
        }
        todo.push(start);
        while let Some(node) = todo.pop() {
            match part.get_mut(node) {
                Some(p) if *p == usize::MAX => *p = count,
                _ => continue,
            }
            for &prev in rev.get(node).into_iter().flatten() {
                if part.get(prev) == Some(&usize::MAX) {
                    todo.push(prev);
                }
            }
        }
        count += 1;
    }
    part
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Date, DurationUnit, Link, LinkKind};

    fn project(levels: &[u32], preds: &[&[u32]]) -> Project {
        let mut p = Project::new("p", Date::EPOCH);
        for (i, level) in levels.iter().enumerate() {
            let id = p.add_task(format!("T{}", i + 1), 480).unwrap();
            p.task_mut(id).unwrap().outline_level = *level;
        }
        for (t, ps) in p.tasks.iter_mut().zip(preds) {
            t.preds = ps.iter().map(|&k| Link { pred: TaskId(k), kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days }).collect();
        }
        p
    }

    #[test]
    fn parts_find_loops() {
        let adj = vec![vec![1], vec![2], vec![0], vec![0]];
        let p = parts(&adj);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert_ne!(p[3], p[0]);
    }

    #[test]
    fn a_plain_chain_keeps_every_link() {
        let mut p = project(&[1, 1, 1], &[&[], &[1], &[2]]);
        assert!(drop_loops(&mut p).is_empty());
        assert_eq!(p.tasks[2].preds.len(), 1);
    }

    #[test]
    fn the_link_that_closes_a_loop_goes() {
        let mut p = project(&[1, 1, 1], &[&[3], &[1], &[2]]);
        assert_eq!(drop_loops(&mut p), [2]);
        assert!(p.tasks[2].preds.is_empty());
        assert!(crate::model::schedule(&p).is_ok());
    }

    #[test]
    fn a_loop_through_a_summary_goes() {
        // T1 summarises T2; T2 → T3 → T1, and T1 waits for T2 to finish.
        let mut p = project(&[1, 2, 1], &[&[3], &[], &[2]]);
        assert!(crate::model::schedule(&p).is_err(), "a loop, as built");
        assert_eq!(drop_loops(&mut p), [2]);
        assert!(crate::model::schedule(&p).is_ok());
    }
}
