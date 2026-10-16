//! Worked examples for soft
//! constraints (Start No Earlier Than, Finish No Earlier Than) and typed dates. Each is worked out
//! by hand from the Microsoft documentation it cites, before the code. The hard types are in
//! `hard_constraints.rs`.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Constraint, Date, Project, TaskDates, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Project start Monday 2026-01-05. A (3 days) → B (2 days), FS. A runs Mon 5 – Wed 7, so B is
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

/// Worked example 5: Start No Earlier Than later than the predecessor's finish wins; earlier, the
/// link wins (ES = max(link-driven ES, constraint date)).
/// Source: "Start No Earlier Than (SNET): Schedules the task to start on or after a specified
/// date", https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints
#[test]
fn worked_example_05_snet_later_than_the_predecessor_finish_wins() {
    let mut s = a_then_b();
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));

    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-12"})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::StartNoEarlierThan(date("2026-01-12")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-13")));
    // B now drives the finish; A has the gap as slack (Thu 8, Fri 9: 2 days).
    assert!(dates(&s, 2).critical);
    assert_eq!(dates(&s, 1).slack, 2);
    assert_eq!(s.schedule.as_ref().unwrap().finish, date("2026-01-13"));

    // Earlier than the link allows: the link wins.
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-06"})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));

    // Finish No Earlier Than: EF = max(link-driven EF, constraint date).
    s.run("task.setConstraint", &json!({"id": 2, "type": "FNET", "date": "2026-01-14"})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-13"), date("2026-01-14")));
}

/// Worked example 11: setting As Soon As Possible clears the constraint, and the task goes back
/// to link-driven dates. Undo brings the constraint back as one step.
/// Source: "As Soon As Possible (ASAP): Schedules the task as early as it can, based on other
/// scheduling parameters. ... No constraint date", https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints
#[test]
fn worked_example_11_clearing_to_asap_returns_to_link_driven_dates() {
    let mut s = a_then_b();
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-19"})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-19"), date("2026-01-20")));

    // A date given with ASAP is ignored: ASAP has none.
    s.run("task.setConstraint", &json!({"id": 2, "type": "ASAP", "date": "2026-01-19"})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::Asap);
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));

    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-19"), date("2026-01-20")));
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));

    // The cleared task follows its predecessor again.
    s.run("task.setDuration", &json!({"id": 1, "duration": 1})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-06"), date("2026-01-07")));
}

/// Worked example 14: a constraint date on a non-working day moves to the next working day for a
/// start type and to the previous working day for a finish type (spec, "Constraints and
/// deadlines"). Constraint dates are scheduled in working time like every other date:
/// https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints,
/// https://support.microsoft.com/en-us/project/set-the-general-working-days-and-times-for-a-project
#[test]
fn worked_example_14_a_constraint_date_on_a_non_working_day_moves_to_a_working_day() {
    let mut p = Project::new("t", date("2026-01-05"));
    p.add_task("C", 2 * 480).unwrap();
    // Monday 2026-01-19 is a holiday.
    p.calendar.exceptions.insert(date("2026-01-19"), false);
    let mut s = Session::new(p);

    // SNET Saturday 10 → Monday 12.
    s.run("task.setConstraint", &json!({"id": 1, "type": "SNET", "date": "2026-01-10"})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-12"), date("2026-01-13")));
    // SNET Sunday 18 → past the Monday 19 holiday → Tuesday 20.
    s.run("task.setConstraint", &json!({"id": 1, "type": "SNET", "date": "2026-01-18"})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-20"), date("2026-01-21")));

    // FNET Sunday 11 → Friday 9.
    s.run("task.setConstraint", &json!({"id": 1, "type": "FNET", "date": "2026-01-11"})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-08"), date("2026-01-09")));
    // FNET on the Monday 19 holiday → Friday 16.
    s.run("task.setConstraint", &json!({"id": 1, "type": "FNET", "date": "2026-01-19"})).unwrap();
    assert_eq!(start_finish(&s, 1), (date("2026-01-15"), date("2026-01-16")));

    // The stored date is what was typed; only the schedule moves it.
    assert_eq!(s.project.task(TaskId(1)).unwrap().constraint, Constraint::FinishNoEarlierThan(date("2026-01-19")));
}

/// Worked example 15: typing a Start sets Start No Earlier Than, typing a Finish sets Finish No
/// Earlier Than, and the dates shown still come from the scheduler. The sheet sends exactly these
/// commands (see `projectcraft_ui_egui`'s `typed_date` and its test).
/// Sources: Start, "If you enter a start date ... Project sets a Start No Earlier Than
/// constraint", https://support.microsoft.com/en-US/project/constraint-type-task-field; Finish sets
/// Finish No Earlier Than, https://support.microsoft.com/en-us/project/finish-task-field (the
/// Constraint Type page says FNLT; we follow the more specific Finish page, as the spec records).
#[test]
fn worked_example_15_typing_a_start_or_finish_sets_snet_or_fnet() {
    let mut s = a_then_b();

    // Typed Start, later than the link allows: B starts on it.
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-14"})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::StartNoEarlierThan(date("2026-01-14")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-14"), date("2026-01-15")));

    // Typed Start, earlier than the link allows: the constraint is kept, but the shown Start is
    // the scheduler's, after A.
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-05"})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::StartNoEarlierThan(date("2026-01-05")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));

    // Typed Finish Friday 16: B finishes then, starting Thursday 15 (duration kept).
    s.run("task.setConstraint", &json!({"id": 2, "type": "FNET", "date": "2026-01-16"})).unwrap();
    assert_eq!(s.project.task(TaskId(2)).unwrap().constraint, Constraint::FinishNoEarlierThan(date("2026-01-16")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-15"), date("2026-01-16")));
    assert_eq!(s.project.task(TaskId(2)).unwrap().duration, 2 * 480);
}

/// Bad params are refused and change nothing.
#[test]
fn bad_constraint_params_change_nothing() {
    let mut s = a_then_b();
    let before = s.project.clone();
    let e = s.run("task.setConstraint", &json!({"id": 2, "type": "SNET"})).unwrap_err().to_string();
    assert!(e.contains("date"), "{e}");
    assert!(s.run("task.setConstraint", &json!({"id": 2, "type": "MSO"})).is_err());
    assert!(s.run("task.setConstraint", &json!({"id": 2, "type": "ALAP", "date": "2026-01-12"})).is_err());
    assert!(s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-02-30"})).is_err());
    assert!(s.run("task.setConstraint", &json!({"id": 9, "type": "SNET", "date": "2026-01-12"})).is_err());
    assert_eq!(s.project, before);
}

/// Dates at the ends of the calendar schedule or fail cleanly; they never panic.
#[test]
fn far_and_early_constraint_dates_never_panic() {
    let mut s = a_then_b();
    // Before the project start: no effect.
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "0001-01-01"})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    for d in ["9999-12-31", "9999-12-30", "0001-01-01"] {
        for ty in ["SNET", "FNET", "SNLT", "FNLT", "MSO", "MFO"] {
            let _ = s.run("task.setConstraint", &json!({"id": 2, "type": ty, "date": d}));
        }
    }
}
