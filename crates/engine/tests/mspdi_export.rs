//! Microsoft Project XML export round trips: export → import → export is byte for byte the same,
//! and import → export → import gives the same project, over generated projects (every field
//! the export writes, with hostile text) and the import fixtures.

mod common;

use common::generated;
use projectcraft_engine::Session;
use projectcraft_engine::interchange::Format;
use projectcraft_engine::model::{Date, DurationUnit, Link, LinkKind, Project, ResourceType};
use proptest::prelude::*;

/// What an export → import keeps of `like`: everything but the counters and views the file has
/// no place for, with names trimmed of spaces at their ends, as the importer reads every name.
fn comparable(mut p: Project, like: &Project) -> Project {
    for (t, l) in p.tasks.iter_mut().zip(&like.tasks) {
        if t.name == l.name.trim() {
            t.name = l.name.clone();
        }
    }
    // A project without a name is written with the name an import gives one.
    if p.name == like.name.trim() || (like.name.trim().is_empty() && p.name == "Imported project") {
        p.name = like.name.clone();
    }
    // MSPDI holds an assignment's work and units, not how long it lasts: that is worked out again
    // as work ÷ units (or the task's duration when the work fits it), which can be a minute from
    // what the project held when its work was rounded to the minute.
    for (a, l) in p.assignments.iter_mut().zip(&like.assignments) {
        if a.units == l.units && a.work == l.work && (a.duration - l.duration).abs() <= 1 {
            a.duration = l.duration;
        }
    }
    // A resource's own weekday the same as the project's reads back as following the project's.
    for (r, l) in p.resources.iter_mut().zip(&like.resources) {
        for (i, (day, want)) in r.calendar.week.iter_mut().zip(&l.calendar.week).enumerate() {
            if day.is_none() && want.as_deref() == Some(like.calendar.weekday_times(i)) {
                *day = want.clone();
            }
        }
    }
    p.next_id = like.next_id;
    p.next_resource_id = like.next_resource_id;
    p.columns = like.columns.clone();
    p.options.units_format = like.options.units_format;
    p
}

/// The first part of two projects that differs, to read a failure by.
fn difference(a: &Project, b: &Project) -> String {
    let parts = |p: &Project| -> Vec<String> {
        let mut v = vec![format!("{:?} {:?} {:?} {:?}", p.name, p.start, p.options, p.currency), format!("{:?}", p.calendar)];
        v.extend(p.tasks.iter().map(|t| format!("{t:?}")));
        v.extend(p.resources.iter().map(|r| format!("{r:?}")));
        v.extend(p.assignments.iter().map(|x| format!("{x:?}")));
        v
    };
    let (pa, pb) = (parts(a), parts(b));
    pa.iter()
        .zip(&pb)
        .find(|(x, y)| x != y)
        .map_or_else(|| format!("{} parts vs {}", pa.len(), pb.len()), |(x, y)| format!("\n  got  {x}\n  want {y}"))
}

/// The first line two files differ on, with the lines before it.
fn first_line_difference(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let i = la.iter().zip(&lb).position(|(x, y)| x != y).unwrap_or(la.len().min(lb.len()));
    let from = i.saturating_sub(12);
    format!("line {}:\n{}\n  got  {:?}\n  want {:?}", i + 1, lb[from..i].join("\n"), la.get(i), lb.get(i))
}

fn round_trip(s0: &Session) -> Result<(), TestCaseError> {
    let x0 = s0.export(Format::Mspdi).unwrap();
    let s1 = Session::import(Format::Mspdi, x0.as_bytes()).unwrap();
    let report = s1.import_report().unwrap();
    prop_assert!(report.lines.is_empty(), "{}\n{x0}", report.text());
    let back = comparable(s1.project.clone(), &s0.project);
    prop_assert!(back == s0.project, "export → import is the same project: {}\n{x0}", difference(&back, &s0.project));
    let x1 = s1.export(Format::Mspdi).unwrap();
    prop_assert!(x1 == x0, "export → import → export is byte for byte the same; first difference: {}", first_line_difference(&x1, &x0));
    let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
    prop_assert_eq!(&s2.project, &s1.project, "import → export → import is the same project");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn generated_projects_round_trip_exactly(seed in any::<u64>()) {
        round_trip(&generated(seed))?;
    }
}

/// Plans that once didn't round trip: an assignment's duration a minute from its work ÷ units
/// (53 minutes at 50%; an effort-driven Fixed Work task's work shared out rounded), material
/// and cost resources, and every kind of text.
#[test]
fn plans_that_once_failed_round_trip() {
    for seed in [12171069806144644276, 12171255513429957426, 16235999543351105870, 2118350416455198788, 9653999539, 5239053456218871172] {
        round_trip(&generated(seed)).unwrap();
    }
}

/// A resource weekday set to the project's own times (Saturday off, when the project has
/// Saturdays off) is written as following the project calendar, as the importer reads one: the
/// resource has no calendar of its own after the round trip, and from then on the project
/// imports and exports unchanged.
#[test]
fn a_resource_weekday_like_the_project_s() {
    let mut p = Project::new("Weekdays", Date::from_ymd(2026, 3, 2).unwrap());
    let t = p.add_task("Build", 960).unwrap();
    let r = p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
    p.set_resource_weekday_times(r, 5, Some(vec![])).unwrap();
    p.assign(t, r, None).unwrap();
    let s0 = Session::new(p);
    let x0 = s0.export(Format::Mspdi).unwrap();
    assert!(!x0.contains("<CalendarUID>2</CalendarUID>"), "{x0}");
    let s1 = Session::import(Format::Mspdi, x0.as_bytes()).unwrap();
    assert!(s1.project.resources[0].calendar.is_empty());
    let x1 = s1.export(Format::Mspdi).unwrap();
    let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
    assert_eq!(s2.project, s1.project);
    assert_eq!(s2.export(Format::Mspdi).unwrap(), x1);
}

#[test]
fn the_import_fixtures_round_trip() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../interchange/tests/fixtures");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "xml") {
            continue;
        }
        seen += 1;
        let s1 = Session::import_file(&path, None).unwrap();
        let x1 = s1.export(Format::Mspdi).unwrap();
        let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
        assert_eq!(s2.project, s1.project, "{}: import → export → import", path.display());
        assert!(s2.import_report().unwrap().lines.is_empty(), "{}: {}", path.display(), s2.import_report().unwrap().text());
        assert_eq!(s2.export(Format::Mspdi).unwrap(), x1, "{}: export → import → export", path.display());
    }
    assert!(seen >= 2, "the fixtures were found");
}

#[test]
fn a_project_that_cannot_be_scheduled_still_exports_without_dates() {
    let mut p = Project::new("Loop", Date::from_ymd(2026, 3, 2).unwrap());
    let a = p.add_task("A", 480).unwrap();
    let b = p.add_task("B", 480).unwrap();
    p.set_preds(b, vec![Link { pred: a, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days }]).unwrap();
    // A loop only a damaged file could hold.
    p.task_mut(a).unwrap().preds.push(Link { pred: b, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days });
    let s = Session::new(p);
    assert!(s.schedule.is_err());
    let x = s.export(Format::Mspdi).unwrap();
    assert!(!x.contains("<Start>") && !x.contains("<TotalSlack>"), "{x}");
    assert!(x.contains("<PredecessorUID>2</PredecessorUID>"));
}
