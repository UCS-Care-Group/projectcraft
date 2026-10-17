//! MSPDI import against hand-written files built from Microsoft's published schema docs
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/>).

use std::time::{Duration, Instant};

use projectcraft_interchange::model::{Date, DurationUnit, MAX_MINUTES, MAX_TASKS, WorkingTime};
use projectcraft_interchange::{Format, ImportError, Outcome, ReportLine, detect, import};

const GARDEN: &str = include_str!("fixtures/garden-studio.xml");

fn mspdi(text: &str) -> Result<projectcraft_interchange::Import, ImportError> {
    import(Format::Mspdi, text.as_bytes())
}

fn find<'a>(lines: &'a [ReportLine], feature: &str) -> &'a ReportLine {
    lines.iter().find(|l| l.feature == feature).unwrap_or_else(|| panic!("no `{feature}` line in {lines:#?}"))
}

/// A file of tasks, each `(uid, id, name, level, duration)`, with nothing else.
fn file(tasks: &[(i64, i64, &str, i64, &str)]) -> String {
    let mut s = String::from("<Project xmlns=\"http://schemas.microsoft.com/project\"><StartDate>2026-01-05T08:00:00</StartDate><Tasks>");
    for (uid, id, name, level, duration) in tasks {
        s.push_str(&format!(
            "<Task><UID>{uid}</UID><ID>{id}</ID><Name>{name}</Name><OutlineLevel>{level}</OutlineLevel><Duration>{duration}</Duration></Task>"
        ));
    }
    s.push_str("</Tasks></Project>");
    s
}

#[test]
fn a_hand_written_file_imports_names_outline_levels_and_durations() {
    let i = mspdi(GARDEN).unwrap();
    let p = &i.project;
    assert_eq!(p.name, "Garden studio");
    assert_eq!(p.start, Date::from_ymd(2026, 3, 2).unwrap());
    assert_eq!(p.options.minutes_per_week, 2160);
    // The project summary task (UID 0) and the blank row are not imported.
    let rows: Vec<(u32, &str, u32, i64, DurationUnit)> =
        p.tasks.iter().map(|t| (t.id.0, t.name.as_str(), t.outline_level, t.duration, t.duration_unit)).collect();
    assert_eq!(
        rows,
        [
            (1, "Groundworks", 1, 64 * 60, DurationUnit::Days),
            (2, "Mark out & clear the plot", 2, 16 * 60, DurationUnit::Days),
            (3, "Pour the base", 2, 36 * 60, DurationUnit::Hours),
            (5, "Base cured", 2, 0, DurationUnit::Days),
            (6, "Build", 1, 100 * 60, DurationUnit::Days),
            (7, "Frame and roof", 2, 72 * 60, DurationUnit::Weeks),
            (8, "Fit windows", 2, 1200, DurationUnit::Minutes),
            (10, "Handover", 1, 0, DurationUnit::Days),
        ]
    );
    assert_eq!(p.next_id, 11, "Unique IDs are kept, and new tasks follow them");
    assert_eq!(i.report.tasks, 8);
    assert!(p.tasks.iter().all(|t| t.preds.is_empty() && t.constraint.is_asap() && t.deadline.is_none()));
}

#[test]
fn the_project_calendar_s_working_week_is_imported() {
    let c = mspdi(GARDEN).unwrap().project.calendar;
    // DayType 1 is Sunday … 7 Saturday; the calendar is Monday first.
    assert_eq!(c.working, [true, true, true, true, true, false, false]);
    let t = |a, b| WorkingTime::new(a, b).unwrap();
    assert_eq!(c.weekday_times(0), [t(480, 720), t(780, 1020)]);
    assert_eq!(c.weekday_times(4), [t(480, 720)], "Friday is a half day");
    let holiday = Date::from_ymd(2026, 4, 3).unwrap();
    assert_eq!(c.exceptions.iter().collect::<Vec<_>>(), [(&holiday, &false)], "the spring holiday is a day off");
}

#[test]
fn everything_else_is_a_report_line() {
    let r = mspdi(GARDEN).unwrap().report;
    let l = &r.lines;
    let expect = [
        ("Project summary task", Outcome::Dropped, "UID 0"),
        ("Blank rows", Outcome::Dropped, "1 row"),
        ("Links (predecessors)", Outcome::NotYetImported, "ID 3 Pour the base, ID 4 Base cured"),
        ("Constraint", Outcome::NotYetImported, "ID 3 Pour the base"),
        ("Deadline", Outcome::NotYetImported, "ID 4 Base cured"),
        ("Priority", Outcome::Dropped, "ID 3 Pour the base"),
        ("Notes", Outcome::Dropped, "ID 6 Frame and roof"),
        ("Estimated duration (?)", Outcome::Dropped, "ID 6 Frame and roof"),
        ("Progress (actuals, % complete)", Outcome::Dropped, "ID 7 Fit windows"),
        ("Base calendar", Outcome::Dropped, "Night shift (UID 2)"),
        ("Resource calendars", Outcome::NotYetImported, "1 calendar"),
        ("Resources", Outcome::NotYetImported, "1 resource"),
        ("Assignments", Outcome::NotYetImported, "1 assignment"),
        ("Currency", Outcome::NotYetImported, "EUR"),
    ];
    for (feature, outcome, place) in expect {
        let line = find(l, feature);
        assert_eq!((line.outcome, line.place.as_str()), (outcome, place), "{feature}");
        assert!(!line.why.is_empty());
    }
    assert_eq!(l.len(), expect.len(), "{l:#?}");
    // Converted first, then dropped, then not yet imported.
    assert!(l.windows(2).all(|w| w[0].outcome <= w[1].outcome));
    let text = r.text();
    assert!(text.starts_with("Imported 8 tasks from Microsoft Project XML."), "{text}");
    assert!(text.contains("Not yet imported: Links (predecessors) (ID 3 Pour the base, ID 4 Base cured): links come"), "{text}");
}

#[test]
fn a_bare_file_without_namespace_ids_or_calendar_imports_with_defaults() {
    let i = mspdi("<Project><Tasks><Task><UID>4</UID><Name>Only</Name></Task></Tasks></Project>").unwrap();
    assert_eq!(i.project.name, "Imported project");
    assert_eq!(i.project.tasks.len(), 1);
    assert_eq!(i.project.tasks[0].duration, 480, "no duration: 1 day, as a new task in Project");
    assert_eq!(i.project.calendar, Default::default());
    for f in ["Project start", "Duration", "Project calendar"] {
        assert_eq!(find(&i.report.lines, f).outcome, Outcome::Converted, "{f}");
    }
}

#[test]
fn levels_that_break_the_outline_rule_are_moved_up_and_reported() {
    let i = mspdi(&file(&[(1, 1, "A", 1, "PT8H"), (2, 2, "B", 3, "PT8H"), (3, 3, "C", 0, "PT8H"), (4, 4, "D", 999, "PT8H")])).unwrap();
    let levels: Vec<u32> = i.project.tasks.iter().map(|t| t.outline_level).collect();
    assert_eq!(levels, [1, 2, 1, 2]);
    assert_eq!(find(&i.report.lines, "Outline level").place, "ID 2 B, ID 4 D");
}

#[test]
fn rows_follow_the_file_s_ids_and_bad_unique_ids_are_renumbered() {
    let i = mspdi(&file(&[(7, 2, "Second", 1, "PT8H"), (7, 1, "First", 1, "PT8H")])).unwrap();
    let rows: Vec<(u32, &str)> = i.project.tasks.iter().map(|t| (t.id.0, t.name.as_str())).collect();
    assert_eq!(rows, [(1, "First"), (2, "Second")]);
    assert_eq!(i.project.next_id, 3);
    assert_eq!(find(&i.report.lines, "Unique IDs").outcome, Outcome::Converted);
    let big = mspdi(&file(&[(i64::from(u32::MAX), 1, "Huge", 1, "PT8H")])).unwrap();
    assert_eq!(big.project.tasks[0].id.0, 1);
}

#[test]
fn durations_are_clamped_to_the_model_and_bad_ones_reported() {
    let i = mspdi(&file(&[(1, 1, "Long", 1, "PT99999999H"), (2, 2, "Negative", 1, "-PT8H"), (3, 3, "Months", 1, "P2M")])).unwrap();
    let d: Vec<i64> = i.project.tasks.iter().map(|t| t.duration).collect();
    assert_eq!(d, [MAX_MINUTES, 0, 480]);
    let durations: Vec<&str> = i.report.lines.iter().filter(|l| l.feature == "Duration").map(|l| l.place.as_str()).collect();
    assert_eq!(durations, ["ID 3 Months", "ID 2 Negative", "ID 1 Long"]);
}

#[test]
fn more_tasks_than_a_project_holds_are_refused() {
    let tasks: Vec<(i64, i64, &str, i64, &str)> = (1..=MAX_TASKS as i64 + 1).map(|n| (n, n, "T", 1, "PT8H")).collect();
    assert_eq!(mspdi(&file(&tasks)).unwrap_err(), ImportError::TooManyTasks);
    // The project summary task doesn't count.
    let mut ok: Vec<(i64, i64, &str, i64, &str)> = (1..=MAX_TASKS as i64).map(|n| (n, n, "T", 1, "PT8H")).collect();
    ok.insert(0, (0, 0, "Summary", 0, "PT8H"));
    assert_eq!(mspdi(&file(&ok)).unwrap().project.tasks.len(), MAX_TASKS);
}

#[test]
fn other_xml_is_not_mspdi() {
    assert_eq!(detect(GARDEN.as_bytes()), Some(Format::Mspdi));
    assert_eq!(detect(format!("\u{feff}{GARDEN}").as_bytes()), Some(Format::Mspdi));
    for (text, err) in [
        ("<html><body/></html>", ImportError::NotMspdi("html".into())),
        (
            "<Project xmlns=\"http://schemas.microsoft.com/developer/msbuild/2003\"/>",
            ImportError::NotMspdi("Project in the namespace http://schemas.microsoft.com/developer/msbuild/2003".into()),
        ),
    ] {
        assert_eq!(detect(text.as_bytes()), None);
        assert_eq!(mspdi(text).unwrap_err(), err);
    }
    assert_eq!(detect(b"\x00\x01binary"), None);
    assert!(matches!(mspdi("{\"name\": \"json\"}"), Err(ImportError::Xml(_))));
}

#[test]
fn billion_laughs_and_depth_bombs_are_refused_fast() {
    let laughs = r#"<?xml version="1.0"?>
<!DOCTYPE Project [
 <!ENTITY a "aaaaaaaaaa">
 <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
 <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
 <!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
 <!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;">
 <!ENTITY f "&e;&e;&e;&e;&e;&e;&e;&e;&e;&e;">
 <!ENTITY g "&f;&f;&f;&f;&f;&f;&f;&f;&f;&f;">
 <!ENTITY h "&g;&g;&g;&g;&g;&g;&g;&g;&g;&g;">
 <!ENTITY i "&h;&h;&h;&h;&h;&h;&h;&h;&h;&h;">
]>
<Project xmlns="http://schemas.microsoft.com/project"><Tasks><Task><UID>1</UID><Name>&i;</Name></Task></Tasks></Project>"#;
    let t = Instant::now();
    assert_eq!(mspdi(laughs).unwrap_err(), ImportError::Dtd);
    // An undeclared entity is refused too, even without a DTD.
    let entity = "<Project><Tasks><Task><UID>1</UID><Name>&i;</Name></Task></Tasks></Project>";
    assert_eq!(mspdi(entity).unwrap_err(), ImportError::Entity("i".into()));
    let bomb = format!("<Project>{}", "<Tasks>".repeat(10_000_000));
    assert_eq!(mspdi(&bomb).unwrap_err(), ImportError::TooDeep);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(import(Format::Mspdi, &vec![b' '; projectcraft_interchange::MAX_IMPORT_BYTES + 1]).unwrap_err(), ImportError::TooLarge);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..2048)) {
        let _ = import(Format::Mspdi, &bytes);
        let _ = detect(&bytes);
    }

    #[test]
    fn a_damaged_file_never_panics(at in 0usize..GARDEN.len(), cut in 0usize..64, junk in "\\PC{0,24}") {
        // Splice junk into the hand-written file, so the damage lands inside real MSPDI.
        let mut text = String::new();
        let at = (0..=at).rev().find(|i| GARDEN.is_char_boundary(*i)).unwrap_or(0);
        let end = (at..=at.saturating_add(cut).min(GARDEN.len())).rev().find(|i| GARDEN.is_char_boundary(*i)).unwrap_or(at);
        text.push_str(&GARDEN[..at]);
        text.push_str(&junk);
        text.push_str(&GARDEN[end..]);
        if let Ok(i) = mspdi(&text) {
            proptest::prop_assert!(i.project.tasks.len() <= MAX_TASKS);
            proptest::prop_assert!(i.project.tasks.iter().all(|t| (0..=MAX_MINUTES).contains(&t.duration)));
        }
    }
}
