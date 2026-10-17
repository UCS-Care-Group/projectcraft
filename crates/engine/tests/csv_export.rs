//! CSV export round trips. The task sheet exported as CSV and imported again gives the same
//! tasks in every column the import reads: exporting the imported project gives, cell for cell,
//! the file exported first. Over generated projects (every field, hostile text, names that start
//! like a formula) and the sample.
//!
//! Columns that don't round trip, as the import report says: Work and Cost (worked out from the
//! resources, whose rates and Max Units a CSV doesn't hold); Start and Finish (worked out by the
//! scheduler: the earliest Start is the project's start, and constraints and calendars aren't
//! sheet columns); and so a summary's Duration, rolled up from its subtasks' dates.

mod common;

use common::{Lcg, generated};
use projectcraft_engine::Session;
use projectcraft_engine::interchange::{self, Format};
use projectcraft_engine::model::{Column, Columns, Outline, Project, lag_text};
use proptest::prelude::*;

/// The columns an import reads back as they were.
const ROUND_TRIP: [Column; 9] = [
    Column::Duration,
    Column::Predecessors,
    Column::ResourceNames,
    Column::FixedCost,
    Column::Deadline,
    Column::Type,
    Column::EffortDriven,
    Column::Priority,
    Column::LevelingDelay,
];

/// What the import of an exported sheet says, and nothing else: the columns it works out itself,
/// the project's start and the currency.
const EXPECTED_LINES: [&str; 6] = ["Project start", "Currency", "Work column", "Start column", "Finish column", "Cost column"];

/// `p` with names that a spreadsheet would run as formulas, some already starting with an
/// apostrophe, on some tasks and resources.
fn with_formulas(mut p: Project, seed: u64) -> Project {
    let mut rng = Lcg(seed ^ 0x5eed);
    const START: [&str; 8] = ["=", "+", "-", "@", "'=", "'", "''@", "=HYPERLINK(\"x\")"];
    for t in &mut p.tasks {
        if rng.chance(25) {
            t.name = format!("{}{}", rng.pick(&START), t.name);
        }
    }
    for r in &mut p.resources {
        if rng.chance(25) {
            r.name = format!("{}{}", rng.pick(&["=", "+", "-", "@", "'@"]), r.name);
        }
    }
    p
}

/// The sheet of `p` as a CSV of `columns`, without a schedule (its dates aren't compared) and
/// without its summaries' durations, which are rolled up from their subtasks' dates.
fn sheet(p: &Project, columns: &[Column]) -> String {
    let mut p = p.clone();
    p.columns = Columns::new(columns.iter().copied());
    let outline = Outline::of(&p);
    for (i, t) in p.tasks.iter_mut().enumerate() {
        if outline.is_summary(i) {
            t.duration = 0;
        }
    }
    interchange::export(Format::Csv, &p, None).unwrap()
}

/// `p` with each lag too small to show (`+0w`, 7 minutes in weeks) at none: the cell reads back
/// as no lag, which the Predecessors column shows by leaving it off.
fn lags_as_shown(mut p: Project) -> Project {
    let options = p.options;
    for l in p.tasks.iter_mut().flat_map(|t| t.preds.iter_mut()) {
        let text = lag_text(l.lag, l.lag_unit, &options);
        if text.trim_start_matches(['+', '-']).trim_end_matches(|c: char| c.is_alphabetic()) == "0" {
            l.lag = 0;
        }
    }
    p
}

fn round_trip(p0: &Project) -> Result<(), TestCaseError> {
    let mut p0 = lags_as_shown(p0.clone());
    p0.columns = Columns::all();
    let s0 = Session::new(p0.clone());
    let x0 = s0.export(Format::Csv).unwrap();
    let s1 = Session::import(Format::Csv, x0.as_bytes()).unwrap();
    let report = s1.import_report().unwrap();
    // A task without a name is said to have none.
    let expected = |f: &str| EXPECTED_LINES.contains(&f) || (f == "Task Name" && p0.tasks.iter().any(|t| t.name.is_empty()));
    prop_assert!(report.lines.iter().all(|l| expected(&l.feature)), "{}\n{x0}", report.text());
    let (a, b) = (sheet(&p0, &ROUND_TRIP), sheet(&s1.project, &ROUND_TRIP));
    prop_assert!(a == b, "export → import → export gives the same cells; first difference: {}", first_line_difference(&b, &a));
    // Exported again with every column, the imported plan imports as itself, but for its
    // summaries' rolled-up durations.
    let mut p1 = s1.project.clone();
    p1.columns = Columns::all();
    let x1 = Session::new(p1).export(Format::Csv).unwrap();
    let s2 = Session::import(Format::Csv, x1.as_bytes()).unwrap();
    let leaves = |p: &Project| {
        let outline = Outline::of(p);
        p.tasks.iter().enumerate().filter(|(i, _)| !outline.is_summary(*i)).map(|(_, t)| t.clone()).collect::<Vec<_>>()
    };
    prop_assert_eq!(leaves(&s2.project), leaves(&s1.project));
    prop_assert_eq!(&s2.project.resources, &s1.project.resources);
    Ok(())
}

/// The first line two files differ on.
fn first_line_difference(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let i = la.iter().zip(&lb).position(|(x, y)| x != y).unwrap_or(la.len().min(lb.len()));
    format!("line {}:\n  got  {:?}\n  want {:?}", i + 1, la.get(i), lb.get(i))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn generated_projects_round_trip(seed in any::<u64>()) {
        round_trip(&with_formulas(generated(seed).project, seed))?;
    }

    /// Only the shown columns are written, in the column chooser's order, after ID and Task Name.
    #[test]
    fn hidden_columns_are_not_written(seed in any::<u64>(), picks in proptest::collection::vec(0usize..13, 0..13)) {
        let mut p = generated(seed).project;
        let shown: Vec<Column> = picks.iter().filter_map(|&i| Column::ALL.get(i).copied()).collect();
        p.columns = Columns::new(shown);
        let csv = Session::new(p.clone()).export(Format::Csv).unwrap();
        let heading = csv.trim_start_matches('\u{FEFF}').split("\r\n").next().unwrap_or("").to_owned();
        let mut want = vec!["ID", "Task Name"];
        want.extend(p.columns.shown().iter().map(|c| c.name()));
        if p.tasks.iter().any(|t| t.outline_level > 1) {
            want.push("Outline Level");
        }
        prop_assert_eq!(heading, want.join(","));
        let width = want.len();
        let back = projectcraft_engine::interchange::import(Format::Csv, csv.as_bytes()).unwrap();
        prop_assert!(!back.report.lines.iter().any(|l| l.feature == "Cells past the last column"), "every row is {width} cells: {csv}");
    }
}

#[test]
fn the_sample_round_trips() {
    let s = projectcraft_engine::sample();
    round_trip(&s.project).unwrap();
    // Its packing boxes come back as a material with its label, at the rate it is used.
    let x = s.export(Format::Csv).unwrap();
    let back = Session::import(Format::Csv, x.as_bytes()).unwrap();
    let boxes = back.project.resources.iter().find(|r| r.name == "Packing boxes").unwrap();
    assert_eq!(boxes.kind, projectcraft_engine::model::ResourceType::Material);
}

/// A plan whose dates come only from its links and durations imports with the same Start and
/// Finish: the earliest Start is the project's start.
#[test]
fn dates_come_back_when_nothing_but_links_and_durations_sets_them() {
    let mut s = Session::new(Project::new("Dates", projectcraft_engine::model::Date::from_ymd(2026, 3, 4).unwrap()));
    for (cmd, params) in [
        ("task.add", serde_json::json!({"name": "Dig", "duration": 3})),
        ("task.add", serde_json::json!({"name": "Pour", "duration": "4h"})),
        ("task.add", serde_json::json!({"name": "Cure", "duration": 5})),
        ("task.setPredecessors", serde_json::json!({"id": 2, "text": "1"})),
        ("task.setPredecessors", serde_json::json!({"id": 3, "text": "2FS+1d"})),
    ] {
        s.run(cmd, &params).unwrap();
    }
    let x = s.export(Format::Csv).unwrap();
    let back = Session::import(Format::Csv, x.as_bytes()).unwrap();
    let dates = [Column::Start, Column::Finish];
    assert_eq!(sheet_with_dates(&back, &dates), sheet_with_dates(&s, &dates));
}

fn sheet_with_dates(s: &Session, columns: &[Column]) -> String {
    let mut p = s.project.clone();
    p.columns = Columns::new(columns.iter().copied());
    interchange::export(Format::Csv, &p, s.schedule.as_ref().ok()).unwrap()
}
