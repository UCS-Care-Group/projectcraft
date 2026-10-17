//! Worked examples: Level All and Clear Leveling as commands.
//!
//! Source: Microsoft's "Level resource assignments"
//! (https://support.microsoft.com/en-US/project/distribute-project-work-evenly-level-resource-assignments):
//! levelling resolves overallocation by delaying tasks (ProjectCraft never splits them); it looks
//! for overallocation day by day; the Standard order examines predecessors, slack, dates,
//! priority and constraints; priority runs to 1000, default 500, a lower priority is delayed
//! first and 1000 is not levelled; levelling values are cleared before levelling; Clear Leveling
//! removes them.
//!
//! The default calendar works Monday to Friday, 8 hours a day. The sample project starts on
//! Monday 2 November 2026. Every date is worked out by hand first.

use projectcraft_engine::model::{DEFAULT_PRIORITY, TaskId};
use projectcraft_engine::{Session, sample};
use serde_json::json;

fn dates(s: &Session, id: u32) -> (String, String) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start.to_string(), d.finish.to_string())
}

fn over(s: &Session) -> Vec<String> {
    let u = s.usage().unwrap();
    u.resources.iter().filter(|r| r.is_overallocated()).map(|r| s.project.resource(r.id).unwrap().name.clone()).collect()
}

/// The sample: Priya Shah works half time on "Survey the new floor" (3 days, 2 days of slack)
/// and full time on "Order furniture" (1 day, critical), both from Wednesday 4 November: 150%.
/// Levelling delays the survey, which has the slack, by a day, and the cabling after it; Tom
/// Okafor, 50% on the survey and the cabling, is never over. The Removals crew's 300% on Move day
/// is within its Max Units.
#[test]
fn level_all_delays_the_task_with_slack_in_one_undo_step() {
    let mut s = sample();
    assert_eq!(over(&s), vec!["Priya Shah"]);
    assert_eq!(dates(&s, 2), ("2026-11-04".into(), "2026-11-06".into()));
    let before = s.project.clone();
    let finish = s.schedule.as_ref().unwrap().finish;
    s.run("project.level", &json!({})).unwrap();
    assert!(over(&s).is_empty());
    assert_eq!(dates(&s, 3), ("2026-11-04".into(), "2026-11-04".into()), "the critical task stays");
    assert_eq!(dates(&s, 2), ("2026-11-05".into(), "2026-11-09".into()));
    assert_eq!(dates(&s, 5), ("2026-11-10".into(), "2026-11-16".into()), "its successor moves with it");
    assert_eq!(s.schedule.as_ref().unwrap().finish, finish, "within slack: the finish holds");
    let report = s.level_report().unwrap();
    assert_eq!(report.delayed, vec![TaskId(2)]);
    assert_eq!(report.text(&s.project), "Levelling delayed 1 task.");
    // One undo step.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project, before);
    s.run("edit.redo", &json!({})).unwrap();
    // Clear Leveling: one undo step back to the plan as it was.
    s.run("project.clearLeveling", &json!({})).unwrap();
    assert_eq!(s.project, before);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(dates(&s, 2).0, "2026-11-05");
}

/// Priority 1000 on the survey: it can't be delayed, so the furniture order is, though it is
/// critical, and the plan finishes a day later.
#[test]
fn priority_is_a_command_and_1000_is_never_levelled() {
    let mut s = sample();
    assert_eq!(s.project.task(TaskId(2)).unwrap().priority, DEFAULT_PRIORITY);
    s.run("task.setPriority", &json!({"id": 2, "priority": 1000})).unwrap();
    s.run("project.level", &json!({})).unwrap();
    assert_eq!(s.level_report().unwrap().delayed, vec![TaskId(3)]);
    assert_eq!(dates(&s, 2).0, "2026-11-04");
    // Priya is half time on the survey until Friday: the order waits for Monday 9 November.
    assert_eq!(dates(&s, 3).0, "2026-11-09");
    assert_eq!(s.schedule.as_ref().unwrap().finish.to_string(), "2026-11-25");
    assert!(over(&s).is_empty());
    for bad in [json!({"id": 2, "priority": 1001}), json!({"id": 2, "priority": -1}), json!({"id": 99, "priority": 5})] {
        assert!(s.run("task.setPriority", &bad).is_err(), "{bad}");
    }
}

/// The Leveling Delay can be typed too, and saved; the scheduler adds it after the links.
#[test]
fn a_typed_leveling_delay_moves_the_task_and_is_saved() {
    let mut s = sample();
    s.run("task.setLevelingDelay", &json!({"id": 5, "delay": "1d"})).unwrap();
    assert_eq!(dates(&s, 5).0, "2026-11-10");
    s.run("task.setLevelingDelay", &json!({"id": 5, "delay": 0})).unwrap();
    assert_eq!(dates(&s, 5).0, "2026-11-09");
    s.run("task.setLevelingDelay", &json!({"id": 5, "delay": "4h"})).unwrap();
    let saved = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(saved.project.task(TaskId(5)).unwrap().leveling_delay, 240);
    assert_eq!(saved.schedule, s.schedule);
    // A summary is never delayed itself.
    s.run("task.indent", &json!({"ids": [3]})).unwrap();
    assert!(s.run("task.setLevelingDelay", &json!({"id": 2, "delay": 1})).is_err());
}

/// A plan nothing can level: Tom Okafor at 100% on a task when his Max Units are 50%. Level All
/// still runs, changes nothing, and says what is left.
#[test]
fn a_plan_that_cannot_be_levelled_says_so_and_changes_nothing() {
    let mut s = sample();
    s.run("assignment.setUnits", &json!({"task": 5, "resource": 2, "units": "100%"})).unwrap();
    s.run("task.setPriority", &json!({"id": 3, "priority": 1000})).unwrap();
    s.run("task.setPriority", &json!({"id": 2, "priority": 1000})).unwrap();
    let before = s.project.clone();
    let undo = s.can_undo();
    s.run("project.level", &json!({})).unwrap();
    assert_eq!(s.project, before);
    assert_eq!(s.can_undo(), undo);
    let text = s.level_report().unwrap().text(&s.project);
    assert_eq!(text, "Levelling delayed no task. Still overallocated: Priya Shah (from 2026-11-04), Tom Okafor (from 2026-11-09).");
}
