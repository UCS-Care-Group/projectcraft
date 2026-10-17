//! Worked examples: the As Late As Possible constraint.
//!
//! Source: Microsoft's definitions of Project's constraints
//! (https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints):
//! "As Late As Possible: Schedules the task as late as it can without delaying subsequent tasks.
//! Use no constraint date. ES = (Calculated) LS." So the task takes its late dates, worked out
//! from its successors, its deadline and the project finish; its successors follow it from there,
//! and with ES = LS it has no slack.
//!
//! The project calendar works Monday to Friday, 8:00–12:00 and 13:00–17:00. The project starts on
//! Monday 5 January 2026. A (5 days) alone sets the project finish, Friday 9 January, unless a
//! test says otherwise. Every date is worked out by hand first.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Constraint, Date, Driver, Moment, Project, TaskDates, TaskId};
use serde_json::{Value, json};

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

fn at(d: &str, h: u16, m: u16) -> Moment {
    Moment { date: date(d), minute: h * 60 + m }
}

/// A plan of these tasks (name, duration in days, predecessors by Unique ID).
fn plan(tasks: &[(&str, i64, &str)]) -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (i, (name, days, preds)) in (1..).zip(tasks) {
        run(&mut s, "task.add", json!({"name": name, "duration": days}));
        run(&mut s, "task.setPredecessors", json!({"id": i, "text": preds}));
    }
    s
}

fn run(s: &mut Session, cmd: &str, params: Value) {
    s.run(cmd, &params).unwrap_or_else(|e| panic!("{cmd} {params}: {e}"));
}

fn alap(s: &mut Session, id: u32) {
    run(s, "task.setConstraint", json!({"id": id, "type": "ALAP"}));
}

fn dates(s: &Session, id: u32) -> TaskDates {
    s.schedule.as_ref().unwrap().tasks[&TaskId(id)]
}

fn days(s: &Session, id: u32) -> (Date, Date) {
    let d = dates(s, id);
    (d.start, d.finish)
}

fn finish(s: &Session) -> Date {
    s.schedule.as_ref().unwrap().finish
}

/// B (1 day) → C (2 days), beside A. As soon as possible B is Monday and C Tuesday–Wednesday,
/// with two days of slack each. As Late As Possible: C's late start is Thursday, so B's late
/// finish is Wednesday evening and its late start Wednesday; B is held there and C follows it,
/// Thursday–Friday. B has no slack (ES = LS) and C's slack is used up too: both critical. The
/// project still finishes on Friday.
#[test]
fn an_alap_task_takes_its_late_dates_and_its_successor_follows() {
    let mut s = plan(&[("A", 5, ""), ("B", 1, ""), ("C", 2, "2")]);
    assert_eq!((days(&s, 2), days(&s, 3)), ((date("2026-01-05"), date("2026-01-05")), (date("2026-01-06"), date("2026-01-07"))));
    alap(&mut s, 2);
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::AsLateAsPossible);
    assert_eq!((dates(&s, 2).start_at, dates(&s, 2).finish_at), (at("2026-01-07", 8, 0), at("2026-01-07", 17, 0)));
    assert_eq!(days(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    let (b, c) = (dates(&s, 2), dates(&s, 3));
    assert_eq!((b.late_start, b.late_finish), (date("2026-01-07"), date("2026-01-07")));
    assert_eq!((b.slack, c.slack), (0, 0));
    assert!(b.critical && c.critical);
    assert_eq!(b.driver, Driver::Constraint);
    assert_eq!(c.driver, Driver::Link(TaskId(2)));
    assert_eq!(finish(&s), date("2026-01-09"));
    // Back to As Soon As Possible: the dates come back.
    run(&mut s, "task.setConstraint", json!({"id": 2, "type": "ASAP"}));
    assert_eq!((days(&s, 2), days(&s, 3)), ((date("2026-01-05"), date("2026-01-05")), (date("2026-01-06"), date("2026-01-07"))));
    assert_eq!(dates(&s, 2).slack, 2 * 480);
}

/// With a lag: B → C finish-to-start +1 day. C's late start is Thursday, so B must finish a
/// working day before it, by Tuesday evening: B is Tuesday. Start-to-start instead: B may start
/// when C does, Thursday.
#[test]
fn a_lag_and_a_start_to_start_link_are_kept() {
    let mut s = plan(&[("A", 5, ""), ("B", 1, ""), ("C", 2, "2FS+1d")]);
    alap(&mut s, 2);
    assert_eq!(days(&s, 2), (date("2026-01-06"), date("2026-01-06")));
    assert_eq!(days(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    run(&mut s, "task.setPredecessors", json!({"id": 3, "text": "2SS"}));
    assert_eq!(days(&s, 2), (date("2026-01-08"), date("2026-01-08")));
    assert_eq!(days(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    assert_eq!(finish(&s), date("2026-01-09"));
}

/// With no successors an As Late As Possible task finishes with the project: D (2 days) on
/// Thursday–Friday, and a milestone at the project finish, Friday 17:00.
#[test]
fn with_no_successors_it_finishes_with_the_project() {
    let mut s = plan(&[("A", 5, ""), ("D", 2, ""), ("M", 0, "")]);
    alap(&mut s, 2);
    alap(&mut s, 3);
    assert_eq!(days(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    assert_eq!((dates(&s, 3).start_at, dates(&s, 3).finish_at), (at("2026-01-09", 17, 0), at("2026-01-09", 17, 0)));
    assert_eq!(finish(&s), date("2026-01-09"));
}

/// A chain: P (1 day) → Q (1 day, ALAP) → R (1 day, ALAP). R takes Friday, Q Thursday; P stays on
/// Monday with two days of slack (it may finish as late as Wednesday, before Q).
#[test]
fn a_chain_of_alap_tasks_packs_against_the_finish() {
    let mut s = plan(&[("A", 5, ""), ("P", 1, ""), ("Q", 1, "2"), ("R", 1, "3")]);
    alap(&mut s, 3);
    alap(&mut s, 4);
    assert_eq!(days(&s, 4), (date("2026-01-09"), date("2026-01-09")));
    assert_eq!(days(&s, 3), (date("2026-01-08"), date("2026-01-08")));
    assert_eq!(days(&s, 2), (date("2026-01-05"), date("2026-01-05")));
    assert_eq!(dates(&s, 2).slack, 2 * 480);
}

/// A deadline caps the late finish: E (1 day, ALAP) with a deadline of Wednesday is on Wednesday,
/// and doesn't miss it.
#[test]
fn a_deadline_holds_it_back() {
    let mut s = plan(&[("A", 5, ""), ("E", 1, "")]);
    alap(&mut s, 2);
    run(&mut s, "task.setDeadline", json!({"id": 2, "date": "2026-01-07"}));
    assert_eq!(days(&s, 2), (date("2026-01-07"), date("2026-01-07")));
    assert!(!dates(&s, 2).deadline_missed);
}

/// A summary rolls up where its subtasks end up: Phase { X (1 day, ALAP), Y (1 day) } spans
/// Monday (Y) to Friday (X), five working days. As Late As Possible on the summary itself holds
/// nothing: its subtasks keep their own dates.
#[test]
fn a_summary_rolls_up_its_alap_subtask() {
    let mut s = plan(&[("A", 5, ""), ("Phase", 1, ""), ("X", 1, ""), ("Y", 1, "")]);
    run(&mut s, "task.indent", json!({"ids": [3, 4]}));
    alap(&mut s, 3);
    assert_eq!(days(&s, 3), (date("2026-01-09"), date("2026-01-09")));
    assert_eq!(days(&s, 2), (date("2026-01-05"), date("2026-01-09")));
    assert_eq!(s.schedule.as_ref().unwrap().rolled_up[&TaskId(2)], 5 * 480);
    run(&mut s, "task.setConstraint", json!({"id": 3, "type": "ASAP"}));
    alap(&mut s, 2);
    assert_eq!(days(&s, 2), (date("2026-01-05"), date("2026-01-05")));
}

/// On a resource calendar: F (2 days, Ann) As Late As Possible, Ann on leave on Friday 9. Her
/// late finish is the project finish, Friday 17:00; walked back on her calendar her two days are
/// Wednesday and Thursday, so F is Wednesday 8:00 to Thursday 17:00.
#[test]
fn on_a_resource_calendar_it_is_walked_back_on_the_resource_s_time() {
    let mut s = plan(&[("A", 5, ""), ("F", 2, "")]);
    run(&mut s, "resource.add", json!({"name": "Ann"}));
    run(&mut s, "assignment.add", json!({"task": 2, "resource": 1}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-09", "resource": 1}));
    alap(&mut s, 2);
    assert_eq!((dates(&s, 2).start_at, dates(&s, 2).finish_at), (at("2026-01-07", 8, 0), at("2026-01-08", 17, 0)));
    assert_eq!(finish(&s), date("2026-01-09"));
}

/// When a predecessor holds it later than its late start (a Must Start On successor leaves no
/// room), the link wins: G (2 days, ALAP) after A finishes on the following Tuesday, past H's
/// Must Start On Monday 12; H keeps its date and G has negative slack, as an As Soon As Possible
/// task would.
#[test]
fn links_still_win_when_there_is_no_room() {
    let mut s = plan(&[("A", 5, ""), ("G", 2, "1"), ("H", 1, "2")]);
    run(&mut s, "task.setConstraint", json!({"id": 3, "type": "MSO", "date": "2026-01-12"}));
    let before = (days(&s, 2), dates(&s, 2).slack);
    alap(&mut s, 2);
    assert_eq!((days(&s, 2), dates(&s, 2).slack), before);
    assert_eq!(days(&s, 2), (date("2026-01-12"), date("2026-01-13")));
    assert!(dates(&s, 2).slack < 0);
}

/// The file keeps it, as `"ALAP"`, and loads it back; a file without a constraint is As Soon As
/// Possible.
#[test]
fn the_file_round_trips_it() {
    let mut s = plan(&[("A", 5, ""), ("B", 1, "")]);
    alap(&mut s, 2);
    let text = s.save_json().unwrap();
    assert!(text.contains("\"ALAP\""), "{text}");
    let back = Session::load_json(&text).unwrap();
    assert_eq!(back.project.task(TaskId(2)).unwrap().constraint, Constraint::AsLateAsPossible);
    assert_eq!(back.project.task(TaskId(1)).unwrap().constraint, Constraint::Asap);
    assert_eq!(days(&back, 2), (date("2026-01-09"), date("2026-01-09")));
}
