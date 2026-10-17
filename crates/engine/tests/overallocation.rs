//! Worked examples: overallocation.
//!
//! Source: Microsoft Project's "View resource workloads and availability"
//! (https://support.microsoft.com/en-US/project/view-resource-workloads-and-availability-in-project-desktop):
//! a resource is overallocated when it is assigned more work than it can do in the time
//! available, its Max Units of its working time. ProjectCraft counts day by day: a work resource is
//! overallocated on a working day when its work that day is more than Max Units × that day's
//! working time on its calendar. Materials and cost resources are never overallocated.
//!
//! The project calendar works Monday to Friday, 8:00–12:00 and 13:00–17:00 (8 hours). The project
//! starts on Monday 5 January 2026. Work is in minutes; every number is worked out by hand first.

use std::collections::BTreeSet;

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Project, ResourceId, TaskId, Usage};
use serde_json::{Value, json};

const ANN: ResourceId = ResourceId(1);
const BOB: ResourceId = ResourceId(2);
const CAT: ResourceId = ResourceId(3);
const STEEL: ResourceId = ResourceId(4);
const TRAVEL: ResourceId = ResourceId(5);
const HOUR: i64 = 60;

/// A plan of these tasks (name, duration in days, predecessors by Unique ID) with work resources
/// Ann (100%), Bob (Max Units 50%) and Cat (part time, mornings 8:00–12:00), a material, Steel,
/// and a cost resource, Travel.
fn plan(tasks: &[(&str, i64, &str)]) -> Session {
    let mut s = Session::new(Project::new("t", "2026-01-05".parse().unwrap()));
    for (i, (name, days, preds)) in (1..).zip(tasks) {
        run(&mut s, "task.add", json!({"name": name, "duration": days}));
        run(&mut s, "task.setPredecessors", json!({"id": i, "text": preds}));
    }
    for r in [
        json!({"name": "Ann"}),
        json!({"name": "Bob"}),
        json!({"name": "Cat"}),
        json!({"name": "Steel", "type": "Material"}),
        json!({"name": "Travel", "type": "Cost"}),
    ] {
        run(&mut s, "resource.add", r);
    }
    run(&mut s, "resource.set", json!({"id": 2, "maxUnits": 50}));
    run(&mut s, "resource.set", json!({"id": 4, "materialLabel": "tons"}));
    for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
        run(&mut s, "calendar.setWorkingTimes", json!({"weekday": day, "times": ["8:00-12:00"], "resource": 3}));
    }
    s
}

fn run(s: &mut Session, cmd: &str, params: Value) {
    s.run(cmd, &params).unwrap_or_else(|e| panic!("{cmd} {params}: {e}"));
}

fn names(s: &mut Session, id: u32, text: &str) {
    run(s, "task.setResourceNames", json!({"id": id, "text": text}));
}

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn usage(s: &Session) -> &Usage {
    s.usage().unwrap()
}

/// A resource's work on each day it works: (date, minutes).
fn days(s: &Session, r: ResourceId) -> Vec<(Date, i64)> {
    usage(s).resource(r).unwrap().days.iter().map(|(d, w)| (*d, *w)).collect()
}

fn over(s: &Session, r: ResourceId) -> Vec<Date> {
    usage(s).resource(r).unwrap().over.iter().copied().collect()
}

fn flagged(s: &Session) -> BTreeSet<u32> {
    usage(s).overallocated_tasks().into_iter().map(|t| t.0).collect()
}

/// OA1. Ann at 100% on two 1-day tasks on the same Monday: 16 hours against 8. She is
/// overallocated on Monday, and both tasks have an overallocated assignment. Put in sequence, the
/// work is 8 hours a day and nothing is over.
#[test]
fn worked_example_oa1_two_tasks_at_once_overallocate() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    names(&mut s, 1, "Ann");
    names(&mut s, 2, "Ann");
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 16 * HOUR)]);
    assert_eq!(over(&s, ANN), [d("2026-01-05")]);
    assert!(usage(&s).is_overallocated(ANN));
    assert_eq!(flagged(&s), BTreeSet::from([1, 2]));
    run(&mut s, "task.setPredecessors", json!({"id": 2, "text": "1"}));
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 8 * HOUR), (d("2026-01-06"), 8 * HOUR)]);
    assert!(!usage(&s).is_overallocated(ANN));
    assert!(flagged(&s).is_empty());
}

/// OA2. Ann at 50% on each of two 1-day tasks on the same Monday: 4 + 4 = 8 hours, exactly her
/// day. Not over: only more than Max Units is.
#[test]
fn worked_example_oa2_exactly_max_units_is_not_over() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    names(&mut s, 1, "Ann[50%]");
    names(&mut s, 2, "Ann[50%]");
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 8 * HOUR)]);
    assert!(over(&s, ANN).is_empty());
    assert!(flagged(&s).is_empty());
}

/// OA3. Over on one day only. Dig takes Ann two days (Monday, Tuesday); Fence starts a day after
/// Dig starts (SS+1d) and takes her Tuesday. Tuesday has 16 hours, Monday 8: over on Tuesday
/// only, and both tasks work that Tuesday.
#[test]
fn worked_example_oa3_over_only_where_assignments_meet() {
    let mut s = plan(&[("Dig", 2, ""), ("Fence", 1, "1SS+1d")]);
    names(&mut s, 1, "Ann");
    names(&mut s, 2, "Ann");
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 8 * HOUR), (d("2026-01-06"), 16 * HOUR)]);
    assert_eq!(over(&s, ANN), [d("2026-01-06")]);
    assert_eq!(flagged(&s), BTreeSet::from([1, 2]));
    // A third task on Wednesday, alone, is not flagged.
    run(&mut s, "task.add", json!({"name": "Paint", "duration": 1}));
    run(&mut s, "task.setPredecessors", json!({"id": 3, "text": "1"}));
    names(&mut s, 3, "Ann");
    assert_eq!(flagged(&s), BTreeSet::from([1, 2]));
}

/// OA4. Max Units 50%: Bob can do 4 hours a day. Assigned to a 1-day task he gets 50% (his Max
/// Units), 4 hours on Monday: not over. On a second task the same day, 8 hours: over. At 100%
/// on one task, 8 hours: over too.
#[test]
fn worked_example_oa4_max_units_50_percent() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    names(&mut s, 1, "Bob");
    assert_eq!(days(&s, BOB), [(d("2026-01-05"), 4 * HOUR)]);
    assert!(over(&s, BOB).is_empty());
    names(&mut s, 2, "Bob");
    assert_eq!(days(&s, BOB), [(d("2026-01-05"), 8 * HOUR)]);
    assert_eq!(over(&s, BOB), [d("2026-01-05")]);
    // Removed from both, then assigned afresh at 100% to Fence (a fresh assignment keeps the
    // task's 1 day; changing the units of an existing one would shorten it, Fixed Units).
    names(&mut s, 1, "");
    names(&mut s, 2, "");
    names(&mut s, 2, "Bob[100%]");
    assert_eq!(days(&s, BOB), [(d("2026-01-05"), 8 * HOUR)]);
    assert_eq!(over(&s, BOB), [d("2026-01-05")]);
    assert_eq!(flagged(&s), BTreeSet::from([2]));
}

/// OA5. A part-time calendar: Cat works mornings, 4 hours a day. A 1-day task (8 hours of work at
/// 100%) takes her Monday and Tuesday mornings, 4 hours each: not over. Two such tasks at once
/// give her 8 hours each morning against 4: over on both days.
#[test]
fn worked_example_oa5_part_time_calendar() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    names(&mut s, 1, "Cat");
    assert_eq!(days(&s, CAT), [(d("2026-01-05"), 4 * HOUR), (d("2026-01-06"), 4 * HOUR)]);
    assert!(over(&s, CAT).is_empty());
    names(&mut s, 2, "Cat");
    assert_eq!(days(&s, CAT), [(d("2026-01-05"), 8 * HOUR), (d("2026-01-06"), 8 * HOUR)]);
    assert_eq!(over(&s, CAT), [d("2026-01-05"), d("2026-01-06")]);
    // The same 8 hours a day on Ann's full-time calendar would not be over: the calendar counts.
    assert_eq!(flagged(&s), BTreeSet::from([1, 2]));
}

/// OA6. Part time and Max Units 50% together: Cat at 50% can do 2 hours each morning. At 50% on a
/// 1-day task she works 8 hours of duration (two mornings) for 4 hours of work: 2 hours a day,
/// not over. At 100% on the same 1-day task, 4 hours a morning against 2: over.
#[test]
fn worked_example_oa6_part_time_and_max_units_50_percent() {
    let mut s = plan(&[("Dig", 1, "")]);
    run(&mut s, "resource.set", json!({"id": 3, "maxUnits": 50}));
    names(&mut s, 1, "Cat");
    assert_eq!(days(&s, CAT), [(d("2026-01-05"), 2 * HOUR), (d("2026-01-06"), 2 * HOUR)]);
    assert!(over(&s, CAT).is_empty());
    // Assigned afresh at 100% (changing the units would shorten the task, Fixed Units).
    names(&mut s, 1, "");
    names(&mut s, 1, "Cat[100%]");
    assert_eq!(days(&s, CAT), [(d("2026-01-05"), 4 * HOUR), (d("2026-01-06"), 4 * HOUR)]);
    assert_eq!(over(&s, CAT), [d("2026-01-05"), d("2026-01-06")]);
}

/// OA7. Leave: Ann is off Tuesday. Her 2-day task takes Monday and Wednesday; a 1-day task
/// starting Wednesday (FS after a 2-day task without resources) meets it there: over on
/// Wednesday, never on her day off.
#[test]
fn worked_example_oa7_leave_moves_the_work_and_the_overallocation() {
    let mut s = plan(&[("Dig", 2, ""), ("Wait", 2, ""), ("Fence", 1, "2")]);
    run(&mut s, "calendar.addException", json!({"date": "2026-01-06", "resource": 1}));
    names(&mut s, 1, "Ann");
    names(&mut s, 3, "Ann");
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 8 * HOUR), (d("2026-01-07"), 16 * HOUR)]);
    assert_eq!(over(&s, ANN), [d("2026-01-07")]);
}

/// OA8. Materials and cost resources are never overallocated, however much is assigned at once,
/// and are not in the usage.
#[test]
fn worked_example_oa8_materials_and_costs_are_never_overallocated() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    names(&mut s, 1, "Steel[1000 tons], Travel[£900]");
    names(&mut s, 2, "Steel[50 tons/h], Travel[£900]");
    let u = usage(&s);
    for r in [STEEL, TRAVEL] {
        assert!(u.resource(r).is_none());
        assert!(!u.is_overallocated(r));
    }
    assert!(u.overallocated_tasks().is_empty());
    assert!(u.resources.iter().all(|r| !r.is_overallocated()));
}

/// OA9. Day by day: two 4-hour tasks for Ann at once on Monday morning make 8 hours on Monday,
/// her day; not over on a day-by-day count, though both run in the same hours.
#[test]
fn worked_example_oa9_counted_day_by_day() {
    let mut s = plan(&[("Dig", 1, ""), ("Fence", 1, "")]);
    for id in [1, 2] {
        run(&mut s, "task.setDuration", json!({"id": id, "duration": "4h"}));
        names(&mut s, id, "Ann");
    }
    assert_eq!(days(&s, ANN), [(d("2026-01-05"), 8 * HOUR)]);
    assert!(over(&s, ANN).is_empty());
}

/// Each assignment's own days add up to its work, and the resource's days are their sum.
#[test]
fn assignments_add_up_to_the_resource() {
    let mut s = plan(&[("Dig", 3, ""), ("Fence", 2, "1SS+1d")]);
    names(&mut s, 1, "Ann[50%], Cat");
    names(&mut s, 2, "Ann");
    let u = usage(&s).resource(ANN).unwrap();
    let dig: i64 = u.assignments.iter().find(|(t, _)| *t == TaskId(1)).unwrap().1.values().sum();
    assert_eq!(dig, 12 * HOUR);
    assert_eq!(u.work(), 12 * HOUR + 16 * HOUR);
    assert_eq!(u.work(), s.project.assignments.iter().filter(|a| a.resource == ANN).map(|a| a.work).sum::<i64>());
}

/// The sample's Priya Shah works Survey (50%) and Order furniture (100%) on the same day.
#[test]
fn the_sample_shows_an_overallocation() {
    let s = projectcraft_engine::sample();
    let u = s.usage().unwrap();
    assert!(u.is_overallocated(ResourceId(1)));
    assert!(!u.is_overallocated(ResourceId(2)), "Tom Okafor works within his 50%");
    assert!(u.overallocated_tasks().contains(&TaskId(2)) && u.overallocated_tasks().contains(&TaskId(3)));
}
