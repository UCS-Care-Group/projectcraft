//! Worked examples for the hard constraint types:
//! Start/Finish No Later Than and Must Start/Finish On win over links, a link that can't be
//! honoured gives negative slack and a warning from every schedule run, and the scheduler records
//! what drove each start. Worked out by hand from Microsoft's definitions, before the code:
//! https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints
//!
//! - Start No Later Than: LS = CD (if CD < LS). Finish No Later Than: LF = CD (if CD < LF).
//! - Must Start On: ES, LS, SS = CD. Must Finish On: EF, LF, SF = CD.
//! - Constraints win over links ("Tasks will always honor their constraint dates", Project's
//!   default): https://learn.microsoft.com/dotnet/api/websvcstatusing.statusingdataset.projectsrow.proj_opt_honor_constraints?view=office-project-server

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Driver, Project, TaskDates, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Project start Monday 2026-01-05. A (3 days, Mon 5 – Wed 7) → B (2 days), FS: B is
/// link-driven to Thu 8 – Fri 9.
fn a_then_b() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "B", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s
}

/// The scheduler's dates, with total slack in working days (it counts working minutes), as
/// these examples count it.
fn dates(s: &Session, id: u32) -> TaskDates {
    let mut d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    let day = s.project.minutes_per_day();
    assert_eq!(d.slack % day, 0, "whole days of slack");
    d.slack /= day;
    d
}

fn start_finish(s: &Session, id: u32) -> (Date, Date) {
    let d = dates(s, id);
    (d.start, d.finish)
}

fn constrain(s: &mut Session, id: u32, ty: &str, d: &str) {
    s.run("task.setConstraint", &json!({"id": id, "type": ty, "date": d})).unwrap();
}

/// Worked example 3: MSO / FNLT on a successor holds when the predecessor slips; negative slack
/// on it and its predecessors.
///
/// Must Start On Thu 8 on B. A slips from 3 to 5 days (Mon 5 – Fri 9). B stays Thu 8 – Fri 9:
/// the constraint wins. MSO sets ES = LS = CD, so B's own slack is 0 (critical); its late start
/// caps A's late finish at Wed 7, two working days before A's finish: A's slack is −2.
///
/// Finish No Later Than Fri 9 on B instead. B is held at Thu 8 – Fri 9 (LF = CD), but its early
/// start stays where the link puts it (Mon 12), so B's slack is LS − ES = Thu 8 − Mon 12 = −2,
/// and A's is −2 too.
///
/// Source: https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints
#[test]
fn worked_example_03_mso_and_fnlt_hold_when_the_predecessor_slips() {
    let mut s = a_then_b();
    constrain(&mut s, 2, "MSO", "2026-01-08");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    assert!(!dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (0, 0));

    s.run("task.setDuration", &json!({"id": 1, "duration": 5})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-09")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")), "the constraint wins");
    assert!(dates(&s, 2).constraint_conflict);
    assert!(!dates(&s, 1).constraint_conflict);
    assert_eq!(dates(&s, 2).slack, 0, "MSO: ES = LS = CD");
    assert_eq!(dates(&s, 1).slack, -2);
    assert!(dates(&s, 1).critical);
    assert_eq!(dates(&s, 1).late_finish, date("2026-01-07"));

    constrain(&mut s, 2, "FNLT", "2026-01-09");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")), "held at its date");
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-2, -2));
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);

    // A successor of the held task follows its held dates: C starts Mon 12.
    s.run("task.add", &json!({"name": "C", "duration": 1})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-12")));
    assert_eq!(dates(&s, 3).driver, Driver::Link(TaskId(2)));
}

/// Start No Later Than: LS = CD if CD < LS. Earlier than the link: B is held at Wed 7 (overlapping
/// A), the warning shows, and slack is −1 on B (LS Wed 7 − ES Thu 8) and on A. Later than the
/// link: no effect on the dates, B is link-driven; the late start is capped at the date.
#[test]
fn snlt_holds_the_start_and_caps_the_late_start() {
    let mut s = a_then_b();
    constrain(&mut s, 2, "SNLT", "2026-01-07");
    assert_eq!(start_finish(&s, 2), (date("2026-01-07"), date("2026-01-08")));
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-1, -1));

    // Later than the link: honoured, nothing moves. A later task D (8 days) leaves B room, but
    // the constraint caps B's late start at Fri 9: slack 1.
    s.run("task.add", &json!({"name": "D", "duration": 8})).unwrap();
    constrain(&mut s, 2, "SNLT", "2026-01-09");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    assert!(!dates(&s, 2).constraint_conflict);
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));
    assert_eq!(dates(&s, 2).late_start, date("2026-01-09"));
    assert_eq!(dates(&s, 2).slack, 1);
}

/// Finish No Later Than: LF = CD if CD < LF. On a Sunday it counts as the Friday before.
#[test]
fn fnlt_holds_the_finish_and_caps_the_late_finish() {
    let mut s = a_then_b();
    s.run("task.add", &json!({"name": "D", "duration": 10})).unwrap();
    constrain(&mut s, 2, "FNLT", "2026-01-11");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    assert!(!dates(&s, 2).constraint_conflict);
    assert_eq!(dates(&s, 2).late_finish, date("2026-01-09"));
    assert_eq!(dates(&s, 2).slack, 0);
    // Earlier than the link: held, finishing Thu 8.
    constrain(&mut s, 2, "FNLT", "2026-01-08");
    assert_eq!(start_finish(&s, 2), (date("2026-01-07"), date("2026-01-08")));
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-1, -1));
}

/// Must Start On: ES, LS, SS = CD, whether later or earlier than the links allow.
#[test]
fn mso_fixes_the_start_both_ways() {
    let mut s = a_then_b();
    // Later: B Mon 12 – Tue 13, A has Thu 8, Fri 9 as slack.
    constrain(&mut s, 2, "MSO", "2026-01-12");
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-13")));
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);
    assert!(!dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (2, 0));
    // Earlier: B Tue 6 – Wed 7, A must finish Mon 5: −2.
    constrain(&mut s, 2, "MSO", "2026-01-06");
    assert_eq!(start_finish(&s, 2), (date("2026-01-06"), date("2026-01-07")));
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-2, 0));
    // Before the project start: held at the project start.
    constrain(&mut s, 2, "MSO", "2025-12-01");
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-06")));
}

/// Must Finish On: EF, LF, SF = CD. On a Saturday it counts as the Friday before.
#[test]
fn mfo_fixes_the_finish_both_ways() {
    let mut s = a_then_b();
    constrain(&mut s, 2, "MFO", "2026-01-14");
    assert_eq!(start_finish(&s, 2), (date("2026-01-13"), date("2026-01-14")));
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);
    assert!(!dates(&s, 2).constraint_conflict);
    constrain(&mut s, 2, "MFO", "2026-01-10");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    assert!(!dates(&s, 2).constraint_conflict);
    constrain(&mut s, 2, "MFO", "2026-01-08");
    assert_eq!(start_finish(&s, 2), (date("2026-01-07"), date("2026-01-08")));
    assert!(dates(&s, 2).constraint_conflict);
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-1, 0));
}

/// The warning comes from every schedule run, not only from the edit that set the constraint:
/// a predecessor two links back slipping into it raises it, and shortening again clears it
/// (Bryntum warns only on the conflicting edit; spec, "Lessons from other planners").
#[test]
fn a_predecessor_slipping_into_a_constraint_raises_the_warning() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, d) in [("X", 1), ("A", 2), ("B", 2)] {
        s.run("task.add", &json!({"name": name, "duration": d})).unwrap();
    }
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    // X Mon 5, A Tue 6 – Wed 7, B Must Start On Thu 8.
    constrain(&mut s, 3, "MSO", "2026-01-08");
    assert!(!dates(&s, 3).constraint_conflict);

    // Only X changes.
    s.run("task.setDuration", &json!({"id": 1, "duration": 3})).unwrap();
    assert!(dates(&s, 3).constraint_conflict);
    assert_eq!(start_finish(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    assert_eq!((dates(&s, 1).slack, dates(&s, 2).slack), (-2, -2), "negative slack runs back up the chain");

    s.run("edit.undo", &json!({})).unwrap();
    assert!(!dates(&s, 3).constraint_conflict);
    assert_eq!(dates(&s, 1).slack, 0);
}

/// The driver of each start: the project start, a link (the one that sets the date, the first
/// listed on a tie), or a constraint.
#[test]
fn the_driver_of_each_start_is_recorded() {
    let mut s = a_then_b();
    s.run("task.add", &json!({"name": "C", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "D", "duration": 1})).unwrap();
    assert_eq!(dates(&s, 1).driver, Driver::ProjectStart);
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));

    // D after B (finishes Fri 9) and C (finishes Wed 7): B drives.
    s.run("task.setPredecessors", &json!({"id": 4, "text": "3, 2"})).unwrap();
    assert_eq!(dates(&s, 4).driver, Driver::Link(TaskId(2)));
    // A tie (both finish Wed 7): the first listed.
    s.run("task.setPredecessors", &json!({"id": 4, "text": "3, 1"})).unwrap();
    assert_eq!(dates(&s, 4).driver, Driver::Link(TaskId(3)));
    // A lead that would start before the project start: the project start drives.
    s.run("task.setPredecessors", &json!({"id": 4, "text": "1SS-2"})).unwrap();
    assert_eq!(dates(&s, 4).driver, Driver::ProjectStart);

    // SNET later than the link drives; earlier, the link still does.
    constrain(&mut s, 2, "SNET", "2026-01-12");
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);
    constrain(&mut s, 2, "SNET", "2026-01-06");
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));
    // SNLT later than the link doesn't hold it: the link drives.
    constrain(&mut s, 2, "SNLT", "2026-01-20");
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));
    // On a task with no links: SNET on the project start itself leaves the project start driving.
    constrain(&mut s, 1, "SNET", "2026-01-05");
    assert_eq!(dates(&s, 1).driver, Driver::ProjectStart);
    constrain(&mut s, 1, "MSO", "2026-01-05");
    assert_eq!(dates(&s, 1).driver, Driver::Constraint);
}

/// Hard constraints save and load with their dates, and the loaded plan schedules the same.
#[test]
fn hard_constraints_round_trip_through_the_file() {
    let mut s = a_then_b();
    s.run("task.setDuration", &json!({"id": 1, "duration": 5})).unwrap();
    constrain(&mut s, 2, "MFO", "2026-01-09");
    let text = s.save_json().unwrap();
    assert!(text.contains("\"MFO\": \"2026-01-09\""), "{text}");
    let back = Session::load_json(&text).unwrap();
    assert_eq!(back.project, s.project);
    assert_eq!(dates(&back, 2), dates(&s, 2));
    assert!(dates(&back, 2).constraint_conflict);
}
