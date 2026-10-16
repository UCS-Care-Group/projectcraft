//! Summary tasks through the commands (`task.indent`, `task.outdent`): the outline rule, roll-up,
//! one undo step, and worked example 13.
//! Source for the behaviour: "Create and work with subtasks and summary tasks",
//! https://support.microsoft.com/office/create-and-work-with-subtasks-and-summary-tasks-in-project-desktop-b3ff64ce-b121-42cc-905b-cb9b8ce0255f

use projectcraft_engine::Session;
use projectcraft_engine::model::{CIRCULAR_OUTLINE, Date, Project, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Project start Monday 2026-01-05. Rows: 1 Phase (4 days of its own), 2 A (3 days), 3 B (2 days,
/// after A), all at the top level.
fn rows() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("Phase", 4), ("A", 3), ("B", 2)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    s
}

fn levels(s: &Session) -> Vec<u32> {
    s.project.tasks.iter().map(|t| t.outline_level).collect()
}

fn start_finish(s: &Session, id: u32) -> (Date, Date) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start, d.finish)
}

fn duration(s: &Session, id: u32) -> i64 {
    s.project.task(TaskId(id)).unwrap().duration
}

#[test]
fn indenting_several_tasks_is_one_undo_step_and_rolls_up() {
    let mut s = rows();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    assert_eq!(levels(&s), [1, 2, 2]);
    assert!(s.project.is_summary(TaskId(1)));
    // A Mon 5 – Wed 7, B Thu 8 – Fri 9: Phase spans both, five working days.
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-09")));
    assert_eq!(duration(&s, 1), 5);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(levels(&s), [1, 1, 1]);
    assert_eq!(duration(&s, 1), 4, "undo brings back the duration it had before it was a summary");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(levels(&s), [1, 2, 2]);
}

#[test]
fn the_outline_rule_is_enforced_and_a_refusal_changes_nothing() {
    let mut s = rows();
    let before = s.project.clone();
    let e = s.run("task.indent", &json!({"ids": [1]})).unwrap_err().to_string();
    assert!(e.contains("first row"), "{e}");
    s.run("task.indent", &json!({"ids": [2]})).unwrap();
    let e = s.run("task.indent", &json!({"ids": [2]})).unwrap_err().to_string();
    assert!(e.contains("one level below the row above"), "{e}");
    let e = s.run("task.outdent", &json!({"ids": [1]})).unwrap_err().to_string();
    assert!(e.contains("already at the top level"), "{e}");
    assert!(s.run("task.indent", &json!({"ids": [9]})).is_err());
    assert!(s.run("task.indent", &json!({"ids": ["2"]})).is_err());
    assert!(s.run("task.indent", &json!({"ids": 2})).is_err());
    assert!(s.run("task.indent", &json!({})).is_err());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, before);
}

#[test]
fn a_summary_duration_cannot_be_set_and_its_subtasks_drive_it() {
    let mut s = rows();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    let e = s.run("task.setDuration", &json!({"id": 1, "duration": 9})).unwrap_err().to_string();
    assert!(e.contains("rolled up"), "{e}");
    s.run("task.setDuration", &json!({"id": 2, "duration": 5})).unwrap();
    assert_eq!(duration(&s, 1), 7);
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-13")));
}

#[test]
fn a_summary_takes_links_both_ways_but_not_to_its_own_subtasks() {
    let mut s = rows();
    s.run("task.add", &json!({"name": "C", "duration": 1})).unwrap();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    // A summary as predecessor: C follows Phase's finish, Friday 9, on Monday 12.
    s.run("task.setPredecessors", &json!({"id": 4, "text": "1"})).unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-12"), date("2026-01-12")));
    // C, following Phase, can't become one of its subtasks: a circular relationship.
    assert_eq!(s.run("task.indent", &json!({"ids": [4]})).unwrap_err().to_string(), CIRCULAR_OUTLINE);
    // Unlinked, C joins Phase; one more level makes B, which follows A, a summary over C.
    s.run("task.setPredecessors", &json!({"id": 4, "text": ""})).unwrap();
    s.run("task.indent", &json!({"ids": [4]})).unwrap();
    s.run("task.indent", &json!({"ids": [4]})).unwrap();
    assert!(s.project.is_summary(TaskId(3)));
    // B's link from A now holds C: Thursday 8.
    assert_eq!(start_finish(&s, 4), (date("2026-01-08"), date("2026-01-08")));
}

/// Worked example 10's shape: indenting a task under its own linked predecessor is refused.
/// Source: https://learn.microsoft.com/en-us/office/troubleshoot/project/circular-relationship-error-when-indenting-task
#[test]
fn indenting_under_a_linked_predecessor_is_a_circular_relationship() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A"})).unwrap();
    s.run("task.add", &json!({"name": "B"})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    let e = s.run("task.indent", &json!({"ids": [2]})).unwrap_err().to_string();
    assert_eq!(e, CIRCULAR_OUTLINE);
}

/// Worked example 13: a summary that loses its last subtask becomes a normal task with its last
/// rolled-up duration, whether the subtask is outdented or deleted. Project's documentation is
/// silent on the duration kept; the spec records this as our choice (parity notes).
#[test]
fn worked_example_13_a_summary_that_loses_its_last_subtask_keeps_its_rolled_up_duration() {
    // Outdented: Phase {A (3d)}; A comes up to the top level.
    let mut s = rows();
    s.run("task.indent", &json!({"ids": [2]})).unwrap();
    assert_eq!(duration(&s, 1), 3);
    s.run("task.outdent", &json!({"ids": [2]})).unwrap();
    assert!(!s.project.is_summary(TaskId(1)));
    assert_eq!(duration(&s, 1), 3, "not its own 4 from before");
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-07")));
    // Now a normal task: its duration can be set again.
    s.run("task.setDuration", &json!({"id": 1, "duration": 2})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-06")));

    // Deleted: Phase {A (3d), B (2d)} rolls up to 5; delete both subtasks.
    let mut s = rows();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    s.run("task.delete", &json!({"ids": [3]})).unwrap();
    assert_eq!(duration(&s, 1), 3, "rolled up from A alone");
    s.run("task.delete", &json!({"ids": [2]})).unwrap();
    assert!(!s.project.is_summary(TaskId(1)));
    assert_eq!(duration(&s, 1), 3);
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-07")));
}

#[test]
fn outline_levels_survive_save_and_old_or_damaged_levels_load() {
    let mut s = rows();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    let round = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(round.project, s.project);
    // A file whose levels break the rule loads with the nearest legal ones.
    let damaged = json!({"name": "x", "start": "2026-01-05", "tasks": [
        {"id": 1, "name": "a", "duration": 1, "outline_level": 4},
        {"id": 2, "name": "b", "duration": 1, "outline_level": 9},
        {"id": 3, "name": "c", "duration": 1, "outline_level": 0},
    ]});
    let s = Session::load_json(&damaged.to_string()).unwrap();
    assert_eq!(levels(&s), [1, 2, 1]);
    assert_eq!(duration(&s, 1), 1);
    assert!(s.schedule.is_ok());
}
