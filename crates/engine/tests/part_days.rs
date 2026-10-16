//! Worked examples: part-day durations and lags.
//!
//! Sources: Microsoft Project's duration units, m (minutes), h (hours), d (days), w (weeks) and mo
//! (months), with an `e` in front for elapsed time
//! (https://learn.microsoft.com/en-us/office-project/xml-data-interchange/durationformat-element);
//! a task's duration is held in minutes
//! (https://learn.microsoft.com/en-us/office/vba/api/project.task.duration) and shown in the unit it
//! was entered in; days, weeks and months are converted with the project's hours per day
//! (https://learn.microsoft.com/en-us/office/vba/api/project.project.hoursperday), hours per week and
//! days per month, by default 8, 40 and 20.
//!
//! The default calendar works Monday to Friday, 8:00–12:00 and 13:00–17:00. The project starts on
//! Monday 5 January 2026. Every moment is worked out by hand first.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Moment, Project, TaskId, duration_text};
use serde_json::{Value, json};

fn session(tasks: &[(&str, Value, &str)]) -> Session {
    let mut s = Session::new(Project::new("t", "2026-01-05".parse().unwrap()));
    for (i, (name, duration, preds)) in (1..).zip(tasks) {
        s.run("task.add", &json!({"name": name, "duration": duration})).unwrap();
        s.run("task.setPredecessors", &json!({"id": i, "text": preds})).unwrap();
    }
    s
}

fn at(d: &str, h: u16, m: u16) -> Moment {
    Moment { date: d.parse::<Date>().unwrap(), minute: h * 60 + m }
}

/// When a task starts and finishes.
fn span(s: &Session, id: u32) -> (Moment, Moment) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start_at, d.finish_at)
}

/// The Duration column's text.
fn shown(s: &Session, id: u32) -> String {
    let t = s.project.task(TaskId(id)).unwrap();
    duration_text(t.duration, t.duration_unit, &s.project.options)
}

/// A 1.5-day task is 720 minutes: all of Monday and Tuesday morning. A 4-hour successor takes
/// Tuesday afternoon, after lunch; a 30-minute one the first half hour of Wednesday. A milestone
/// after the first task sits on its finish, Tuesday 12:00. Each shows the unit it was typed in.
#[test]
fn worked_example_pd1_days_hours_and_minutes_in_a_chain() {
    let s = session(&[("A", json!("1.5d"), ""), ("B", json!("4h"), "1"), ("C", json!("30m"), "2"), ("M", json!(0), "1")]);
    assert_eq!(s.project.task(TaskId(1)).unwrap().duration, 720);
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-06", 12, 0)));
    assert_eq!(span(&s, 2), (at("2026-01-06", 13, 0), at("2026-01-06", 17, 0)));
    assert_eq!(span(&s, 3), (at("2026-01-07", 8, 0), at("2026-01-07", 8, 30)));
    assert_eq!(span(&s, 4), (at("2026-01-06", 12, 0), at("2026-01-06", 12, 0)));
    assert_eq!([shown(&s, 1), shown(&s, 2), shown(&s, 3), shown(&s, 4)], ["1.5d", "4h", "30m", "0d"]);
    // The Start and Finish columns still show the day.
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(1)];
    assert_eq!((d.start.to_string(), d.finish.to_string()), ("2026-01-05".into(), "2026-01-06".into()));
}

/// A lag in hours: B follows a 1-day A by 4 hours, so it starts Tuesday 13:00 and, a day long,
/// finishes Wednesday 12:00. A 4-hour lead instead starts it Monday 13:00, half way through A.
/// The Predecessors column shows the lag as typed.
#[test]
fn worked_example_pd2_part_day_lag_and_lead() {
    let mut s = session(&[("A", json!(1), ""), ("B", json!(1), "1FS+4h")]);
    assert_eq!(span(&s, 2), (at("2026-01-06", 13, 0), at("2026-01-07", 12, 0)));
    assert_eq!(s.project.preds_text_rows(&s.project.task(TaskId(2)).unwrap().preds), "1FS+4h");
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1FS-0.5d"})).unwrap();
    assert_eq!(span(&s, 2), (at("2026-01-05", 13, 0), at("2026-01-06", 12, 0)));
    assert_eq!(s.project.preds_text_rows(&s.project.task(TaskId(2)).unwrap().preds), "1FS-0.5");
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1SS+90m"})).unwrap();
    assert_eq!(span(&s, 2), (at("2026-01-05", 9, 30), at("2026-01-06", 9, 30)));
}

/// Days, weeks and months follow the project options, not the calendar. At 6 hours a day, 30 a
/// week and 2 days a month, on the 8-hour calendar: 1d is 360 minutes (Monday 8:00–15:00, with the
/// lunch hour), 1w is 1800 (three full days and 6 hours: Thursday 15:00), 1mo is 720 (Tuesday 12:00).
#[test]
fn worked_example_pd3_units_follow_the_project_options() {
    let mut s = Session::new(Project::new("t", "2026-01-05".parse().unwrap()));
    s.run("project.setOptions", &json!({"hoursPerDay": 6, "hoursPerWeek": 30, "daysPerMonth": 2})).unwrap();
    for d in ["1d", "1w", "1mo"] {
        s.run("task.add", &json!({"duration": d})).unwrap();
    }
    assert_eq!(span(&s, 1), (at("2026-01-05", 8, 0), at("2026-01-05", 15, 0)));
    assert_eq!(span(&s, 2), (at("2026-01-05", 8, 0), at("2026-01-08", 15, 0)));
    assert_eq!(span(&s, 3), (at("2026-01-05", 8, 0), at("2026-01-06", 12, 0)));
    assert_eq!([shown(&s, 1), shown(&s, 2), shown(&s, 3)], ["1d", "1w", "1mo"]);
}

/// Changing hours per day after entry changes no dates: durations are minutes. A 2-day task
/// entered at 8 hours (960 minutes) shows as 2.67d at 6 hours; a 4-hour one stays 4h.
#[test]
fn worked_example_pd4_changing_hours_per_day_keeps_the_minutes() {
    let mut s = session(&[("A", json!(2), ""), ("B", json!("4h"), "")]);
    let before = (span(&s, 1), span(&s, 2));
    s.run("project.setOptions", &json!({"hoursPerDay": 6})).unwrap();
    assert_eq!((span(&s, 1), span(&s, 2)), before);
    assert_eq!([shown(&s, 1), shown(&s, 2)], ["2.67d", "4h"]);
}

/// Integer days still work in commands; elapsed units and junk are refused with the reason, and
/// leave the project as it was.
#[test]
fn worked_example_pd5_whole_days_still_work_and_elapsed_is_refused() {
    let mut s = session(&[("A", json!(3), "")]);
    assert_eq!(shown(&s, 1), "3d");
    let before = s.project.clone();
    let e = s.run("task.setDuration", &json!({"id": 1, "duration": "7ed"})).unwrap_err().to_string();
    assert!(e.contains("elapsed durations aren't supported yet"), "{e}");
    let e = s.run("task.setPredecessors", &json!({"id": 1, "text": "2FS+1ew"})).unwrap_err().to_string();
    assert!(e.contains("elapsed lags aren't supported yet"), "{e}");
    assert!(s.run("task.setDuration", &json!({"id": 1, "duration": "-4h"})).is_err());
    assert_eq!(s.project, before);
    // A part-day duration and its unit survive a save and load.
    s.run("task.setDuration", &json!({"id": 1, "duration": "2.25h"})).unwrap();
    let round = Session::load_json(&s.save_json().unwrap()).unwrap();
    assert_eq!(round.project, s.project);
    assert_eq!(shown(&round, 1), "2.25h");
}
