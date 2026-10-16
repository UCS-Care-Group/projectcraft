//! Worked examples for the dates a milestone shows. A milestone (duration 0) is a moment, not a
//! day: Microsoft Project shows its Finish as its Start. Where it falls between two working days
//! decides the date shown. A link from a predecessor's finish puts it at the end of that day, so it
//! shows the predecessor's finish date. A start-type constraint (Start No Earlier Than, Start No
//! Later Than, Must Start On) puts it at the start of the constraint date, so it shows that date.
//! A finish-type one (Finish No Earlier/Later Than, Must Finish On) puts it at the end of the date,
//! which shows the same date. Worked out by hand from Microsoft's documentation, before the code:
//!
//! - Constraint Type field (entering a Start date sets Start No Earlier Than that date):
//!   https://support.microsoft.com/en-us/project/constraint-type-task-field
//! - Milestone field (a task of zero duration is a milestone):
//!   https://support.microsoft.com/en-us/project/milestone-task-field
//! - Finish field (the date the task is scheduled to be completed):
//!   https://support.microsoft.com/en-us/project/finish-task-field

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Project, TaskDates, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Project start Monday 2026-01-05. A (3 days, Mon 5 – Wed 7) → M (milestone), FS.
fn a_then_m() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "M", "duration": 0})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s
}

fn dates(s: &Session, id: u32) -> TaskDates {
    s.schedule.as_ref().unwrap().tasks[&TaskId(id)]
}

fn shown(s: &Session, id: u32) -> (Date, Date) {
    let d = dates(s, id);
    (d.start, d.finish)
}

fn on(d: &str) -> (Date, Date) {
    (date(d), date(d))
}

fn constrain(s: &mut Session, id: u32, ty: &str, d: &str) {
    s.run("task.setConstraint", &json!({"id": id, "type": ty, "date": d})).unwrap();
}

/// Worked example M1: a milestone constrained Start No Earlier Than Mon 12 shows Start = Finish =
/// Mon 12, and a task after it starts that day. One driven by a finish-to-start link still shows
/// its predecessor's finish (Wed 7).
#[test]
fn worked_example_m1_snet_milestone_shows_the_constraint_date() {
    let mut s = a_then_m();
    assert_eq!(shown(&s, 2), on("2026-01-07"), "link-driven: the predecessor's finish");

    s.run("task.add", &json!({"name": "M alone", "duration": 0})).unwrap();
    s.run("task.add", &json!({"name": "After", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 4, "text": "3"})).unwrap();
    constrain(&mut s, 3, "SNET", "2026-01-12");
    assert_eq!(shown(&s, 3), on("2026-01-12"));
    assert_eq!(shown(&s, 4), (date("2026-01-12"), date("2026-01-13")), "its successor starts on the day");
    assert_eq!(s.schedule.as_ref().unwrap().finish, date("2026-01-13"));
    // A milestone straight after it, FS, is at the same moment: the same day.
    s.run("task.add", &json!({"name": "M2", "duration": 0})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 5, "text": "3"})).unwrap();
    assert_eq!(shown(&s, 5), on("2026-01-12"));

    // On a weekend: the next working day.
    constrain(&mut s, 3, "SNET", "2026-01-10");
    assert_eq!(shown(&s, 3), on("2026-01-12"));
}

/// Worked example M2: SNET against a link. The link puts M at the end of Wed 7. SNET Thu 8 is the
/// start of the next working day, later on the clock, so it wins and M shows Thu 8. SNET Wed 7
/// is earlier than the link: the link wins and M stays on Wed 7. SNET as the project's last
/// moment sets the project finish.
#[test]
fn worked_example_m2_snet_milestone_against_a_link() {
    let mut s = a_then_m();
    constrain(&mut s, 2, "SNET", "2026-01-08");
    assert_eq!(shown(&s, 2), on("2026-01-08"));
    assert_eq!(s.schedule.as_ref().unwrap().finish, date("2026-01-08"));
    assert_eq!(dates(&s, 2).slack, 0);
    assert_eq!((dates(&s, 2).late_start, dates(&s, 2).late_finish), on("2026-01-08"), "late dates as the early ones");

    constrain(&mut s, 2, "SNET", "2026-01-07");
    assert_eq!(shown(&s, 2), on("2026-01-07"));
}

/// Worked example M3: the hard start types. Must Start On Mon 12 shows Mon 12, early and late.
/// Start No Later Than Tue 6, earlier than the link allows, holds M at Tue 6 (a conflict); later
/// than the link, the link wins.
#[test]
fn worked_example_m3_mso_and_snlt_milestones() {
    let mut s = a_then_m();
    constrain(&mut s, 2, "MSO", "2026-01-12");
    let d = dates(&s, 2);
    assert_eq!((d.start, d.finish, d.late_start, d.late_finish), (date("2026-01-12"), date("2026-01-12"), date("2026-01-12"), date("2026-01-12")));

    constrain(&mut s, 2, "SNLT", "2026-01-06");
    assert_eq!(shown(&s, 2), on("2026-01-06"));
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!(dates(&s, 2).late_start, date("2026-01-06"));

    constrain(&mut s, 2, "SNLT", "2026-01-12");
    assert_eq!(shown(&s, 2), on("2026-01-07"));
}

/// Worked example M4: the finish types put a milestone at the end of the date, so it shows the
/// date, and a task after it starts the next working day.
#[test]
fn worked_example_m4_finish_type_milestones() {
    let mut s = a_then_m();
    s.run("task.add", &json!({"name": "After", "duration": 1})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    constrain(&mut s, 2, "FNET", "2026-01-09");
    assert_eq!(shown(&s, 2), on("2026-01-09"));
    assert_eq!(shown(&s, 3), on("2026-01-12"));

    constrain(&mut s, 2, "MFO", "2026-01-09");
    assert_eq!(shown(&s, 2), on("2026-01-09"));

    constrain(&mut s, 2, "FNLT", "2026-01-06");
    assert_eq!(shown(&s, 2), on("2026-01-06"));
    assert!(dates(&s, 2).constraint_conflict);
}

/// Worked example M5: a deadline on an SNET milestone. On the constraint date, met; the working
/// day before, missed.
#[test]
fn worked_example_m5_deadline_on_an_snet_milestone() {
    let mut s = a_then_m();
    constrain(&mut s, 2, "SNET", "2026-01-12");
    s.run("task.setDeadline", &json!({"id": 2, "date": "2026-01-12"})).unwrap();
    assert!(!dates(&s, 2).deadline_missed);
    s.run("task.setDeadline", &json!({"id": 2, "date": "2026-01-09"})).unwrap();
    assert!(dates(&s, 2).deadline_missed);
}

/// Worked example M6: roll-up. Phase { A (Mon 5 – Wed 7); M SNET Thu 8 } runs Mon 5 – Thu 8.
/// A summary over only an SNET milestone shows its date.
#[test]
fn worked_example_m6_summary_rolls_up_an_snet_milestone() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (n, d) in [("Phase", 1), ("A", 3), ("M", 0), ("Gates", 1), ("G", 0)] {
        s.run("task.add", &json!({"name": n, "duration": d})).unwrap();
    }
    s.run("task.indent", &json!({"ids": [2, 3]})).unwrap();
    s.run("task.indent", &json!({"ids": [5]})).unwrap();
    constrain(&mut s, 3, "SNET", "2026-01-08");
    constrain(&mut s, 5, "SNET", "2026-01-12");
    assert_eq!(shown(&s, 3), on("2026-01-08"));
    assert_eq!(shown(&s, 1), (date("2026-01-05"), date("2026-01-08")));
    assert_eq!(shown(&s, 4), on("2026-01-12"));
    assert_eq!(s.schedule.as_ref().unwrap().finish, date("2026-01-12"));
}
