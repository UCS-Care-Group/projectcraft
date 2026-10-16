//! Worked examples for the four link types with lag, and for loops of links. Each is worked out
//! by hand from the Microsoft documentation it cites, before the code.
//!
//! Link types: "FinishStart ... must finish before [the successor] can start", "StartStart ...
//! cannot start until [the predecessor] starts", "FinishFinish ... cannot finish until [the
//! predecessor] finishes", "StartFinish ... must start before [the successor] finishes",
//! https://learn.microsoft.com/dotnet/api/websvcproject.projectdataset.dependencyrow.link_type?view=office-project-server.
//! Lag is added to the link and lead time is a negative lag,
//! https://learn.microsoft.com/office/vba/api/project.taskdependencies.add.
//!
//! Every plan starts Monday 2026-01-05 on the default calendar. Working-day offsets:
//! 0 Mon 5, 1 Tue 6, 2 Wed 7, 3 Thu 8, 4 Fri 9, 5 Mon 12, 6 Tue 13, 7 Wed 14, 8 Thu 15.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Project, TaskId};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Lead (2 days) → A (3 days), FS; then B (2 days) linked to A by `link`. Lead runs Mon 5 –
/// Tue 6 and A Wed 7 – Fri 9 (offsets 2..5). Changing Lead's duration moves A's start and finish;
/// changing A's duration moves only its finish.
fn lead_a_b(link: &str) -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "Lead", "duration": 2})).unwrap();
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "B", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 3, "text": link})).unwrap();
    assert_eq!(start_finish(&s, 2), (date("2026-01-07"), date("2026-01-09")));
    s
}

fn start_finish(s: &Session, id: u32) -> (Date, Date) {
    let d = s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
    (d.start, d.finish)
}

fn duration(s: &mut Session, id: u32, days: i64) {
    s.run("task.setDuration", &json!({"id": id, "duration": days})).unwrap();
}

/// Start-to-start with 1 day of lag: B starts a working day after A starts (offset 2 + 1 = 3,
/// Thu 8 – Fri 9). Lead to 4 days: A starts Fri 9, so B starts Mon 12 – Tue 13 (later). Lead to
/// 1 day: A starts Tue 6, so B starts Wed 7 – Thu 8 (earlier than at first). A's own duration
/// doesn't move B.
#[test]
fn a_start_to_start_successor_with_lag_moves_later_and_earlier_with_the_predecessor_start() {
    let mut s = lead_a_b("2SS+1");
    assert_eq!(start_finish(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    duration(&mut s, 1, 4);
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-13")));
    duration(&mut s, 1, 1);
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-08")));
    duration(&mut s, 2, 10);
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-08")));
}

/// Finish-to-finish with 2 days of lag: B finishes two working days after A does. A ends at
/// offset 5, so B ends at 7: Mon 12 – Tue 13. A to 5 days ends at 7, B at 9: Wed 14 – Thu 15
/// (later). A to 1 day ends at 3, B at 5: Thu 8 – Fri 9 (earlier than at first).
#[test]
fn a_finish_to_finish_successor_with_lag_moves_later_and_earlier_with_the_predecessor_finish() {
    let mut s = lead_a_b("2FF+2");
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-13")));
    duration(&mut s, 2, 5);
    assert_eq!(start_finish(&s, 3), (date("2026-01-14"), date("2026-01-15")));
    duration(&mut s, 2, 1);
    assert_eq!(start_finish(&s, 3), (date("2026-01-08"), date("2026-01-09")));
}

/// Start-to-finish with 3 days of lag: B finishes three working days after A starts. A starts
/// at offset 2, so B ends at 5: Thu 8 – Fri 9. Lead to 4 days: A starts at 4, B ends at 7:
/// Mon 12 – Tue 13 (later). Lead to 1 day: A starts at 1, B ends at 4: Wed 7 – Thu 8 (earlier).
#[test]
fn a_start_to_finish_successor_with_lag_moves_later_and_earlier_with_the_predecessor_start() {
    let mut s = lead_a_b("2SF+3");
    assert_eq!(start_finish(&s, 3), (date("2026-01-08"), date("2026-01-09")));
    duration(&mut s, 1, 4);
    assert_eq!(start_finish(&s, 3), (date("2026-01-12"), date("2026-01-13")));
    duration(&mut s, 1, 1);
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-08")));
}

/// Finish-to-start with 1 day of lead (a negative lag): B starts a working day before A
/// finishes. A ends at offset 5, so B starts at 4: Fri 9 – Mon 12. A to 1 day ends at 3, B
/// starts at 2: Wed 7 – Thu 8.
#[test]
fn a_finish_to_start_successor_with_lead_time_overlaps_its_predecessor() {
    let mut s = lead_a_b("2FS-1");
    assert_eq!(start_finish(&s, 3), (date("2026-01-09"), date("2026-01-12")));
    duration(&mut s, 2, 1);
    assert_eq!(start_finish(&s, 3), (date("2026-01-07"), date("2026-01-08")));
}

/// A → B, then B → A: the second link closes a loop and is refused with a message; the project
/// and its schedule are unchanged. Project refuses a circular relationship
/// (https://learn.microsoft.com/en-us/office/troubleshoot/project/circular-relationship-error-when-indenting-task).
#[test]
fn linking_b_back_to_a_is_refused_with_a_message_and_changes_nothing() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "B", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    let before = s.project.clone();
    let dates = s.schedule.as_ref().unwrap().clone();
    for text in ["2", "2SS+1", "2FF", "2SF-3"] {
        let e = s.run("task.setPredecessors", &json!({"id": 1, "text": text})).unwrap_err().to_string();
        assert!(e.contains("loop"), "{text}: {e}");
        assert_eq!(s.project, before);
        assert_eq!(s.schedule.as_ref().unwrap(), &dates);
    }
    // A loop through a third task is refused the same way, and so is a link to itself.
    s.run("task.add", &json!({"name": "C", "duration": 1})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 3, "text": "2"})).unwrap();
    assert!(s.run("task.setPredecessors", &json!({"id": 1, "text": "3"})).unwrap_err().to_string().contains("loop"));
    assert!(s.run("task.setPredecessors", &json!({"id": 1, "text": "1"})).unwrap_err().to_string().contains("loop"));
    assert!(s.project.task(TaskId(1)).unwrap().preds.is_empty());
}

/// A file holding a loop of links (only a damaged or hand-edited one could) loads; the scheduler
/// reports the loop, naming a task in it, and stops. A ring of 50,000 tasks is reported as
/// quickly as a ring of two. Breaking the loop with a command schedules the plan again.
#[test]
fn a_loop_in_a_loaded_file_is_reported_and_scheduling_ends() {
    for n in [2u32, 50_000] {
        let tasks: Vec<_> = (1..=n)
            .map(|i| json!({"id": i, "name": format!("T{i}"), "duration": 1, "preds": [{"pred": if i == 1 { n } else { i - 1 }, "kind": "SS", "lag": 1}]}))
            .collect();
        let file = json!({"name": "looped", "start": "2026-01-05", "tasks": tasks});
        let mut s = Session::load_json(&file.to_string()).unwrap();
        let e = s.schedule.as_ref().unwrap_err().to_string();
        assert!(e.contains("loop through task"), "{e}");

        // Any other edit still runs and still reports the loop.
        s.run("task.rename", &json!({"id": 1, "name": "Start"})).unwrap();
        assert!(s.schedule.is_err());
        s.run("task.setPredecessors", &json!({"id": 1, "text": ""})).unwrap();
        assert!(s.schedule.is_ok(), "{n}: the loop is broken");
        assert_eq!(start_finish(&s, 1), (date("2026-01-05"), date("2026-01-05")));
    }
}
