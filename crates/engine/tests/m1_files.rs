//! Files saved by M1 open with the same dates.
//!
//! The fixtures in `fixtures/m1/` were saved by ProjectCraft at the last M1 commit (file format 1,
//! durations and lags in whole working days, no working hours): the sample project; the 200-task
//! generated plan the speed gate uses (summaries three levels deep, all four link types with lag
//! and lead, every constraint type, deadlines, holidays and working Saturdays); a plan with
//! summary links, a Must Start On conflict, negative slack and deadlines; a four-day week; and a
//! link matrix (every link type with a lead, none and a lag, from a task, a milestone at the
//! start of a day and one at the end of a day, to a one-day task and to a milestone). Beside each,
//! its `.m1-dates` file is what M1's scheduler gave for it, written by M1 itself: the project
//! finish, then for each task its start, finish, late start, late finish, total slack in working
//! days, critical, constraint conflict, missed deadline and start driver, then each summary's
//! rolled-up duration in working days.
//!
//! Today's scheduler counts working minutes; an M1 day is the default 8 hours (480 minutes), so
//! every date, flag and driver must match and every slack and rolled-up duration be 480 × M1's,
//! except where [`CHANGED_SINCE_M1`] says otherwise.

use projectcraft_engine::Session;
use projectcraft_engine::model::TaskId;
use serde_json::Value;

const FIXTURES: &[&str] = &["sample", "generated-200", "site-office", "four-day-week", "link-matrix"];

/// An M1 working day in today's working minutes.
const DAY: i64 = 480;

/// Dates deliberately changed since M1 (fixture, task Unique ID, the finish shown now). Issue
/// #74: a task finish-to-finish (or start-to-finish) with no lag after a milestone at the start
/// of a day finishes at that moment, as Microsoft Project shows it (Wednesday 8:00), where M1
/// showed the end of the day before (Tuesday 17:00). The same working time, so its start, late
/// dates and slack are M1's; only the finish date shown moves.
const CHANGED_SINCE_M1: &[(&str, u32, &str)] = &[("link-matrix", 42, "2026-01-14"), ("link-matrix", 48, "2026-01-14")];

fn fixture(name: &str, ext: &str) -> String {
    let path = format!("{}/tests/fixtures/m1/{name}.{ext}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The dates M1 recorded for a fixture, with [`CHANGED_SINCE_M1`] applied.
fn expected(name: &str) -> String {
    let mut out = String::new();
    for line in fixture(name, "m1-dates").lines() {
        let mut cols: Vec<&str> = line.split('\t').collect();
        if let ["task", id, ..] = cols[..]
            && let Some((_, _, finish)) = CHANGED_SINCE_M1.iter().find(|(f, t, _)| *f == name && t.to_string() == id)
        {
            assert_ne!(cols[3], *finish, "{name} task {id}: the change is no longer needed");
            cols[3] = finish;
        }
        out += &cols.join("\t");
        out.push('\n');
    }
    out
}

/// What today's scheduler gives for a file, in the `.m1-dates` layout (slack and rolled-up
/// durations converted back to M1's working days).
fn dates_record(s: &Session) -> String {
    let sch = s.schedule.as_ref().unwrap();
    let mut out = format!("finish\t{}\n", sch.finish);
    for t in &s.project.tasks {
        let d = &sch.tasks[&t.id];
        assert_eq!(d.slack % DAY, 0, "task {}: slack {} is not whole days", t.id.0, d.slack);
        out += &format!(
            "task\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:?}\n",
            t.id.0,
            d.start,
            d.finish,
            d.late_start,
            d.late_finish,
            d.slack / DAY,
            d.critical,
            d.constraint_conflict,
            d.deadline_missed,
            d.driver
        );
    }
    for (id, minutes) in &sch.rolled_up {
        assert_eq!(minutes % DAY, 0, "summary {}: {minutes} minutes is not whole days", id.0);
        out += &format!("rolled_up\t{}\t{}\n", id.0, minutes / DAY);
    }
    out
}

/// The lines of `got` that differ from `want`, M1's beside today's.
fn differences(name: &str, want: &str, got: &str) -> Option<String> {
    let mut differ: Vec<String> = want.lines().zip(got.lines()).filter(|(e, g)| e != g).map(|(e, g)| format!("  M1:  {e}\n  now: {g}")).collect();
    if want.lines().count() != got.lines().count() {
        differ.push(format!("  {} lines in M1's record, {} now", want.lines().count(), got.lines().count()));
    }
    (!differ.is_empty()).then(|| format!("{name}: {} line(s) differ:\n{}", differ.len(), differ.join("\n")))
}

#[test]
fn m1_files_open_with_the_dates_m1_gave_them() {
    let mut failures = Vec::new();
    for name in FIXTURES {
        let text = fixture(name, "projectcraft");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["format_version"], 1, "{name} is an M1 file");
        let s = Session::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        failures.extend(differences(name, &expected(name), &dates_record(&s)));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Opened, saved as today's format and opened again, an M1 file keeps its dates; its durations
/// are still whole days.
#[test]
fn m1_files_saved_again_keep_their_dates() {
    for name in FIXTURES {
        let s = Session::load_json(&fixture(name, "projectcraft")).unwrap();
        let again = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(again.project, s.project, "{name}");
        assert_eq!(again.schedule.as_ref().unwrap(), s.schedule.as_ref().unwrap(), "{name}");
        assert_eq!(differences(name, &expected(name), &dates_record(&again)), None);
        for t in &again.project.tasks {
            assert_eq!(t.duration % DAY, 0, "{name}: task {}", t.id.0);
        }
    }
}

/// A few of the recorded dates spelled out, so the fixtures can be checked by eye.
#[test]
fn m1_dates_spot_checks() {
    let open = |name: &str| Session::load_json(&fixture(name, "projectcraft")).unwrap();
    let shown = |s: &Session, id: u32| {
        let d = &s.schedule.as_ref().unwrap().tasks[&TaskId(id)];
        (d.start.to_string(), d.finish.to_string(), d.slack / DAY)
    };
    // The sample, Office move, from Monday 2 November 2026: Move day is Friday 20 November.
    let s = open("sample");
    assert_eq!(shown(&s, 7), ("2026-11-20".into(), "2026-11-20".into(), 0));
    assert_eq!(s.schedule.as_ref().unwrap().finish.to_string(), "2026-11-20");
    // Site office: the Must Start On holds Drawings on Wednesday 4 March against its link, so
    // Survey shows two days' negative slack; Fit out runs over Good Friday and Easter Monday.
    let s = open("site-office");
    assert_eq!(shown(&s, 3), ("2026-03-04".into(), "2026-03-10".into(), 0));
    assert_eq!(shown(&s, 2), ("2026-03-02".into(), "2026-03-04".into(), -2));
    assert_eq!(shown(&s, 9), ("2026-04-08".into(), "2026-04-14".into(), 0));
    assert!(s.schedule.as_ref().unwrap().tasks[&TaskId(3)].constraint_conflict);
    assert_eq!(s.schedule.as_ref().unwrap().finish.to_string(), "2026-04-16");
    // Four-day week: Do runs seven Monday-to-Thursday days from Wednesday 3 June, with Friday 5
    // June worked and Thursday 11 June off.
    let s = open("four-day-week");
    assert_eq!(shown(&s, 2), ("2026-06-03".into(), "2026-06-15".into(), 7));
    // Link matrix: a one-day task finish-to-finish a day after a milestone at the start of
    // Wednesday 14 January finishes at the end of Wednesday (issue #60), not on Thursday.
    let s = open("link-matrix");
    assert_eq!(shown(&s, 44), ("2026-01-14".into(), "2026-01-14".into(), 1));
    assert_eq!(shown(&s, 40), ("2026-01-12".into(), "2026-01-12".into(), 3));
}
