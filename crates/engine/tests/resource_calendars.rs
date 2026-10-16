//! Worked examples: resource calendars.
//!
//! Sources: Microsoft Project's Ignore Resource Calendar field
//! (https://support.microsoft.com/en-us/project/ignore-resource-calendar-task-field): it is No by
//! default, so a task's schedule takes its resources' calendars into account; only a task
//! calendar with the field set to Yes leaves them out (ProjectCraft has no task calendars yet, so
//! every task with resources is scheduled on their calendars). Each resource's calendar is based
//! on the project calendar, with its own working times and exceptions (leave, part-time working);
//! each assignment is worked on its resource's calendar and the task spans its earliest
//! assignment's start to its latest assignment's finish. A task without resources is scheduled on
//! the project calendar.
//!
//! The project calendar works Monday to Friday, 8:00–12:00 and 13:00–17:00. The project starts on
//! Monday 5 January 2026. Every moment is worked out by hand first.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Moment, Project, ResourceId, TaskId};
use serde_json::{Value, json};

const ANN: ResourceId = ResourceId(1);
const BOB: ResourceId = ResourceId(2);

/// A plan of these tasks (name, duration in days, predecessors by Unique ID), with work
/// resources Ann and Bob (Unique IDs 1 and 2) and a material, Steel (3).
fn plan(tasks: &[(&str, i64, &str)]) -> Session {
    let mut s = Session::new(Project::new("t", "2026-01-05".parse().unwrap()));
    for (i, (name, days, preds)) in (1..).zip(tasks) {
        s.run("task.add", &json!({"name": name, "duration": days})).unwrap();
        s.run("task.setPredecessors", &json!({"id": i, "text": preds})).unwrap();
    }
    for r in [json!({"name": "Ann"}), json!({"name": "Bob"}), json!({"name": "Steel", "type": "Material"})] {
        s.run("resource.add", &r).unwrap();
    }
    s
}

fn run(s: &mut Session, cmd: &str, params: Value) {
    s.run(cmd, &params).unwrap_or_else(|e| panic!("{cmd} {params}: {e}"));
}

fn at(d: &str, h: u16, m: u16) -> Moment {
    Moment { date: d.parse::<Date>().unwrap(), minute: h * 60 + m }
}

/// When a task starts and finishes.
fn span(s: &Session, id: u32) -> (Moment, Moment) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start_at, d.finish_at)
}

/// When a resource's assignment to a task starts and finishes.
fn worked(s: &Session, id: u32, r: ResourceId) -> (Moment, Moment) {
    s.schedule.as_ref().unwrap().assignments[&(TaskId(id), r)]
}

/// Leave in the middle: Ann is off Tuesday and Wednesday. Her 5 days on Build take Monday,
/// Thursday, Friday, and Monday and Tuesday of the next week, so Build finishes Tuesday 13
/// January at 17:00, two days later, and Paint after it starts Wednesday. The duration stays
/// 5 days: the leave isn't working time.
#[test]
fn worked_example_rc1_leave_moves_the_assignment_and_the_task() {
    let mut s = plan(&[("Build", 5, ""), ("Paint", 1, "1")]);
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-09", 17, 0)));
    for day in ["2026-01-06", "2026-01-07"] {
        run(&mut s, "calendar.addException", json!({"date": day, "working": false, "resource": 1}));
    }
    assert_eq!(worked(&s, 1, ANN), (at("2026-01-05", 8, 0), at("2026-01-13", 17, 0)));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-13", 17, 0)));
    assert_eq!(span(&s, 2), (at("2026-01-14", 8, 0), at("2026-01-14", 17, 0)));
    assert_eq!(s.project.task(TaskId(1)).unwrap().duration, 5 * 480);
    // The project calendar is untouched: a task without resources still runs Monday to Friday.
    assert!(s.project.calendar.exceptions.is_empty());
}

/// Leave at the start: Ann is off Monday and Tuesday, so her assignment, and Build, start on
/// Wednesday 7 January and finish the next Tuesday.
#[test]
fn worked_example_rc2_leave_at_the_start_moves_the_start() {
    let mut s = plan(&[("Build", 5, "")]);
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    for day in ["2026-01-05", "2026-01-06"] {
        run(&mut s, "calendar.addException", json!({"date": day, "resource": 1}));
    }
    assert_eq!(span(&s, 1), (at("2026-01-07", 8, 0), at("2026-01-13", 17, 0)));
    // Removing the leave puts it back.
    for day in ["2026-01-05", "2026-01-06"] {
        run(&mut s, "calendar.removeException", json!({"date": day, "resource": 1}));
    }
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-09", 17, 0)));
}

/// Two resources on different calendars on one task: Ann works the standard week, Bob part time,
/// mornings only (8:00–12:00). On a 2-day task each works 16 hours: Ann Monday and Tuesday,
/// Bob four mornings, Monday to Thursday. The task spans both: Monday 8:00 to Thursday 12:00.
#[test]
fn worked_example_rc3_two_calendars_on_one_task_span_both() {
    let mut s = plan(&[("Survey", 2, ""), ("Report", 1, "1")]);
    for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
        run(&mut s, "calendar.setWorkingTimes", json!({"weekday": day, "times": ["8:00-12:00"], "resource": 2}));
    }
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann, Bob"}));
    assert_eq!(worked(&s, 1, ANN), (at("2026-01-05", 8, 0), at("2026-01-06", 17, 0)));
    assert_eq!(worked(&s, 1, BOB), (at("2026-01-05", 8, 0), at("2026-01-08", 12, 0)));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-08", 12, 0)));
    // Report, on the project calendar, follows at 13:00 the same day.
    assert_eq!(span(&s, 2), (at("2026-01-08", 13, 0), at("2026-01-09", 12, 0)));
    // Back on the project's times, Bob's mornings go: the task is two days again.
    for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
        run(&mut s, "calendar.setWorkingTimes", json!({"weekday": day, "fromProject": true, "resource": 2}));
    }
    assert!(s.project.resource(BOB).unwrap().calendar.is_empty());
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-06", 17, 0)));
}

/// A task without resources stays on the project calendar, whatever the resources' calendars
/// say; so does one with only materials, which have no calendar.
#[test]
fn worked_example_rc4_a_task_without_resources_stays_on_the_project_calendar() {
    let mut s = plan(&[("Order", 3, ""), ("Deliver", 3, "")]);
    run(&mut s, "calendar.setWorking", json!({"weekdays": ["Mon"], "resource": 1}));
    run(&mut s, "task.setResourceNames", json!({"id": 2, "text": "Steel[5]"}));
    for id in [1, 2] {
        assert_eq!(span(&s, id), (at("2026-01-05", 8, 0), at("2026-01-07", 17, 0)), "{id}");
    }
    // Ann on Order works Mondays only: three Mondays.
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-19", 17, 0)));
    // A material has no calendar. Bob, on leave all week, made a material loses his, so Deliver
    // with him on it keeps the project calendar; and a file giving Steel one opens without it.
    for day in ["2026-01-05", "2026-01-06", "2026-01-07"] {
        run(&mut s, "calendar.addException", json!({"date": day, "resource": 2}));
    }
    run(&mut s, "resource.set", json!({"id": 2, "type": "Material"}));
    assert!(s.project.resource(BOB).unwrap().calendar.is_empty());
    run(&mut s, "task.setResourceNames", json!({"id": 2, "text": "Steel[5], Bob[1]"}));
    assert_eq!(span(&s, 2), (at("2026-01-05", 8, 0), at("2026-01-07", 17, 0)));
    let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
    v["resources"][2]["calendar"] = json!({"exceptions": [{"date": "2026-01-05", "working": false}]});
    let opened = Session::load_json(&v.to_string()).unwrap();
    assert!(opened.project.resource(ResourceId(3)).unwrap().calendar.is_empty(), "dropped, not refused");
    assert_eq!(opened.schedule, s.schedule);
}

/// A resource calendar is based on the project calendar, and a resource works only within the
/// project calendar's working time (Microsoft Project also lets a resource work when the project
/// doesn't; ProjectCraft doesn't yet). Build, 5 days for Ann from Monday 5 January: Wednesday
/// 7 is a project holiday, which reaches her; Thursday 8 she works the afternoon only; Saturday 10
/// she would work 9:00–13:00, but the project doesn't work Saturdays, so she doesn't. Her 40 hours
/// are Monday, Tuesday, Thursday's 4, Friday, Monday 12 and Tuesday 13 until 12:00.
#[test]
fn worked_example_rc5_the_project_calendar_is_the_base() {
    let mut s = plan(&[("Build", 5, "")]);
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-07"}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-08", "times": ["13:00-17:00"], "resource": 1}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-10", "times": ["9:00-13:00"], "resource": 1}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-13", 12, 0)));
    // A working day of her own on the project holiday isn't worked either...
    run(&mut s, "calendar.addException", json!({"date": "2026-01-07", "working": true, "resource": 1}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-13", 12, 0)));
    // ...until the project works it: then her 8 hours that day bring the finish to Monday 12:00.
    run(&mut s, "calendar.removeException", json!({"date": "2026-01-07"}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-12", 12, 0)));
    // The same plan gives the same dates, and saves and opens with them.
    let again = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(again.project, s.project);
    assert_eq!(again.schedule, s.schedule);
}

/// Late dates are worked on the resources' calendars too: a 2-day task for Ann beside a 6-day
/// one, both from Monday 5 January, with Ann on leave Thursday and Friday. Ann's task finishes
/// Tuesday; it may finish as late as the long one, Monday 12 January, and two of Ann's working
/// days back from there are that Monday and Wednesday 7 (Thursday and Friday are leave): its late
/// start is Wednesday, two project working days after its start.
#[test]
fn worked_example_rc6_slack_on_a_resource_calendar() {
    let mut s = plan(&[("Short", 2, ""), ("Long", 6, ""), ("End", 0, "1, 2")]);
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    for day in ["2026-01-08", "2026-01-09"] {
        run(&mut s, "calendar.addException", json!({"date": day, "resource": 1}));
    }
    let sch = s.schedule.as_ref().unwrap();
    let short = sch.tasks[&TaskId(1)];
    assert_eq!((short.start_at, short.finish_at), (at("2026-01-05", 8, 0), at("2026-01-06", 17, 0)));
    // Late: finish by Monday 12 January 17:00; Ann's two days back from there are Monday 12 and
    // Wednesday 7 (Thursday and Friday are leave), so the late start is Wednesday.
    assert_eq!((short.late_start, short.late_finish), ("2026-01-07".parse().unwrap(), "2026-01-12".parse().unwrap()));
    assert_eq!(short.slack, 2 * 480);
    assert!(!short.critical);
    assert!(sch.tasks[&TaskId(2)].critical);
}

/// An assignment at 0% does no work, so its resource's calendar doesn't hold the task: Ann on a
/// 2-day task, Bob on leave all week at 0% beside her. Survey takes Ann's two days.
#[test]
fn worked_example_rc7_an_assignment_at_0_percent_doesnt_hold_the_task() {
    let mut s = plan(&[("Survey", 2, "")]);
    for day in ["2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08", "2026-01-09"] {
        run(&mut s, "calendar.addException", json!({"date": day, "resource": 2}));
    }
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann, Bob[0%]"}));
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-06", 17, 0)));
    assert!(!s.schedule.as_ref().unwrap().assignments.contains_key(&(TaskId(1), BOB)), "no work, nothing to schedule");
}

#[test]
fn calendar_commands_take_a_resource_and_refuse_what_cannot_be() {
    let mut s = plan(&[("Build", 5, "")]);
    let before = s.project.clone();
    for (cmd, params, why) in [
        ("calendar.addException", json!({"date": "2026-01-06", "resource": 3}), "only work resources have calendars"),
        ("calendar.addException", json!({"date": "2026-01-06", "resource": 9}), "no resource"),
        ("calendar.setWorking", json!({"weekdays": ["Sat"], "resource": 1}), ""),
        ("calendar.setWorkingTimes", json!({"weekday": "Mon", "fromProject": true}), "for a resource's calendar"),
        ("calendar.setWorkingTimes", json!({"weekday": "Mon", "times": [], "fromProject": true, "resource": 1}), "not both"),
        ("calendar.setWorkingTimes", json!({"weekday": "Mon", "resource": 1}), "`times`"),
        ("calendar.removeException", json!({"date": "2026-01-06", "resource": 1}), "no exception"),
    ] {
        match s.run(cmd, &params) {
            Ok(()) if why.is_empty() => {
                s.run("edit.undo", &json!({})).unwrap();
            }
            Ok(()) => panic!("{cmd} {params} ran"),
            Err(e) => assert!(e.to_string().contains(why), "{cmd} {params}: {e}"),
        }
        assert_eq!(s.project, before, "{cmd} {params}");
    }
    // A week with no working day is refused for a resource as for the project.
    for day in ["Mon", "Tue", "Wed", "Thu"] {
        run(&mut s, "calendar.setWorkingTimes", json!({"weekday": day, "times": [], "resource": 1}));
    }
    let e = s.run("calendar.setWorkingTimes", &json!({"weekday": "Fri", "times": [], "resource": 1})).unwrap_err().to_string();
    assert!(e.contains("at least one working weekday"), "{e}");
    // So is a project-calendar edit that would leave her with none, naming her (Ann works
    // Fridays only): the project working Mondays only, or not working Fridays. Nothing changes.
    let before = s.project.clone();
    for (cmd, params) in [("calendar.setWorking", json!({"weekdays": ["Mon"]})), ("calendar.setWorkingTimes", json!({"weekday": "Fri", "times": []}))]
    {
        let e = s.run(cmd, &params).unwrap_err().to_string();
        assert!(e.contains("Ann would have no working weekday"), "{cmd}: {e}");
        assert_eq!(s.project, before);
    }
}

/// A file that leaves a resource no working weekday (a hand edit: the commands refuse it) still
/// opens with its dates: that resource is worked on the project calendar. Not something Microsoft
/// documents; ProjectCraft's choice, so a plan is never left without dates.
#[test]
fn a_resource_left_with_no_working_time_is_worked_on_the_project_calendar() {
    let mut s = plan(&[("Build", 2, "")]);
    for day in ["Mon", "Tue", "Wed", "Thu"] {
        run(&mut s, "calendar.setWorkingTimes", json!({"weekday": day, "times": [], "resource": 1}));
    }
    run(&mut s, "task.setResourceNames", json!({"id": 1, "text": "Ann"}));
    // Ann works Fridays only: Friday 9 and Friday 16.
    assert_eq!(span(&s, 1), (at("2026-01-09", 8, 0), at("2026-01-16", 17, 0)));
    // The file edited to a project working Mondays only: Ann has no working weekday left.
    let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
    v["calendar"]["times"] = json!([[{"from": 480, "to": 720}, {"from": 780, "to": 1020}], [], [], [], [], [], []]);
    let opened = Session::load_json(&v.to_string()).unwrap();
    assert_eq!(opened.project.calendar.working, [true, false, false, false, false, false, false]);
    let d = opened.schedule.as_ref().unwrap().tasks[&TaskId(1)];
    assert_eq!((d.start_at, d.finish_at), (at("2026-01-05", 8, 0), at("2026-01-12", 17, 0)), "two Mondays, on the project calendar");
}

/// Resource calendars are saved with the resource; a hostile one is refused on open.
#[test]
fn resource_calendars_save_and_hostile_ones_are_refused() {
    let mut s = plan(&[("Build", 5, "")]);
    run(&mut s, "calendar.setWorkingTimes", json!({"weekday": "Fri", "times": ["8:00-12:00"], "resource": 1}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-06", "resource": 1}));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-10", "times": ["9:00-13:00"], "resource": 1}));
    let v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
    assert_eq!(
        v["resources"][0]["calendar"],
        json!({"week": [null, null, null, null, [{"from": 480, "to": 720}], null, null],
            "exceptions": [{"date": "2026-01-06", "working": false}, {"date": "2026-01-10", "working": true, "times": [{"from": 540, "to": 780}]}]})
    );
    assert!(v["resources"][1].get("calendar").is_none(), "nothing of its own, nothing written");
    assert_eq!(Session::load_json(&v.to_string()).unwrap().project, s.project);
    for bad in [
        json!({"week": [[{"from": 720, "to": 480}], null, null, null, null, null, null]}),
        json!({"week": [null, null]}),
        json!({"exceptions": [{"date": "2026-01-12", "working": false, "times": [{"from": 480, "to": 720}]}]}),
        json!({"exceptions": [{"date": "2026-02-30"}]}),
        json!({"exceptions": [{"date": "2026-01-12", "working": true, "times": [{"from": 480, "to": 2000}]}]}),
    ] {
        let mut f = v.clone();
        f["resources"][0]["calendar"] = bad.clone();
        assert!(Session::load_json(&f.to_string()).is_err(), "{bad}");
    }
}
