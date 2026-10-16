//! Inserting, moving and deleting rows through the commands (`task.insert`, `task.move`,
//! `task.delete`): Unique IDs are kept and never reused, links survive a reorder, a summary takes
//! its whole subtree, and each command is one undo step.
//! Source for the behaviour: "Create and work with subtasks and summary tasks" ("When you move or
//! delete a summary task, Project moves or deletes all of its subtasks"),
//! https://support.microsoft.com/office/create-and-work-with-subtasks-and-summary-tasks-in-project-desktop-b3ff64ce-b121-42cc-905b-cb9b8ce0255f

use projectcraft_engine::Session;
use projectcraft_engine::model::{CIRCULAR_OUTLINE, Date, Project, TaskId, preds_text};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Unique IDs in row order.
fn order(s: &Session) -> Vec<u32> {
    s.project.tasks.iter().map(|t| t.id.0).collect()
}

fn levels(s: &Session) -> Vec<u32> {
    s.project.tasks.iter().map(|t| t.outline_level).collect()
}

fn preds(s: &Session, id: u32) -> String {
    preds_text(&s.project.task(TaskId(id)).unwrap().preds)
}

fn rows_text(s: &Session, id: u32) -> String {
    s.project.preds_text_rows(&s.project.task(TaskId(id)).unwrap().preds)
}

/// 1 Phase {2 A (3d), 3 B (2d, after A)}, 4 C (1d), 5 D (1d, after C).
fn phased() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("Phase", 1), ("A", 3), ("B", 2), ("C", 1), ("D", 1)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 5, "text": "4"})).unwrap();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    assert_eq!(levels(&s), [1, 2, 2, 1, 1]);
    s
}

/// Worked example: reordering rows keeps every Unique ID and every link; only the ID column
/// (row numbers) and the Predecessors column, which shows row IDs, renumber. The dates don't move.
#[test]
fn worked_example_unique_ids_kept_and_links_survive_reorder() {
    let mut s = projectcraft_engine::sample();
    let ids_before: Vec<u32> = {
        let mut v = order(&s);
        v.sort();
        v
    };
    let links_before: Vec<String> = s.project.tasks.iter().map(|t| preds_text(&t.preds)).collect();
    let dates_before = s.schedule.as_ref().unwrap().tasks.clone();
    // Network cabling (Unique ID 5, row 5) moves above Survey the new floor (row 2).
    s.run("task.move", &json!({"ids": [5], "to": 2})).unwrap();
    assert_eq!(order(&s), [1, 5, 2, 3, 4, 6, 7, 8]);
    assert_eq!(s.project.row_of(TaskId(5)), Some(2));
    // Network cabling still follows Survey the new floor (Unique ID 2), now on row 3.
    assert_eq!(preds(&s, 5), "2");
    assert_eq!(rows_text(&s, 5), "3");
    // Pack the old office follows 4FS-1 and 5 by Unique ID; by row that's now 5FS-1 and 2.
    assert_eq!(preds(&s, 6), "4FS-1, 5");
    assert_eq!(rows_text(&s, 6), "5FS-1, 2");
    let mut ids_after = order(&s);
    ids_after.sort();
    assert_eq!(ids_after, ids_before);
    let by_id: Vec<String> = {
        let mut ts = s.project.tasks.clone();
        ts.sort_by_key(|t| t.id);
        ts.iter().map(|t| preds_text(&t.preds)).collect()
    };
    assert_eq!(by_id, links_before);
    assert_eq!(s.schedule.as_ref().unwrap().tasks, dates_before, "same plan, same dates");
    let round = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(round.project, s.project);
}

#[test]
fn deleted_unique_ids_are_never_reused() {
    let mut s = projectcraft_engine::sample();
    s.run("task.delete", &json!({"ids": [8, 3]})).unwrap();
    assert_eq!(order(&s), [1, 2, 4, 5, 6, 7]);
    // Links to deleted tasks go with them.
    assert_eq!(preds(&s, 4), "");
    s.run("task.insert", &json!({"before": 2})).unwrap();
    assert_eq!(order(&s), [1, 9, 2, 4, 5, 6, 7], "9, not a deleted 3 or 8");
    s.run("task.insert", &json!({"name": "Last"})).unwrap();
    assert_eq!(s.project.tasks.last().unwrap().id, TaskId(10));
    let mut round = Session::load_json(&s.save_json().unwrap()).unwrap();
    round.run("task.insert", &json!({})).unwrap();
    assert_eq!(round.project.tasks.last().unwrap().id, TaskId(11));
}

#[test]
fn insert_goes_above_a_task_at_its_outline_level() {
    let mut s = phased();
    s.run("task.insert", &json!({"before": 3, "name": "A2", "duration": 4})).unwrap();
    assert_eq!(order(&s), [1, 2, 6, 3, 4, 5]);
    assert_eq!(levels(&s), [1, 2, 2, 2, 1, 1], "a subtask, like the row it went above");
    let t = s.project.task(TaskId(6)).unwrap();
    assert_eq!((t.name.as_str(), t.duration), ("A2", 4));
    // Above a top-level row: top level. At the end: the level of the last row.
    s.run("task.insert", &json!({"before": 4})).unwrap();
    assert_eq!(levels(&s), [1, 2, 2, 2, 1, 1, 1]);
    assert_eq!(s.project.task(TaskId(7)).unwrap().name, "New task");
    s.run("task.setPredecessors", &json!({"id": 5, "text": ""})).unwrap();
    s.run("task.indent", &json!({"ids": [5]})).unwrap();
    s.run("task.insert", &json!({})).unwrap();
    assert_eq!(order(&s), [1, 2, 6, 3, 7, 4, 5, 8]);
    assert_eq!(levels(&s), [1, 2, 2, 2, 1, 1, 2, 2]);
    // An unknown task: refused, nothing changes.
    let before = s.project.clone();
    assert!(s.run("task.insert", &json!({"before": 99})).is_err());
    assert!(s.run("task.insert", &json!({"duration": -1})).is_err());
    assert_eq!(s.project, before);
}

#[test]
fn moving_a_summary_takes_its_subtree() {
    let mut s = phased();
    // Phase with A and B to the end.
    s.run("task.move", &json!({"ids": [1]})).unwrap();
    assert_eq!(order(&s), [4, 5, 1, 2, 3]);
    assert_eq!(levels(&s), [1, 1, 1, 2, 2]);
    assert!(s.project.is_summary(TaskId(1)));
    // And back above C.
    s.run("task.move", &json!({"ids": [1], "to": 4})).unwrap();
    assert_eq!(order(&s), [1, 2, 3, 4, 5]);
    assert_eq!(levels(&s), [1, 2, 2, 1, 1]);
    // Naming a subtask beside its summary moves it once, with the summary.
    s.run("task.move", &json!({"ids": [3, 1], "to": 5})).unwrap();
    assert_eq!(order(&s), [4, 1, 2, 3, 5]);
    assert_eq!(levels(&s), [1, 1, 2, 2, 1]);
}

#[test]
fn moved_rows_take_the_level_of_the_row_they_go_above() {
    let mut s = phased();
    // C (top level) above B: it becomes a subtask of Phase.
    s.run("task.move", &json!({"ids": [4], "to": 3})).unwrap();
    assert_eq!(order(&s), [1, 2, 4, 3, 5]);
    assert_eq!(levels(&s), [1, 2, 2, 2, 1]);
    // A (a subtask) above D: top level. To the end: top level.
    s.run("task.move", &json!({"ids": [2], "to": 5})).unwrap();
    assert_eq!(order(&s), [1, 4, 3, 2, 5]);
    assert_eq!(levels(&s), [1, 2, 2, 1, 1]);
    s.run("task.move", &json!({"ids": [3]})).unwrap();
    assert_eq!(order(&s), [1, 4, 2, 5, 3]);
    assert_eq!(levels(&s), [1, 2, 1, 1, 1]);
    // Phase's last subtask leaves it: it keeps its rolled-up duration (C, 1 day).
    s.run("task.move", &json!({"ids": [4]})).unwrap();
    assert!(!s.project.is_summary(TaskId(1)));
    assert_eq!(s.project.task(TaskId(1)).unwrap().duration, 1);
}

#[test]
fn deleting_a_summary_takes_its_subtree() {
    let mut s = phased();
    s.run("task.delete", &json!({"ids": [1]})).unwrap();
    assert_eq!(order(&s), [4, 5]);
    assert_eq!(preds(&s, 5), "4");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(order(&s), [1, 2, 3, 4, 5]);
    // A subtask alone: its summary and sibling stay; B's link to A goes.
    s.run("task.delete", &json!({"ids": [2]})).unwrap();
    assert_eq!(order(&s), [1, 3, 4, 5]);
    assert_eq!(levels(&s), [1, 2, 1, 1]);
    assert_eq!(preds(&s, 3), "");
}

#[test]
fn each_command_is_one_undo_step_and_all_or_nothing() {
    let mut s = phased();
    let start = s.project.clone();
    s.run("task.move", &json!({"ids": [1, 5], "to": 4})).unwrap();
    assert_eq!(order(&s), [1, 2, 3, 5, 4]);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, start, "one undo step undoes the whole move");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(order(&s), [1, 2, 3, 5, 4]);
    s.run("edit.undo", &json!({})).unwrap();

    s.run("task.delete", &json!({"ids": [2, 4, 5]})).unwrap();
    assert_eq!(order(&s), [1, 3]);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, start);
    s.run("task.insert", &json!({"before": 2})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, start);

    // One unknown task refuses the whole command.
    assert!(s.run("task.delete", &json!({"ids": [2, 99]})).is_err());
    assert!(s.run("task.move", &json!({"ids": [2, 99], "to": 4})).is_err());
    assert!(s.run("task.move", &json!({"ids": [2], "to": 99})).is_err());
    assert!(s.run("task.delete", &json!({"id": 2})).is_err(), "`ids`, a list, not `id`");
    assert_eq!(s.project, start);
    // Nothing named: nothing to do.
    s.run("task.delete", &json!({"ids": []})).unwrap();
    s.run("task.move", &json!({"ids": []})).unwrap();
    assert_eq!(s.project, start);
}

#[test]
fn a_move_into_itself_or_into_a_loop_is_refused() {
    let mut s = phased();
    let start = s.project.clone();
    // Above one of its own subtasks, or itself.
    let e = s.run("task.move", &json!({"ids": [1], "to": 3})).unwrap_err().to_string();
    assert!(e.contains("can't move above itself"), "{e}");
    assert!(s.run("task.move", &json!({"ids": [4], "to": 4})).is_err());
    s.run("task.setPredecessors", &json!({"id": 4, "text": "1"})).unwrap();
    let linked = s.project.clone();
    // C follows Phase; moving C into Phase (above B) would link a summary to its own subtask.
    assert_eq!(s.run("task.move", &json!({"ids": [4], "to": 3})).unwrap_err().to_string(), CIRCULAR_OUTLINE);
    assert_eq!(s.project, linked);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, start);
}
