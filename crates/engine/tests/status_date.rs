//! The status date: the date progress is reported as of, as Microsoft Project's Status Date
//! (Project ▸ Project Information). The owner sets or clears it; the project never reads the
//! clock for it, and it moves no task. Saved in the project file from format 3.

use projectcraft_engine::Session;
use projectcraft_engine::model::{Date, Project};
use serde_json::json;

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// Start Monday 2026-01-05: A (3 days, Mon 5 – Wed 7) → B (2 days, Thu 8 – Fri 9).
fn plan() -> Session {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("task.add", &json!({"name": "A", "duration": 3})).unwrap();
    s.run("task.add", &json!({"name": "B", "duration": 2})).unwrap();
    s.run("task.setPredecessors", &json!({"id": 2, "text": "1"})).unwrap();
    s
}

#[test]
fn a_new_project_has_no_status_date() {
    assert_eq!(plan().project.status_date, None);
}

#[test]
fn setting_and_clearing_are_one_undo_step_each() {
    let mut s = plan();
    s.run("project.setStatusDate", &json!({"date": "2026-01-07"})).unwrap();
    assert_eq!(s.project.status_date, Some(date("2026-01-07")));
    s.run("project.setStatusDate", &json!({"date": "2026-01-08"})).unwrap();
    s.run("project.setStatusDate", &json!({})).unwrap();
    assert_eq!(s.project.status_date, None);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project.status_date, Some(date("2026-01-08")));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project.status_date, Some(date("2026-01-07")));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project.status_date, None);
    assert!(s.can_undo(), "the plan's own edits are still there");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(s.project.status_date, Some(date("2026-01-07")));
}

#[test]
fn setting_the_same_date_again_adds_no_undo_step() {
    let mut s = Session::new(Project::new("t", date("2026-01-05")));
    s.run("project.setStatusDate", &json!({"date": "2026-01-07"})).unwrap();
    s.run("project.setStatusDate", &json!({"date": "2026-01-07"})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project.status_date, None);
    assert!(!s.can_undo());
}

/// A status date before, inside or after the plan moves no task.
#[test]
fn the_status_date_moves_no_task() {
    let mut s = plan();
    let before = s.schedule.clone().unwrap();
    for d in ["2025-12-01", "2026-01-07", "2026-01-08", "2027-06-30"] {
        s.run("project.setStatusDate", &json!({"date": d})).unwrap();
        assert_eq!(s.schedule.as_ref().unwrap(), &before, "{d}");
    }
}

#[test]
fn a_bad_date_is_refused_and_changes_nothing() {
    let mut s = plan();
    s.run("project.setStatusDate", &json!({"date": "2026-01-07"})).unwrap();
    let before = s.project.clone();
    for bad in
        [json!({"date": "2026-02-30"}), json!({"date": "7 Jan 2026"}), json!({"date": 20260107}), json!({"date": null}), json!({"day": "2026-01-07"})]
    {
        assert!(s.run("project.setStatusDate", &bad).is_err(), "{bad}");
        assert_eq!(s.project, before, "{bad}");
    }
}

#[test]
fn the_status_date_survives_a_save_and_open() {
    let mut s = plan();
    s.run("project.setStatusDate", &json!({"date": "2026-01-07"})).unwrap();
    let text = s.save_json().unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!((v["format_version"].clone(), v["status_date"].clone()), (json!(3), json!("2026-01-07")));
    let opened = Session::load_json(&text).unwrap();
    assert_eq!(opened.project, s.project);
}
