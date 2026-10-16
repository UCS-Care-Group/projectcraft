//! Worked examples for deadlines. A deadline is a target finish, not a constraint: it never moves
//! the task, but it caps the late finish (LF = min(LF, deadline)), so slack is measured from the
//! deadline when that is earlier, and a finish after the deadline gives negative slack that runs
//! back to the predecessors. Worked out by hand from Microsoft's documentation, before the code:
//!
//! - Deadline field ("does not affect the scheduling of the task", "can affect Total Slack"):
//!   https://support.microsoft.com/en-us/office/deadline-task-field-8c6c4acd-1324-484a-a92b-9bd2eb313dfa
//! - Show slack ("negative slack can also occur when a task is scheduled to finish after its
//!   deadline"; a deadline earlier than the late finish makes total slack count from the deadline):
//!   https://support.microsoft.com/en-us/project/show-slack-in-your-project-in-project-desktop
//! - Total Slack field: https://support.microsoft.com/en-us/project/total-slack-task-field

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Driver, Project, TaskDates, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Project start Monday 2026-01-05. A (3 days, Mon 5 – Wed 7) → B (2 days, Thu 8 – Fri 9), and
/// D (10 days, Mon 5 – Fri 16) alone, which sets the project finish. Without a deadline B's late
/// finish is Fri 16: slack 5, and A's 5.
fn plan() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "B", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s.run("task.add", &json!({"name": "D", "duration": 10})).unwrap();
    s
}

fn dates(s: &Session, id: u32) -> TaskDates {
    s.schedule.as_ref().unwrap().tasks[&TaskId(id)]
}

fn deadline(s: &mut Session, id: u32, d: &str) {
    s.run("task.setDeadline", &json!({"id": id, "date": d})).unwrap();
}

fn unmoved(s: &Session) {
    assert_eq!((dates(s, 1).start, dates(s, 1).finish), (date("2026-01-05"), date("2026-01-07")));
    assert_eq!((dates(s, 2).start, dates(s, 2).finish), (date("2026-01-08"), date("2026-01-09")));
    assert_eq!(dates(s, 2).driver, Driver::Link(TaskId(1)));
    assert_eq!(s.schedule.as_ref().unwrap().finish, date("2026-01-16"));
}

/// Deadline Wed 14 on B, before its late finish Fri 16: LF = Wed 14, so B's slack is 3 (Fri 9 to
/// Wed 14), measured from the deadline, and A's is 3 too. Nothing moves; it is not missed.
#[test]
fn a_deadline_before_the_late_finish_measures_slack_from_the_deadline() {
    let mut s = plan();
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (5, 5));
    deadline(&mut s, 2, "2026-01-14");
    unmoved(&s);
    assert_eq!(s.project.task(TaskId(2)).unwrap().deadline, Some(date("2026-01-14")));
    assert_eq!(dates(&s, 2).late_finish, date("2026-01-14"));
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (3, 3));
    assert!(!dates(&s, 2).deadline_missed);
    assert!(!dates(&s, 2).critical);

    // On its finish day: slack 0, critical, still not missed.
    deadline(&mut s, 2, "2026-01-09");
    unmoved(&s);
    assert_eq!(dates(&s, 2).slack, 0);
    assert!(dates(&s, 2).critical && dates(&s, 1).critical);
    assert!(!dates(&s, 2).deadline_missed);

    // On a Saturday: counts as the Friday before, as a Finish No Later Than date does.
    deadline(&mut s, 2, "2026-01-10");
    assert_eq!(dates(&s, 2).slack, 0);
    assert!(!dates(&s, 2).deadline_missed);

    // After the project finish: no effect on slack.
    deadline(&mut s, 2, "2026-02-27");
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (5, 5));
}

/// Deadline Wed 7 on B, which finishes Fri 9: missed by two working days. B is not moved (unlike
/// a Finish No Later Than), its slack is −2, and A's −2 too. No constraint conflict is raised.
#[test]
fn a_missed_deadline_is_flagged_with_negative_slack_and_nothing_moves() {
    let mut s = plan();
    deadline(&mut s, 2, "2026-01-07");
    unmoved(&s);
    assert!(dates(&s, 2).deadline_missed);
    assert!(!dates(&s, 1).deadline_missed, "A has no deadline");
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-2, -2), "negative slack runs back to the predecessor");
    assert!(!dates(&s, 2).constraint_conflict);
    assert_eq!(dates(&s, 2).late_finish, date("2026-01-07"));

    // A deadline on a milestone: C (0 days) after B sits on Fri 9.
    s.run("task.add", &json!({"name": "C", "duration": 0})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 4, "text": "2"})).unwrap();
    deadline(&mut s, 4, "2026-01-09");
    assert!(!dates(&s, 4).deadline_missed);
    assert_eq!(dates(&s, 4).slack, 0);
    deadline(&mut s, 4, "2026-01-08");
    assert!(dates(&s, 4).deadline_missed);
    assert_eq!(dates(&s, 4).slack, -1);
}

/// `task.setDeadline` with no date clears it, and slack returns; it is one undo step each way.
#[test]
fn clearing_a_deadline_restores_the_slack() {
    let mut s = plan();
    deadline(&mut s, 2, "2026-01-07");
    assert_eq!(dates(&s, 2).slack, -2);
    s.run("task.setDeadline", &json!({"id": 2})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().deadline, None);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (5, 5));
    assert!(!dates(&s, 2).deadline_missed);
    s.run("edit.undo", &json!({})).unwrap();
    assert!(dates(&s, 2).deadline_missed);
}

/// A deadline on a summary caps its subtasks' late finish: Phase { A → B } finishes Fri 9;
/// a deadline of Thu 8 on Phase gives A and B −1 and flags the summary.
#[test]
fn a_deadline_on_a_summary_caps_its_subtasks() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (n, d) in [("Phase", 1), ("A", 3), ("B", 2), ("D", 10)] {
        s.run("task.add", &json!({"name": n, "duration": d})).unwrap();
    }
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    deadline(&mut s, 1, "2026-01-08");
    assert_eq!(dates(&s, 1).finish, date("2026-01-09"));
    assert!(dates(&s, 1).deadline_missed);
    assert!(!dates(&s, 3).deadline_missed);
    assert_eq!((dates(&s, 2).slack, dates(&s, 3).slack, dates(&s, 1).slack), (-1, -1, -1));

    // A predecessor of the summary itself: X (1 day) → Phase. Phase now runs Tue 6 – Mon 12,
    // missing Thu 8 by two working days, and X carries the −2 back.
    s.run("task.add", &json!({"name": "X", "duration": 1})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 1, "text": "5"})).unwrap();
    assert_eq!(dates(&s, 1).finish, date("2026-01-12"));
    assert!(dates(&s, 1).deadline_missed);
    assert_eq!((dates(&s, 5).slack, dates(&s, 2).slack), (-2, -2));
}

/// Deadlines save and load; a file without one has none (no format change).
#[test]
fn deadlines_round_trip_through_the_file() {
    let mut s = plan();
    let plain = s.save_json().unwrap();
    assert!(!plain.contains("deadline"), "absent when unset: {plain}");
    deadline(&mut s, 2, "2026-01-07");
    let text = s.save_json().unwrap();
    assert!(text.contains("\"deadline\": \"2026-01-07\""), "{text}");
    let back = Session::load_json(&text).unwrap();
    assert_eq!(back.project, s.project);
    assert_eq!(dates(&back, 2), dates(&s, 2));
}

/// Hostile dates: refused cleanly, or (at the ends of the calendar) scheduled without a panic.
#[test]
fn hostile_deadlines_never_panic() {
    let mut s = plan();
    let before = s.project.clone();
    for bad in ["", "2026-02-30", "0000-00-00", "next week", "2026-1-7x", "99999-01-01"] {
        assert!(s.run("task.setDeadline", &json!({"id": 2, "date": bad})).is_err(), "{bad}");
    }
    assert!(s.run("task.setDeadline", &json!({"id": 99, "date": "2026-01-07"})).is_err());
    assert!(s.run("task.setDeadline", &json!({"id": 2, "date": 7})).is_err());
    assert_eq!(s.project, before);
    for edge in ["0001-01-01", "2025-12-31", "9999-12-31"] {
        deadline(&mut s, 2, edge);
        deadline(&mut s, 1, edge);
        assert!(s.schedule.is_ok(), "{edge}");
    }
    // Long before the project start: missed, with dates unmoved.
    deadline(&mut s, 2, "0001-01-01");
    unmoved(&s);
    assert!(dates(&s, 2).deadline_missed);
    assert!(dates(&s, 2).slack < 0);
}
