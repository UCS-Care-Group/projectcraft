//! Worked examples: task types and effort-driven scheduling.
//!
//! Sources: Microsoft's "Change the task type for more accurate scheduling"
//! (https://support.microsoft.com/en-us/project/change-the-task-type-for-more-accurate-scheduling):
//! Work = Duration × Units, and the task type says which stays when another changes:
//!
//! | Type | Units edited | Duration edited | Work edited |
//! |---|---|---|---|
//! | Fixed Units | duration recalculated | work recalculated | duration recalculated |
//! | Fixed Work | duration recalculated | units recalculated | duration recalculated |
//! | Fixed Duration | work recalculated | work recalculated | units recalculated |
//!
//! Its examples start from one full-time person on a 10-day task: 80 hours of work. Effort-driven
//! scheduling, from "Change the effort driven setting for task types"
//! (https://support.microsoft.com/en-us/office/change-the-effort-driven-setting-for-task-types-18efd7c7-d146-4b06-bdbe-b6a11564bdf3)
//! and the Effort Driven field
//! (https://support.microsoft.com/en-us/office/effort-driven-task-field-2155647a-a232-48a6-a832-ed64fa87dc71):
//! the total work stays as resources are added or removed, after the first assignment; a Fixed
//! Units task's duration changes, a Fixed Duration task's units do; Fixed Work is always
//! effort-driven; summaries never are. Removing the only resource from a Fixed Units or Fixed
//! Duration task takes its work away ("Remove or replace a resource assignment",
//! https://support.microsoft.com/en-us/project/remove-or-replace-a-resource-assignment).
//!
//! The default calendar works Monday to Friday, 8:00–12:00 and 13:00–17:00, 8 hours a day. The
//! project starts on Monday 5 January 2026. Every number is worked out by hand first.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Moment, Project, ResourceId, TaskId, TaskType, Units};
use serde_json::json;

/// Working minutes in an hour and in a day.
const H: i64 = 60;
const D: i64 = 480;
const DRAFT: TaskId = TaskId(1);
const ANN: ResourceId = ResourceId(1);
const BOB: ResourceId = ResourceId(2);

/// A 10-day task "Draft" of type `ty`, effort-driven or not, with four work resources (Ann, Bob,
/// Cat, Dan, Unique IDs 1–4) and Ann on it full time: 80 hours of work.
fn draft(ty: &str, effort_driven: bool) -> Session {
    let mut s = Session::new(Project::new("t", "2026-01-05".parse().unwrap()));
    s.run("task.add", &json!({"name": "Draft", "duration": 10})).unwrap();
    for n in ["Ann", "Bob", "Cat", "Dan"] {
        s.run("resource.add", &json!({"name": n})).unwrap();
    }
    s.run("task.setType", &json!({"id": 1, "type": ty})).unwrap();
    if ty != "Fixed Work" {
        s.run("task.setType", &json!({"id": 1, "effortDriven": effort_driven})).unwrap();
    }
    s.run("assignment.add", &json!({"task": 1, "resource": 1})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (10 * D, 80 * H, 100.0));
    s
}

fn duration(s: &Session) -> i64 {
    s.project.task(DRAFT).unwrap().duration
}

fn work(s: &Session) -> i64 {
    s.project.task_work(DRAFT)
}

/// A resource's units on Draft, in percent.
fn units(s: &Session, r: ResourceId) -> f64 {
    match s.project.assignments_of(DRAFT).find(|a| a.resource == r).unwrap().units {
        Units::Work(u) => f64::from(u) / 100.0,
        other => panic!("{other:?}"),
    }
}

/// A resource's work on Draft, in working minutes.
fn work_of(s: &Session, r: ResourceId) -> i64 {
    s.project.assignments_of(DRAFT).find(|a| a.resource == r).unwrap().work
}

fn finish(s: &Session) -> Moment {
    s.schedule.as_ref().unwrap().tasks[&DRAFT].finish_at
}

fn at(d: &str, h: u16) -> Moment {
    Moment { date: d.parse::<Date>().unwrap(), minute: h * 60 }
}

/// Fixed Units, units edited: the duration is recalculated. Ann at 200% does her 80 hours in
/// 5 days, finishing Friday 9 January.
#[test]
fn worked_example_tt1_fixed_units_units_edited_recalculates_duration() {
    let mut s = draft("Fixed Units", false);
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "200%"})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (5 * D, 80 * H, 200.0));
    assert_eq!(finish(&s), at("2026-01-09", 17));
}

/// Fixed Units, duration edited: the work is recalculated. 8 days at 100% is 64 hours.
#[test]
fn worked_example_tt2_fixed_units_duration_edited_recalculates_work() {
    let mut s = draft("Fixed Units", false);
    s.run("task.setDuration", &json!({"id": 1, "duration": 8})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (8 * D, 64 * H, 100.0));
    assert_eq!(finish(&s), at("2026-01-14", 17));
}

/// Fixed Units, work edited: the duration is recalculated. 100 hours at 100% is 12.5 days,
/// finishing at noon on Wednesday 21 January.
#[test]
fn worked_example_tt3_fixed_units_work_edited_recalculates_duration() {
    let mut s = draft("Fixed Units", false);
    s.run("task.setWork", &json!({"id": 1, "work": 100})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (12 * D + 4 * H, 100 * H, 100.0));
    assert_eq!(finish(&s), at("2026-01-21", 12));
}

/// Fixed Work, units edited: the duration is recalculated. 80 hours at 200% is 5 days.
#[test]
fn worked_example_tt4_fixed_work_units_edited_recalculates_duration() {
    let mut s = draft("Fixed Work", true);
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "200%"})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (5 * D, 80 * H, 200.0));
}

/// Fixed Work, duration edited: the units are recalculated. 80 hours in 8 days needs 125%.
#[test]
fn worked_example_tt5_fixed_work_duration_edited_recalculates_units() {
    let mut s = draft("Fixed Work", true);
    s.run("task.setDuration", &json!({"id": 1, "duration": 8})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (8 * D, 80 * H, 125.0));
    assert_eq!(finish(&s), at("2026-01-14", 17));
}

/// Fixed Work, work edited: the duration is recalculated. 100 hours at 100% is 12.5 days.
#[test]
fn worked_example_tt6_fixed_work_work_edited_recalculates_duration() {
    let mut s = draft("Fixed Work", true);
    s.run("task.setWork", &json!({"id": 1, "work": "100h"})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (12 * D + 4 * H, 100 * H, 100.0));
}

/// Fixed Duration, units edited: the work is recalculated. 10 days at 50% is 40 hours.
#[test]
fn worked_example_tt7_fixed_duration_units_edited_recalculates_work() {
    let mut s = draft("Fixed Duration", false);
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "50%"})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (10 * D, 40 * H, 50.0));
    assert_eq!(finish(&s), at("2026-01-16", 17));
}

/// Fixed Duration, duration edited: the work is recalculated. 8 days at 100% is 64 hours.
#[test]
fn worked_example_tt8_fixed_duration_duration_edited_recalculates_work() {
    let mut s = draft("Fixed Duration", false);
    s.run("task.setDuration", &json!({"id": 1, "duration": 8})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (8 * D, 64 * H, 100.0));
}

/// Fixed Duration, work edited: the units are recalculated. 100 hours in 10 days needs 125%.
#[test]
fn worked_example_tt9_fixed_duration_work_edited_recalculates_units() {
    let mut s = draft("Fixed Duration", false);
    s.run("task.setWork", &json!({"id": 1, "work": 100})).unwrap();
    assert_eq!((duration(&s), work(&s), units(&s, ANN)), (10 * D, 100 * H, 125.0));
}

/// Effort-driven, adding a full-time resource: the 80 hours are shared, 40 each. Fixed Units
/// and Fixed Work take 5 days; Fixed Duration keeps 10 days and both drop to 50%.
#[test]
fn worked_example_tt10_effort_driven_adding_a_resource_keeps_the_work() {
    for (ty, days, each) in [("Fixed Units", 5, 100.0), ("Fixed Work", 5, 100.0), ("Fixed Duration", 10, 50.0)] {
        let mut s = draft(ty, true);
        s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
        assert_eq!((duration(&s), work(&s)), (days * D, 80 * H), "{ty}");
        assert_eq!((work_of(&s, ANN), work_of(&s, BOB)), (40 * H, 40 * H), "{ty}");
        assert_eq!((units(&s, ANN), units(&s, BOB)), (each, each), "{ty}");
    }
}

/// Not effort-driven, adding a full-time resource: the duration stays 10 days and the new
/// resource brings its own 80 hours, 160 in all.
#[test]
fn worked_example_tt11_not_effort_driven_adding_a_resource_adds_work() {
    for ty in ["Fixed Units", "Fixed Duration"] {
        let mut s = draft(ty, false);
        s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
        assert_eq!((duration(&s), work(&s)), (10 * D, 160 * H), "{ty}");
        assert_eq!((units(&s, ANN), units(&s, BOB)), (100.0, 100.0), "{ty}");
    }
}

/// Effort-driven, removing a resource: the work left goes to the resources left. Ann and Bob
/// share 80 hours over 5 days (Fixed Units) or at 50% over 10 (Fixed Duration); with Bob gone,
/// Ann has all 80 hours: 10 days at 100% either way.
#[test]
fn worked_example_tt12_effort_driven_removing_a_resource_keeps_the_work() {
    for ty in ["Fixed Units", "Fixed Work", "Fixed Duration"] {
        let mut s = draft(ty, true);
        s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
        s.run("assignment.remove", &json!({"task": 1, "resource": 2})).unwrap();
        assert_eq!((duration(&s), work(&s), units(&s, ANN)), (10 * D, 80 * H, 100.0), "{ty}");
    }
}

/// Not effort-driven, removing a resource: its work goes with it and the duration stays.
#[test]
fn worked_example_tt13_not_effort_driven_removing_a_resource_takes_its_work() {
    let mut s = draft("Fixed Units", false);
    s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
    s.run("assignment.remove", &json!({"task": 1, "resource": 2})).unwrap();
    assert_eq!((duration(&s), work(&s)), (10 * D, 80 * H));
    // Removing the only resource takes its work away and leaves the duration.
    s.run("assignment.remove", &json!({"task": 1, "resource": 1})).unwrap();
    assert_eq!((duration(&s), work(&s)), (10 * D, 0));
}

/// The Effort Driven field's own example: a four-week drawing task for one drafter, effort-driven,
/// takes one week once four drafters are on it.
#[test]
fn worked_example_tt14_four_drafters_take_a_quarter_of_the_time() {
    let mut s = draft("Fixed Units", true);
    s.run("task.setDuration", &json!({"id": 1, "duration": "4w"})).unwrap();
    assert_eq!(work(&s), 160 * H);
    for r in [2, 3, 4] {
        s.run("assignment.add", &json!({"task": 1, "resource": r})).unwrap();
    }
    assert_eq!((duration(&s), work(&s)), (5 * D, 160 * H));
}

/// Effort-driven calculations apply only after the first assignment: resources assigned together
/// to a task with no work each work the whole duration; one added later shares the work.
#[test]
fn worked_example_tt15_resources_assigned_together_first_are_not_shared() {
    let mut s = draft("Fixed Units", true);
    s.run("task.setResourceNames", &json!({"id": 1, "text": ""})).unwrap();
    assert_eq!((duration(&s), work(&s)), (10 * D, 0));
    s.run("task.setResourceNames", &json!({"id": 1, "text": "Ann, Bob"})).unwrap();
    assert_eq!((duration(&s), work(&s)), (10 * D, 160 * H));
    // A third full-time resource, typed in: the 160 hours shared three ways, 53.33 hours each.
    s.run("task.setResourceNames", &json!({"id": 1, "text": "Ann, Bob, Cat"})).unwrap();
    assert_eq!(work(&s), 160 * H);
    assert_eq!(duration(&s), 3200, "160 hours over 300%: 53 hours 20 minutes");
    // The order typed doesn't change the result: units changes go before new resources.
    let mut a = draft("Fixed Units", true);
    let mut b = draft("Fixed Units", true);
    a.run("task.setResourceNames", &json!({"id": 1, "text": "Ann[50%], Bob"})).unwrap();
    b.run("task.setResourceNames", &json!({"id": 1, "text": "Bob, Ann[50%]"})).unwrap();
    assert_eq!((duration(&a), work(&a)), (duration(&b), work(&b)));
    assert_eq!(a.project.resource_names(DRAFT), "Ann[50%], Bob");
    assert_eq!(b.project.resource_names(DRAFT), "Bob, Ann[50%]");
}

/// Changing a task's type recalculates nothing, and with several resources on different units
/// the duration is the longest assignment's: Ann at 200% and Bob at 100% each keep their 80
/// hours on a Fixed Units task; Ann's take 5 days, Bob's 10.
#[test]
fn worked_example_tt16_each_assignment_keeps_its_own_duration() {
    let mut s = draft("Fixed Units", false);
    s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "200%"})).unwrap();
    let spans: Vec<i64> = s.project.assignments_of(DRAFT).map(|a| a.duration).collect();
    assert_eq!(spans, [5 * D, 10 * D]);
    assert_eq!((duration(&s), work(&s)), (10 * D, 160 * H));
    let before = s.project.assignments.clone();
    s.run("task.setType", &json!({"id": 1, "type": "Fixed Duration"})).unwrap();
    assert_eq!(s.project.assignments, before);
    assert_eq!(duration(&s), 10 * D);
    // 120 hours, shared in proportion to their work: 60 each. Fixed Duration keeps each one's
    // duration, so Ann goes to 150% and Bob to 75%.
    s.run("task.setWork", &json!({"id": 1, "work": 120})).unwrap();
    assert_eq!((work_of(&s, ANN), work_of(&s, BOB)), (60 * H, 60 * H), "shared in proportion to their work");
    assert_eq!((units(&s, ANN), units(&s, BOB)), (150.0, 75.0));
}

/// Costs follow the work the task type keeps: Ann and Bob at £10.00 an hour. Effort-driven,
/// adding Bob shares the 80 hours, so the task still costs £800.00; not effort-driven, Bob brings
/// 80 hours of his own and it costs £1,600.00.
#[test]
fn worked_example_tt17_costs_follow_the_work() {
    for (effort_driven, pence) in [(true, 80_000), (false, 160_000)] {
        let mut s = draft("Fixed Units", effort_driven);
        for r in [1, 2] {
            s.run("resource.set", &json!({"id": r, "standardRate": 1000})).unwrap();
        }
        assert_eq!(s.project.total_cost(), 80_000);
        s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
        assert_eq!(s.project.total_cost(), pence, "effort-driven: {effort_driven}");
    }
}

/// An assignment at 0% does no work and doesn't hold the duration: Ann and Bob at 100% on 10
/// days, Bob set to 0%, then Ann to 200%: Ann's 80 hours take 5 days, and so does the task.
#[test]
fn worked_example_tt18_an_assignment_at_0_percent_holds_nothing() {
    let mut s = draft("Fixed Units", false);
    s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 2, "units": "0%"})).unwrap();
    assert_eq!((duration(&s), work(&s), work_of(&s, BOB)), (10 * D, 80 * H, 0));
    s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "200%"})).unwrap();
    assert_eq!((duration(&s), work(&s)), (5 * D, 80 * H));
    // It opens the same way.
    let again = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(again.project, s.project);
}

/// A duration typed on a task whose assignments are of different lengths: every work assignment
/// takes the new duration, and the task type says what else moves. Microsoft's pages give the
/// rule for one assignment (the table above; "How Project schedules tasks: Behind the scenes",
/// https://support.microsoft.com/en-us/project/how-project-schedules-tasks-behind-the-scenes)
/// and say what a duration is ("Duration (task field)",
/// https://support.microsoft.com/en-us/project/duration-task-field; "Change a task duration",
/// https://support.microsoft.com/en-us/office/change-a-task-duration-0c551218-e543-4a5a-80f5-363c5c7c4e49),
/// but not what happens to assignments of different lengths, so this pins ProjectCraft's choice
/// (documented on `Project::set_duration`).
///
/// Ann at 200% and Bob at 100%, 80 hours each: Ann's take 5 days, Bob's 10, the task 10. Typing
/// 8 days:
/// - Fixed Units and Fixed Duration keep the units: Ann 8 days × 200% = 128 hours, Bob 8 days ×
///   100% = 64 hours, 192 in all.
/// - Fixed Work keeps the work: 80 hours in 8 days is 125% for each.
///
/// Either way both finish with the task, at 17:00 on Wednesday 14 January.
#[test]
fn worked_example_tt19_a_new_duration_on_assignments_of_different_lengths() {
    for ty in ["Fixed Units", "Fixed Duration", "Fixed Work"] {
        let mut s = draft("Fixed Units", false);
        s.run("assignment.add", &json!({"task": 1, "resource": 2})).unwrap();
        s.run("assignment.setUnits", &json!({"task": 1, "resource": 1, "units": "200%"})).unwrap();
        s.run("task.setType", &json!({"id": 1, "type": ty})).unwrap();
        let spans: Vec<i64> = s.project.assignments_of(DRAFT).map(|a| a.duration).collect();
        assert_eq!(spans, [5 * D, 10 * D], "{ty}: different lengths before");
        assert_eq!(duration(&s), 10 * D);
        s.run("task.setDuration", &json!({"id": 1, "duration": 8})).unwrap();
        let spans: Vec<i64> = s.project.assignments_of(DRAFT).map(|a| a.duration).collect();
        assert_eq!(spans, [8 * D, 8 * D], "{ty}: every assignment takes the new duration");
        assert_eq!(duration(&s), 8 * D, "{ty}");
        assert_eq!(finish(&s), at("2026-01-14", 17), "{ty}");
        if ty == "Fixed Work" {
            assert_eq!((work_of(&s, ANN), work_of(&s, BOB), work(&s)), (80 * H, 80 * H, 160 * H), "{ty}");
            assert_eq!((units(&s, ANN), units(&s, BOB)), (125.0, 125.0), "{ty}");
        } else {
            assert_eq!((work_of(&s, ANN), work_of(&s, BOB), work(&s)), (128 * H, 64 * H, 192 * H), "{ty}");
            assert_eq!((units(&s, ANN), units(&s, BOB)), (200.0, 100.0), "{ty}");
        }
        // Undo puts back the different lengths.
        s.run("edit.undo", &json!({})).unwrap();
        let spans: Vec<i64> = s.project.assignments_of(DRAFT).map(|a| a.duration).collect();
        assert_eq!(spans, [5 * D, 10 * D], "{ty}: undone");
    }
}

#[test]
fn types_and_options_are_commands_and_bad_ones_change_nothing() {
    let mut s = draft("Fixed Units", false);
    let task = |s: &Session| s.project.task(DRAFT).unwrap().clone();
    // Fixed Work is always effort-driven: making one sets it, and it can't be cleared.
    s.run("task.setType", &json!({"id": 1, "type": "Fixed Work"})).unwrap();
    assert!(task(&s).effort_driven);
    let before = s.project.clone();
    let e = s.run("task.setType", &json!({"id": 1, "effortDriven": false})).unwrap_err().to_string();
    assert!(e.contains("always effort-driven"), "{e}");
    for bad in [json!({"id": 1}), json!({"id": 1, "type": "Fixed Cost"}), json!({"id": 9, "type": "Fixed Work"})] {
        assert!(s.run("task.setType", &bad).is_err(), "{bad}");
    }
    // Work is refused below 0, on a task with no work resource, and as junk.
    s.run("task.add", &json!({"name": "Alone"})).unwrap();
    let e = s.run("task.setWork", &json!({"id": 2, "work": 8})).unwrap_err().to_string();
    assert!(e.contains("assign a work resource"), "{e}");
    for bad in [json!(-1), json!("lots"), json!("-2h"), json!("1e308")] {
        assert!(s.run("task.setWork", &json!({"id": 1, "work": bad})).is_err(), "{bad}");
    }
    // A summary's type, effort-driven setting, duration and work are its subtasks'.
    s.run("task.indent", &json!({"ids": [2]})).unwrap();
    for (cmd, params) in [
        ("task.setType", json!({"id": 1, "type": "Fixed Units"})),
        ("task.setWork", json!({"id": 1, "work": 8})),
        ("task.setDuration", json!({"id": 1, "duration": 3})),
    ] {
        assert!(s.run(cmd, &params).is_err(), "{cmd}");
    }
    assert!(!s.project.is_effort_driven(DRAFT), "a summary is never effort-driven");
    assert_eq!(s.project.task_type_of(DRAFT), TaskType::FixedDuration);
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, before);
    // New tasks take the project's defaults; new Fixed Work tasks are always effort-driven.
    s.run("project.setOptions", &json!({"defaultTaskType": "Fixed Duration", "newTasksEffortDriven": true})).unwrap();
    s.run("task.add", &json!({"name": "New"})).unwrap();
    let t = s.project.tasks.last().unwrap().clone();
    assert_eq!((t.task_type, t.effort_driven), (TaskType::FixedDuration, true));
    s.run("project.setOptions", &json!({"defaultTaskType": "Fixed Work", "newTasksEffortDriven": false})).unwrap();
    s.run("task.insert", &json!({"name": "Newer"})).unwrap();
    let t = s.project.tasks.last().unwrap().clone();
    assert_eq!((t.task_type, t.effort_driven), (TaskType::FixedWork, true));
    assert!(s.run("project.setOptions", &json!({"defaultTaskType": "Fixed"})).is_err());
    let round = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(round.project, s.project);
}

/// A file from before task types: tasks are Fixed Units, not effort-driven, and each work
/// assignment works its task's duration at its units.
#[test]
fn a_file_from_before_task_types_loads_with_its_work() {
    let old = json!({
        "format_version": 2, "name": "Old", "start": "2026-01-05",
        "tasks": [{"id": 1, "name": "A", "duration": 2400}], "next_id": 2,
        "resources": [{"id": 1, "name": "Ann"}, {"id": 2, "name": "Steel", "type": "Material"}],
        "assignments": [{"task": 1, "resource": 1, "units": {"work": 5000}}, {"task": 1, "resource": 2, "units": {"quantity": 500}}],
    });
    let s = Session::load_json(&old.to_string()).unwrap();
    let t = s.project.task(TaskId(1)).unwrap();
    assert_eq!((t.task_type, t.effort_driven), (TaskType::FixedUnits, false));
    let a: Vec<(i64, i64)> = s.project.assignments.iter().map(|a| (a.work, a.duration)).collect();
    assert_eq!(a, [(20 * H, 5 * D), (0, 0)]);
    // A file whose task's duration isn't its longest assignment's (a hand edit, say) still opens:
    // the duration is repaired, so the plan is never lost.
    let mut edited = serde_json::from_str::<serde_json::Value>(&s.save_json().unwrap()).unwrap();
    edited["assignments"][0]["duration"] = json!(9999);
    let opened = Session::load_json(&edited.to_string()).unwrap();
    assert_eq!(opened.project.task(TaskId(1)).unwrap().duration, 9999);
    edited["tasks"][0]["duration"] = json!(17);
    assert_eq!(Session::load_json(&edited.to_string()).unwrap().project.task(TaskId(1)).unwrap().duration, 9999);
    // Work or a duration out of range is still refused.
    for (k, v) in [("work", json!(-5)), ("duration", json!(i64::MAX)), ("duration", json!(-2)), ("work", json!(i64::MAX))] {
        let mut bad = serde_json::from_str::<serde_json::Value>(&s.save_json().unwrap()).unwrap();
        bad["assignments"][0][k] = v.clone();
        assert!(Session::load_json(&bad.to_string()).is_err(), "{k} {v}");
    }
}
