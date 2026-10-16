//! Worked example 7 of M1: working weekdays and
//! calendar exceptions. Durations and lags count working days only; an exception wins over its
//! weekday, both ways.
//!
//! Source: Microsoft Project's Change Working Time ("Set the general working days and times for a
//! project", "Add a holiday to the project calendar"): a task is scheduled only in the calendar's
//! working time; an exception makes a day non-working (a holiday) or working (a normally
//! non-working day such as a Saturday), overriding the work week.
//! https://support.microsoft.com/en-us/project/set-the-general-working-days-and-times-for-a-project
//!
//! Every edit goes through the `calendar.*` commands. Each date is worked out by hand first.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Project, TaskId};
use serde_json::{Value, json};

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

fn session(start: &str, tasks: &[(&str, i64, &str)]) -> Session {
    let mut s = Session::new(Project::new("t", date(start)));
    for (i, (name, duration, preds)) in (1..).zip(tasks) {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
        s.run("task.setPredecessors", &json!({"id": i, "text": preds})).unwrap();
    }
    s
}

fn run(s: &mut Session, id: &str, params: Value) {
    s.run(id, &params).unwrap();
}

fn start_finish(s: &Session, id: u32) -> (Date, Date) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start, d.finish)
}

/// A 2-day task starting on a Friday works Friday and the next Monday: Saturday and Sunday are
/// not working weekdays in the default calendar.
#[test]
fn worked_example_07a_a_two_day_task_starting_friday_skips_the_weekend() {
    let s = session("2026-01-09", &[("A", 2, "")]);
    assert_eq!(date("2026-01-09").weekday(), 4, "a Friday");
    assert_eq!(start_finish(&s, 1), (date("2026-01-09"), date("2026-01-12")));
}

/// A non-working exception inside a task's span pushes its finish out a working day. Project
/// start Monday 2026-01-05, a 3-day task: Mon 5, Tue 6, Wed 7. With Wednesday 7 a holiday:
/// Mon 5, Tue 6, Thu 8. Removing the exception (or undoing it) brings Wednesday back.
#[test]
fn worked_example_07b_a_non_working_exception_inside_the_span() {
    let mut s = session("2026-01-05", &[("A", 3, "")]);
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-07")));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-07", "working": false}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-08")));
    run(&mut s, "calendar.removeException", json!({"date": "2026-01-07"}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-07")));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-08")));
}

/// A lag spanning an exception counts working days only. A (1 day) runs Monday 2026-01-05.
/// B is FS+2: the lag takes Tue 6 and Wed 7, so B starts Thu 8. With Wednesday 7 a holiday the
/// lag takes Tue 6 and Thu 8, so B starts Fri 9 and, being 2 days, finishes Mon 12.
#[test]
fn worked_example_07c_a_lag_spanning_an_exception() {
    let mut s = session("2026-01-05", &[("A", 1, ""), ("B", 2, "1FS+2")]);
    assert_eq!(start_finish(&s, 2), (date("2026-01-08"), date("2026-01-09")));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-07"}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-05")));
    assert_eq!(start_finish(&s, 2), (date("2026-01-09"), date("2026-01-12")));
}

/// A working exception on a Saturday is worked. Project start Friday 2026-01-09, a 3-day task:
/// Fri 9, Mon 12, Tue 13. With Saturday 10 made working: Fri 9, Sat 10, Mon 12.
#[test]
fn worked_example_07d_a_working_exception_on_a_saturday() {
    let mut s = session("2026-01-09", &[("A", 3, "")]);
    assert_eq!(start_finish(&s, 1), (date("2026-01-09"), date("2026-01-13")));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-10", "working": true}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-09"), date("2026-01-12")));
    // A task starting on that Saturday: the project start moves there and the task begins on it.
    run(&mut s, "project.setStart", json!({"date": "2026-01-10"}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-10"), date("2026-01-13")));
}

/// Changing the working weekdays moves the dates; a week with no working day is refused and
/// changes nothing, even with a working exception in the calendar.
#[test]
fn working_weekdays_move_dates_and_a_week_with_none_is_refused() {
    let mut s = session("2026-01-09", &[("A", 2, "")]);
    run(&mut s, "calendar.setWorking", json!({"weekdays": ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]}));
    assert_eq!(start_finish(&s, 1), (date("2026-01-09"), date("2026-01-10")));
    run(&mut s, "calendar.addException", json!({"date": "2026-01-11", "working": true}));
    let before = s.project.clone();
    let e = s.run("calendar.setWorking", &json!({"weekdays": []})).unwrap_err().to_string();
    assert!(e.contains("at least one working weekday"), "{e}");
    assert_eq!(s.project, before);
    assert!(s.schedule.is_ok());
}
