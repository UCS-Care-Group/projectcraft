//! Links and constraints on summary tasks, through the commands. A summary's predecessor or
//! constraint holds all its subtasks: none starts before the date the summary's link or constraint
//! allows, through every level of nesting. A link between a summary and one of its own subtasks is
//! refused, and so is an outline change that would make one.
//!
//! Sources:
//! - Summary links are allowed (and discouraged), and outdenting a linked subtask makes a linked
//!   summary: https://learn.microsoft.com/en-us/archive/technet-wiki/29569.ms-project-linking-andor-assigning-resources-to-summary-tasks
//! - Circular relationships through the outline are refused:
//!   https://learn.microsoft.com/en-us/office/troubleshoot/project/circular-relationship-error-when-indenting-task
//! - Constraint definitions: https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints

use projectcraft_engine::Session;
use projectcraft_engine::model::{CIRCULAR_OUTLINE, Date, Driver, Project, TaskDates, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

fn dates(s: &Session, id: u32) -> TaskDates {
    s.schedule.as_ref().unwrap().tasks[&TaskId(id)]
}

fn start_finish(s: &Session, id: u32) -> (Date, Date) {
    let d = dates(s, id);
    (d.start, d.finish)
}

fn preds(s: &mut Session, id: u32, text: &str) -> Result<(), String> {
    s.run("task.setPredecessors", &json!({"id": id, "text": text})).map(|_| ()).map_err(|e| e.to_string())
}

/// Project start Monday 2026-01-05. Rows: 1 P (5 days, Mon 5 – Fri 9); 2 Phase { 3 A (3 days);
/// 4 B (2 days, after A) }. Phase follows P, finish-to-start.
fn p_then_phase() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("P", 5), ("Phase", 1), ("A", 3), ("B", 2)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    s.run("task.indent", &json!({"ids": [3, 4]})).unwrap();
    preds(&mut s, 4, "3").unwrap();
    preds(&mut s, 2, "1").unwrap();
    s
}

/// Worked example 1: a summary as successor moves earlier when its predecessor shortens
/// (GanttProject #2002 kept the summary where it was).
///
/// P Mon 5 – Fri 9 → Phase: A Mon 12 – Wed 14, B Thu 15 – Fri 16, Phase Mon 12 – Fri 16.
/// P shortens to 2 days (Mon 5 – Tue 6): A Wed 7 – Fri 9, B Mon 12 – Tue 13, Phase Wed 7 – Tue 13.
#[test]
fn worked_example_01_a_summary_successor_moves_earlier_when_its_predecessor_shortens() {
    let mut s = p_then_phase();
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-14")));
    assert_eq!(start_finish(&s, 4), (date("2026-01-15"), date("2026-01-16")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-16")));
    s.run("task.setDuration", &json!({"id": 1, "duration": 2})).unwrap();
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-09")), "earlier, not left where it was");
    assert_eq!(start_finish(&s, 4), (date("2026-01-12"), date("2026-01-13")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-07"), date("2026-01-13")));
    // Every task is on the one chain: all critical, and P's late dates come through Phase's subtasks.
    for id in 1..=4 {
        assert_eq!(dates(&s, id).slack, 0, "task {id}");
    }
}

/// Worked example 2: a subtask of a linked or Start No Earlier Than summary starts no earlier than
/// the summary's driven start, and the scheduler records the summary as what drove it.
#[test]
fn worked_example_02_a_subtask_starts_no_earlier_than_its_summarys_driven_start() {
    // Linked: A is held by Phase's link from P; B is still driven by its own link from A.
    let mut s = p_then_phase();
    assert_eq!(dates(&s, 3).driver, Driver::ParentSummary(TaskId(2)));
    assert_eq!(dates(&s, 4).driver, Driver::Link(TaskId(3)));
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));

    // Start No Earlier Than Wed 14 on Phase, unlinked: A Wed 14 – Fri 16, B Mon 19 – Tue 20.
    preds(&mut s, 2, "").unwrap();
    s.run("task.setConstraint", &json!({"id": 2, "type": "SNET", "date": "2026-01-14"})).unwrap();
    assert_eq!(start_finish(&s, 3), (date("2026-01-14"), date("2026-01-16")));
    assert_eq!(start_finish(&s, 4), (date("2026-01-19"), date("2026-01-20")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-14"), date("2026-01-20")));
    assert_eq!(dates(&s, 3).driver, Driver::ParentSummary(TaskId(2)));
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);

    // A subtask's own later constraint wins over the hold: SNET Mon 19 on A.
    s.run("task.setConstraint", &json!({"id": 3, "type": "SNET", "date": "2026-01-19"})).unwrap();
    assert_eq!(start_finish(&s, 3), (date("2026-01-19"), date("2026-01-21")));
    assert_eq!(dates(&s, 3).driver, Driver::Constraint);

    // Back to As Soon As Possible on Phase: A follows its own SNET, B follows A.
    s.run("task.setConstraint", &json!({"id": 3, "type": "ASAP"})).unwrap();
    s.run("task.setConstraint", &json!({"id": 2, "type": "ASAP"})).unwrap();
    assert_eq!(start_finish(&s, 3), (date("2026-01-05"), date("2026-01-07")));
    assert_eq!(dates(&s, 3).driver, Driver::ProjectStart);
}

/// The hold goes through every level of nesting: Outer { Inner { X } } with Outer after P puts X
/// after P; Inner's own later SNET then holds X later still.
#[test]
fn the_hold_goes_through_nested_summaries() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("P", 5), ("Outer", 1), ("Inner", 1), ("X", 2), ("Y", 1)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    s.run("task.indent", &json!({"ids": [3, 4, 5]})).unwrap();
    s.run("task.indent", &json!({"ids": [4]})).unwrap();
    preds(&mut s, 2, "1").unwrap();
    // X (inside Inner) and Y (directly inside Outer) both start Mon 12, after P.
    assert_eq!(start_finish(&s, 4), (date("2026-01-12"), date("2026-01-13")));
    assert_eq!(start_finish(&s, 5), (date("2026-01-12"), date("2026-01-12")));
    assert_eq!(dates(&s, 4).driver, Driver::ParentSummary(TaskId(2)), "named after the summary whose link holds it");
    assert_eq!(dates(&s, 3).driver, Driver::ParentSummary(TaskId(2)));
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-13")));
    s.run("task.setConstraint", &json!({"id": 3, "type": "SNET", "date": "2026-01-14"})).unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-14"), date("2026-01-15")));
    assert_eq!(dates(&s, 4).driver, Driver::ParentSummary(TaskId(3)));
    assert_eq!(start_finish(&s, 5), (date("2026-01-12"), date("2026-01-12")), "Y is outside Inner");
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-15")));
    // A link from Inner's subtask to Outer, an ancestor two levels up, is a circular relationship.
    assert_eq!(preds(&mut s, 2, "1, 4").unwrap_err(), CIRCULAR_OUTLINE);
    assert_eq!(preds(&mut s, 4, "2").unwrap_err(), CIRCULAR_OUTLINE);
}

/// Worked example 9: a link between a summary and any of its own subtasks is refused, either way
/// round, and the refusal changes nothing.
#[test]
fn worked_example_09_a_link_between_a_summary_and_its_own_subtask_is_refused() {
    let mut s = p_then_phase();
    let before = s.project.clone();
    assert_eq!(preds(&mut s, 2, "1, 3").unwrap_err(), CIRCULAR_OUTLINE, "the summary after its subtask");
    assert_eq!(preds(&mut s, 3, "2").unwrap_err(), CIRCULAR_OUTLINE, "the subtask after its summary");
    assert_eq!(preds(&mut s, 4, "3, 2SS").unwrap_err(), CIRCULAR_OUTLINE);
    assert_eq!(s.project, before);
    // A link to a task outside the summary is fine either way round.
    s.run("task.add", &json!({"name": "C", "duration": 1})).unwrap();
    preds(&mut s, 5, "2").unwrap();
    assert_eq!(start_finish(&s, 5), (date("2026-01-19"), date("2026-01-19")), "after Phase's finish, Fri 16");
}

/// Worked example 10: indenting a task under its own linked predecessor is refused, now that a
/// summary may otherwise keep its links.
#[test]
fn worked_example_10_indenting_under_a_linked_predecessor_is_refused() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for name in ["A", "B", "C"] {
        s.run("task.add", &json!({"name": name, "duration": 2})).unwrap();
    }
    preds(&mut s, 2, "1").unwrap();
    let before = s.project.clone();
    assert_eq!(s.run("task.indent", &json!({"ids": [2]})).unwrap_err().to_string(), CIRCULAR_OUTLINE);
    assert_eq!(s.project, before);
    // C follows B; making B a summary over C is the same loop the other way round.
    preds(&mut s, 3, "2").unwrap();
    assert_eq!(s.run("task.indent", &json!({"ids": [3]})).unwrap_err().to_string(), CIRCULAR_OUTLINE);
}

/// A task with links or a constraint may become a summary: its link and constraint then hold its
/// new subtasks (outdenting a linked subtask makes a linked summary, as in Project).
#[test]
fn a_linked_or_constrained_task_can_become_a_summary_and_holds_its_subtasks() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("P", 5), ("B", 2), ("C", 3)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    preds(&mut s, 2, "1").unwrap();
    s.run("task.indent", &json!({"ids": [3]})).unwrap();
    assert!(s.project.is_summary(TaskId(2)));
    // C, unlinked itself, starts after P because its summary B follows P.
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-14")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-14")));
    s.run("edit.undo", &json!({})).unwrap();
    // A constrained task too.
    s.run("task.setConstraint", &json!({"id": 3, "type": "SNET", "date": "2026-01-07"})).unwrap();
    s.run("task.add", &json!({"name": "D", "duration": 1})).unwrap();
    s.run("task.indent", &json!({"ids": [4]})).unwrap();
    assert!(s.project.is_summary(TaskId(3)));
    assert_eq!(start_finish(&s, 4), (date("2026-01-07"), date("2026-01-07")));
}

/// Every constraint type on a summary: the No Later Than and Must types win over the summary's
/// link and show a warning; Finish types and finish-to-finish links use the summary's rolled-up
/// duration.
#[test]
fn every_constraint_type_and_link_kind_works_on_a_summary() {
    let mut s = p_then_phase();
    let constrain = |s: &mut Session, ty: &str, d: &str| {
        s.run("task.setConstraint", &json!({"id": 2, "type": ty, "date": d})).unwrap();
    };
    // Must Start On Mon 5 beats the link from P (Mon 12): the subtasks start Mon 5, Phase warns.
    constrain(&mut s, "MSO", "2026-01-05");
    assert_eq!(start_finish(&s, 3), (date("2026-01-05"), date("2026-01-07")));
    assert!(dates(&s, 2).constraint_conflict);
    assert!(dates(&s, 1).slack < 0, "P is held back by Phase's constraint: negative slack");
    // Start No Later Than Wed 7: held at Wed 7, a warning.
    constrain(&mut s, "SNLT", "2026-01-07");
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-09")));
    assert!(dates(&s, 2).constraint_conflict);
    // Finish No Earlier Than Fri 23: Phase (5 days) finishes Fri 23, so it starts Mon 19.
    constrain(&mut s, "FNET", "2026-01-23");
    assert_eq!(start_finish(&s, 2), (date("2026-01-19"), date("2026-01-23")));
    assert!(!dates(&s, 2).constraint_conflict);
    // Must Finish On Fri 16: the link's own date, no conflict.
    constrain(&mut s, "MFO", "2026-01-16");
    assert_eq!(start_finish(&s, 2), (date("2026-01-12"), date("2026-01-16")));
    assert!(!dates(&s, 2).constraint_conflict);
    // Finish No Later Than Wed 14, earlier than the link allows: held, warned, negative slack.
    constrain(&mut s, "FNLT", "2026-01-14");
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-14")));
    assert!(dates(&s, 2).constraint_conflict);
    assert!(dates(&s, 1).slack < 0);
    constrain(&mut s, "ASAP", "2026-01-14");
    // Finish-to-finish from P (Fri 9): Phase (5 days) finishes Fri 9, so starts Mon 5.
    preds(&mut s, 2, "1FF").unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-09")));
    // Start-to-start plus 2: Wed 7.
    preds(&mut s, 2, "1SS+2").unwrap();
    assert_eq!(start_finish(&s, 3).0, date("2026-01-07"));
}

/// A file holding a link between a summary and its own subtask (only a damaged or hand-edited one
/// could) loads, and the scheduler reports the loop instead of spinning.
#[test]
fn a_loaded_summary_to_subtask_link_is_reported_as_a_loop() {
    let damaged = json!({"name": "x", "start": "2026-01-05", "tasks": [
        {"id": 1, "name": "Phase", "duration": 1, "outline_level": 1, "preds": [{"pred": 3}]},
        {"id": 2, "name": "Inner", "duration": 1, "outline_level": 2},
        {"id": 3, "name": "X", "duration": 1, "outline_level": 3},
    ]});
    let s = Session::load_json(&damaged.to_string()).unwrap();
    assert!(s.schedule.is_err());
}

/// Rows: 1 X (10 days, Mon 5 – Fri 16); 2 Phase { 3 A (2 days, Must Start On Mon 5); 4 B (3 days,
/// after A) }. A is fixed, so Phase's span depends on where its hold puts B: Mon 5 – Fri 9 at the
/// earliest, longer the later B starts. A finish-type hold has to land on the date, not on an
/// estimate of the span taken without it.
fn fixed_then_chain() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    for (name, duration) in [("X", 10), ("Phase", 1), ("A", 2), ("B", 3)] {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
    }
    s.run("task.indent", &json!({"ids": [3, 4]})).unwrap();
    s.run("task.setConstraint", &json!({"id": 3, "type": "MSO", "date": "2026-01-05"})).unwrap();
    preds(&mut s, 4, "3").unwrap();
    s
}

/// Finish No Earlier Than on a summary: "the task finishes on or after the date"; otherwise as soon
/// as possible. Phase FNET Fri 16: B (after A, Mon 5 – Tue 6) is held to Wed 14 – Fri 16, so Phase
/// finishes exactly Fri 16 (a span estimate without the hold would leave it on Fri 9).
#[test]
fn a_finish_no_earlier_than_summary_finishes_on_its_date_when_its_subtasks_link_inside_it() {
    let mut s = fixed_then_chain();
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-09")), "unconstrained: A then B");
    s.run("task.setConstraint", &json!({"id": 2, "type": "FNET", "date": "2026-01-16"})).unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-14"), date("2026-01-16")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-16")));
    assert_eq!(dates(&s, 4).driver, Driver::ParentSummary(TaskId(2)));
    assert_eq!(dates(&s, 2).driver, Driver::Constraint);
    // A date the subtasks already reach is no hold: back to Mon 5 – Fri 9.
    s.run("task.setConstraint", &json!({"id": 2, "type": "FNET", "date": "2026-01-07"})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-09")));
    assert_eq!(start_finish(&s, 4), (date("2026-01-07"), date("2026-01-09")));
}

/// Finish-to-finish into a summary: "the dependent task can't finish until the predecessor
/// finishes". X FF Phase: Phase finishes with X, Fri 16, and no later.
#[test]
fn a_finish_to_finish_predecessor_into_a_summary_lands_on_its_finish() {
    let mut s = fixed_then_chain();
    preds(&mut s, 2, "1FF").unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-14"), date("2026-01-16")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-16")));
    assert_eq!(dates(&s, 2).driver, Driver::Link(TaskId(1)));
    // With lag: FF+1 finishes Mon 19.
    preds(&mut s, 2, "1FF+1").unwrap();
    assert_eq!(start_finish(&s, 2).1, date("2026-01-19"));
    // Start-to-finish from X (starts Mon 5) plus 7: Phase finishes no earlier than Tue 13.
    preds(&mut s, 2, "1SF+7").unwrap();
    assert_eq!(start_finish(&s, 2).1, date("2026-01-13"));
}

/// Must Finish On a summary: "the task must finish on the date". Phase MFO Fri 16 finishes Fri 16;
/// Finish No Later Than Tue 13 with a link from a predecessor that would take it to Wed 14 holds
/// it at Tue 13, with a warning.
#[test]
fn must_finish_on_and_finish_no_later_than_a_summary_land_on_their_dates() {
    let mut s = fixed_then_chain();
    s.run("task.setConstraint", &json!({"id": 2, "type": "MFO", "date": "2026-01-16"})).unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-14"), date("2026-01-16")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-16")));
    assert!(!dates(&s, 2).constraint_conflict);
    // P (5 days, Mon 5 – Fri 9) → Phase puts B Mon 12 – Wed 14; FNLT Tue 13 holds it a day earlier.
    s.run("task.add", &json!({"name": "P", "duration": 5})).unwrap();
    preds(&mut s, 2, "5").unwrap();
    s.run("task.setConstraint", &json!({"id": 2, "type": "FNLT", "date": "2026-01-13"})).unwrap();
    assert_eq!(start_finish(&s, 4), (date("2026-01-09"), date("2026-01-13")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-05"), date("2026-01-13")));
    assert!(dates(&s, 2).constraint_conflict);
}
