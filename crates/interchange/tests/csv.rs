//! CSV import: a heading row matched to the task sheet's column names, then a task a row.

use std::time::{Duration, Instant};

use projectcraft_interchange::model::{Constraint, Date, DurationUnit, LinkKind, MAX_MINUTES, MAX_TASKS, Project, TaskId, TaskType};
use projectcraft_interchange::{Format, ImportError, Outcome, ReportLine, import};

/// A kitchen refit, saved by a spreadsheet as "CSV UTF-8" with semicolons: a BOM, CRLF line
/// ends, quoted cells holding the separator and doubled quotes.
const KITCHEN: &[u8] = include_bytes!("fixtures/kitchen-refit.csv");

fn csv(text: &str) -> Result<projectcraft_interchange::Import, ImportError> {
    import(Format::Csv, text.as_bytes())
}

fn lines_about<'a>(lines: &'a [ReportLine], feature: &str) -> Vec<&'a ReportLine> {
    lines.iter().filter(|l| l.feature == feature).collect()
}

fn find<'a>(lines: &'a [ReportLine], feature: &str) -> &'a ReportLine {
    lines.iter().find(|l| l.feature == feature).unwrap_or_else(|| panic!("no `{feature}` line in {lines:#?}"))
}

fn date(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

fn names(p: &Project) -> Vec<&str> {
    p.tasks.iter().map(|t| t.name.as_str()).collect()
}

#[test]
fn a_file_with_a_bom_and_semicolons_imports() {
    assert!(KITCHEN.starts_with(b"\xEF\xBB\xBF"), "the fixture has a BOM");
    let i = import(Format::Csv, KITCHEN).unwrap();
    let p = &i.project;
    assert_eq!(
        names(p),
        [
            "Kitchen refit",
            "Strip out the old units",
            "Plumbing; first fix",
            "Plaster the walls",
            "Fit units, worktops and \"soft-close\" doors",
            "Kitchen ready"
        ]
    );
    assert_eq!(i.report.format, Format::Csv);
    assert_eq!(i.report.tasks, 6);
    let ids: Vec<u32> = p.tasks.iter().map(|t| t.id.0).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
    assert_eq!(p.next_id, 7);
    let levels: Vec<u32> = p.tasks.iter().map(|t| t.outline_level).collect();
    assert_eq!(levels, [1, 2, 2, 2, 2, 1]);
    let durations: Vec<(i64, DurationUnit)> = p.tasks.iter().map(|t| (t.duration, t.duration_unit)).skip(1).collect();
    assert_eq!(
        durations,
        [(960, DurationUnit::Days), (720, DurationUnit::Days), (1440, DurationUnit::Days), (2400, DurationUnit::Weeks), (0, DurationUnit::Days)]
    );
    // Predecessors are by row: row 3's `3FS+1d` is the third row's task.
    let t4 = &p.tasks[3];
    assert_eq!(t4.preds.len(), 1);
    assert_eq!((t4.preds[0].pred, t4.preds[0].kind, t4.preds[0].lag), (TaskId(3), LinkKind::FS, 480));
    assert_eq!(p.tasks[5].preds[0].pred, TaskId(5));
    assert_eq!(p.tasks[2].constraint, Constraint::StartNoEarlierThan(date(2026, 11, 9)));
    assert_eq!(p.tasks[5].constraint, Constraint::MustFinishOn(date(2026, 12, 4)));
    assert_eq!(t4.deadline, Some(date(2026, 11, 20)));
    // Resource Names make work resources, once each, and assign them with their units.
    let res: Vec<&str> = p.resources.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(res, ["Dev Patel", "Skip hire", "Rosa Lind"]);
    assert_eq!(p.resource_names(TaskId(2)), "Dev Patel, Skip hire[50%]");
    assert_eq!(p.resource_names(TaskId(5)), "Dev Patel[50%], Rosa Lind");
    assert_eq!(p.resource_names(TaskId(4)), "Dev Patel");
    // A CSV has no project start: the earliest constraint date is used, and the report says so.
    assert_eq!(p.start, date(2026, 11, 9));
    let start = find(&i.report.lines, "Project start");
    assert!(start.why.contains("2026-11-09"), "{start:?}");
    // The ID column is the row number, read without a line; Notes is reported.
    assert!(lines_about(&i.report.lines, "ID column").is_empty());
    let notes = find(&i.report.lines, "Notes column");
    assert_eq!((notes.outcome, notes.place.as_str()), (Outcome::Dropped, "Column J"));
    assert!(model_schedules(p));
}

fn model_schedules(p: &Project) -> bool {
    projectcraft_interchange::model::schedule(p).is_ok()
}

#[test]
fn a_comma_file_without_a_bom_imports_and_headings_match_any_case() {
    let i = csv("task name,DURATION,predecessors\nDig,2d,\nPour,4h,1\n").unwrap();
    assert_eq!(names(&i.project), ["Dig", "Pour"]);
    assert_eq!(i.project.tasks[1].duration, 240);
    assert_eq!(i.project.tasks[1].preds[0].pred, TaskId(1));
}

#[test]
fn a_file_without_a_task_name_column_is_refused_with_a_clear_error() {
    for text in ["Name of task;Duration\nDig;2\n", "Duration\n2\n", "", "\u{feff}", "\n\n"] {
        let e = csv(text).err().unwrap_or_else(|| panic!("{text:?} imported"));
        assert_eq!(e, ImportError::NoTaskName, "{text:?}");
        assert!(e.to_string().contains("Task Name"), "{e}");
    }
}

#[test]
fn every_unknown_column_has_its_own_report_line() {
    let i = csv("Task Name,Notes,Work,Cost,Finish,,Duration\nDig,deep,8h,£5.00,2026-01-01,x,1\n").unwrap();
    let dropped: Vec<(&str, &str)> =
        i.report.lines.iter().filter(|l| l.feature.ends_with("column")).map(|l| (l.feature.as_str(), l.place.as_str())).collect();
    assert_eq!(
        dropped,
        [
            ("Notes column", "Column B"),
            ("Work column", "Column C"),
            ("Cost column", "Column D"),
            ("Finish column", "Column E"),
            ("Untitled column", "Column F")
        ]
    );
    assert!(find(&i.report.lines, "Finish column").why.contains("works out"), "dates come from the scheduler");
    assert!(find(&i.report.lines, "Work column").why.contains("works out"), "work comes from duration and resources");
    assert!(find(&i.report.lines, "Cost column").why.contains("works out"), "costs are worked out");
    // A column with no heading and nothing in it (a spreadsheet's trailing separators) is no news.
    let i = csv("Task Name;Duration;;\nDig;1;;\n").unwrap();
    assert!(i.report.lines.is_empty() || i.report.lines.iter().all(|l| l.feature == "Project start"), "{:#?}", i.report.lines);
}

#[test]
fn a_repeated_column_reads_the_first() {
    let i = csv("Task Name,Duration,Duration\nDig,2,5\n").unwrap();
    assert_eq!(i.project.tasks[0].duration, 960);
    let l = find(&i.report.lines, "Duration column");
    assert_eq!(l.place, "Column C");
}

#[test]
fn bad_cells_are_report_lines_not_failures() {
    let text = "Task Name,Duration,Predecessors,Outline Level,Constraint Type,Constraint Date,Deadline\n\
        A,soon,,1,,,\n\
        B,3 edays,9,x,Start No Earlier Than,,\n\
        C,1,1;2;1,1,As Late As Possible,,31/12/2026\n\
        D,2d?,zz,1,Sometime,2026-01-01,\n\
        E,1,5,1,FNLT,next week,\n";
    let i = csv(text).unwrap();
    let p = &i.project;
    assert_eq!(p.tasks.len(), 5);
    let day = 480;
    assert_eq!(p.tasks[0].duration, day, "unreadable: 1 day");
    assert_eq!(p.tasks[1].duration, day, "elapsed: 1 day");
    assert_eq!(p.tasks[3].duration, 960, "the estimate mark is dropped, the duration kept");
    assert!(p.tasks[1].preds.is_empty(), "row 9 isn't a task");
    assert_eq!(p.tasks[2].preds.iter().map(|l| l.pred.0).collect::<Vec<_>>(), [1, 2], "the repeat is dropped");
    assert!(p.tasks[3].preds.is_empty());
    assert!(p.tasks[4].preds.is_empty(), "a task can't be its own predecessor");
    assert_eq!(p.tasks[2].constraint, Constraint::AsLateAsPossible, "As Late As Possible comes in as it is");
    assert!(p.tasks.iter().enumerate().all(|(k, t)| (k == 2 || t.constraint.is_asap()) && t.deadline.is_none()));
    let r = &i.report.lines;
    let place = |f: &str| find(r, f).place.clone();
    assert_eq!(place("Duration"), "Row 1 A, Row 2 B");
    assert_eq!(place("Estimated duration (?)"), "Row 4 D");
    assert_eq!(place("Predecessors"), "Row 4 D");
    assert!(lines_about(r, "Links (predecessors)").iter().any(|l| l.place == "Row 2 B" && l.why.contains("isn't a task")), "{r:#?}");
    assert!(lines_about(r, "Links (predecessors)").iter().any(|l| l.place == "Row 3 C" && l.why.contains("twice")), "{r:#?}");
    assert!(lines_about(r, "Links (predecessors)").iter().any(|l| l.place == "Row 5 E" && l.why.contains("its own predecessor")), "{r:#?}");
    assert_eq!(place("Outline Level"), "Row 2 B");
    assert!(lines_about(r, "As Late As Possible").is_empty(), "{r:#?}");
    assert_eq!(place("Constraint Type"), "Row 4 D");
    assert!(lines_about(r, "Constraint Date").iter().any(|l| l.place == "Row 2 B"), "{r:#?}");
    assert!(lines_about(r, "Constraint Date").iter().any(|l| l.place == "Row 5 E"), "{r:#?}");
    assert_eq!(place("Deadline"), "Row 3 C");
    assert!(model_schedules(p));
}

#[test]
fn outline_levels_are_kept_to_the_outline_rule_and_links_into_a_summary_s_own_subtasks_dropped() {
    let i = csv("Task Name,Outline Level,Predecessors\nTop,1,\nDeep,3,1\nNext,2,\nOut,0,\nHuge,999999999999999999999,\n").unwrap();
    let levels: Vec<u32> = i.project.tasks.iter().map(|t| t.outline_level).collect();
    assert_eq!(levels, [1, 2, 2, 1, 1], "unreadable or out of range: level 1");
    assert!(i.project.tasks[1].preds.is_empty(), "a subtask can't link to its own summary");
    assert_eq!(find(&i.report.lines, "Outline level").place, "Row 2 Deep");
    assert_eq!(find(&i.report.lines, "Outline Level").place, "Row 4 Out, Row 5 Huge");
    assert!(model_schedules(&i.project));
}

#[test]
fn links_that_close_a_loop_are_dropped_and_the_rest_kept() {
    // 1 → 2 → 3 → 1 is a loop. Read in row order, as typing them would be, row 3's link closes it.
    let i = csv("Task Name,Predecessors\nA,3\nB,1\nC,2\nD,3\n").unwrap();
    let preds: Vec<Vec<u32>> = i.project.tasks.iter().map(|t| t.preds.iter().map(|l| l.pred.0).collect()).collect();
    assert_eq!(preds, [vec![3], vec![1], vec![], vec![3]]);
    assert!(lines_about(&i.report.lines, "Links (predecessors)").iter().any(|l| l.place == "Row 3 C" && l.why.contains("loop")));
    assert!(model_schedules(&i.project));
    // Through a summary: B is under A, and A's predecessor is C, which follows B.
    let i = csv("Task Name,Outline Level,Predecessors\nA,1,3\nB,2,\nC,1,2\n").unwrap();
    assert!(model_schedules(&i.project), "{:?}", i.project.tasks);
}

#[test]
fn quoted_cells_hold_separators_quotes_and_line_breaks() {
    let text = "\"Task Name\",\"Duration\"\r\n\"Say \"\"hi\"\"\",\"2\"\r\n\"Line\none\",1\rLast,3";
    let i = csv(text).unwrap();
    assert_eq!(names(&i.project), ["Say \"hi\"", "Line one", "Last"]);
    assert_eq!(find(&i.report.lines, "Task Name").place, "Row 2 Line one");
    assert_eq!(i.project.tasks[2].duration, 3 * 480);
}

#[test]
fn a_quoted_task_name_keeps_its_spaces_and_others_are_trimmed() {
    let i = csv("Task Name,Resource Names\n\"  Dig \" ,\" Ann \"\n  Pour  ,Bob\n\"  \",Cy\n").unwrap();
    assert_eq!(names(&i.project), ["  Dig ", "Pour", "  "], "padding after the closing quote isn't the name's");
    assert_eq!(i.project.resources.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Ann", "Bob", "Cy"]);
    assert!(lines_about(&i.report.lines, "Task Name").is_empty(), "a name of spaces, quoted, is a name");
    let i = csv("Task Name,Resource Names\n\"\",Cy\n").unwrap();
    assert_eq!(find(&i.report.lines, "Task Name").place, "Row 1");
}

/// The CSV export puts an apostrophe in front of a Task Name or Resource Names cell that a
/// spreadsheet would run as a formula; the import takes it off again, and leaves every other
/// apostrophe as it is.
#[test]
fn an_apostrophe_guarding_a_formula_is_taken_off() {
    let i = csv("Task Name,Resource Names,Duration\n'=1+1,'@Ann,1\n'-2 days early,,1\n\"'+44, call\",,1\n'Tis the season,,1\n''=x,,1\n").unwrap();
    assert_eq!(names(&i.project), ["=1+1", "-2 days early", "+44, call", "'Tis the season", "'=x"]);
    assert_eq!(i.project.resources.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["@Ann"]);
}

#[test]
fn fixed_cost_type_effort_driven_priority_and_leveling_delay_are_read() {
    let text = "Task Name,Fixed Cost,Type,Effort Driven,Priority,Leveling Delay,Outline Level\n\
        Build,£1.00,Fixed Work,No,10,2d,1\n\
        Dig,\"£1,250.50\",fixed duration,yes,1000,4h,2\n\
        Pour,,Fixed Work,No,0,1.5d,2\n\
        Cure,lots,Fixed Speed,maybe,1001,soon,1\n\
        Tidy,£0.00,Fixed Duration,No,500,0d,1\n";
    let i = csv(text).unwrap();
    let t = &i.project.tasks;
    let defaults = &i.project.options;
    assert_eq!(t.iter().map(|t| t.fixed_cost).collect::<Vec<_>>(), [100, 125_050, 0, 0, 0], "a summary keeps its own fixed cost");
    // A summary's type, effort driven and delay follow its subtasks: the defaults, and a line.
    assert_eq!((t[0].task_type, t[0].effort_driven, t[0].leveling_delay), (defaults.task_type, defaults.effort_driven, 0));
    assert_eq!((t[1].task_type, t[1].effort_driven, t[1].leveling_delay), (TaskType::FixedDuration, true, 240));
    assert_eq!((t[2].task_type, t[2].effort_driven, t[2].leveling_delay), (TaskType::FixedWork, true, 720), "Fixed Work is always effort driven");
    assert_eq!((t[3].task_type, t[3].effort_driven, t[3].leveling_delay), (defaults.task_type, defaults.effort_driven, 0));
    assert_eq!(t.iter().map(|t| t.priority).collect::<Vec<_>>(), [10, 1000, 0, 500, 500]);
    let r = &i.report.lines;
    for feature in ["Fixed Cost", "Type", "Effort Driven", "Priority", "Leveling Delay"] {
        assert_eq!(find(r, feature).place, "Row 4 Cure", "{feature}");
    }
    assert_eq!(find(r, "Summary task fields").place, "Row 1 Build");
    assert!(!r.iter().any(|l| l.feature.ends_with("column")), "every column is read: {r:#?}");
    // A summary showing what the sheet shows for one (Fixed Duration, No, no delay) is no news.
    let i = csv("Task Name,Type,Effort Driven,Leveling Delay,Outline Level\nBuild,Fixed Duration,No,,1\nDig,Fixed Units,No,0d,2\n").unwrap();
    assert!(lines_about(&i.report.lines, "Summary task fields").is_empty(), "{:#?}", i.report.lines);
}

#[test]
fn the_currency_is_the_one_the_amounts_are_written_in() {
    let i = csv("Task Name,Fixed Cost,Resource Names\nDig,$1.50,Crane\nPour,\"$2,000.00\",\n").unwrap();
    assert_eq!(i.project.currency.code, "USD");
    assert_eq!(i.project.tasks.iter().map(|t| t.fixed_cost).collect::<Vec<_>>(), [150, 200_000]);
    assert!(find(&i.report.lines, "Currency").why.contains("USD"));
    let i = csv("Task Name,Cost,Fixed Cost\nDig,¥1235,¥1\n").unwrap();
    assert_eq!((i.project.currency.code.as_str(), i.project.tasks[0].fixed_cost), ("JPY", 1));
    // The default's symbol, no symbol, or one ProjectCraft doesn't know: the default, no line.
    for text in ["Task Name,Fixed Cost\nDig,£1.50\n", "Task Name,Fixed Cost\nDig,1.50\n", "Task Name,Fixed Cost\nDig,Q1.50\n"] {
        let i = csv(text).unwrap();
        assert_eq!(i.project.currency, Default::default(), "{text}");
        assert!(lines_about(&i.report.lines, "Currency").is_empty());
    }
}

#[test]
fn a_quote_left_open_reads_to_the_end_of_the_file_and_says_so() {
    let i = csv("Task Name,Duration\nA,1\n\"B,2\nC,3\n").unwrap();
    assert_eq!(i.project.tasks.len(), 2);
    assert_eq!(find(&i.report.lines, "Quotes").place, "Row 2");
}

#[test]
fn blank_rows_are_skipped_but_still_count_for_predecessors() {
    let i = csv("Task Name,Predecessors\nA,\n,\n\nC,1\nD,4\n\n\n").unwrap();
    assert_eq!(names(&i.project), ["A", "C", "D"]);
    // Row 4 (C) is the third task; D's predecessor `4` is C.
    assert_eq!(i.project.tasks[2].preds[0].pred, i.project.tasks[1].id);
    assert_eq!(i.project.tasks[1].preds[0].pred, i.project.tasks[0].id);
    assert_eq!(find(&i.report.lines, "Blank rows").place, "2 rows");
}

#[test]
fn european_decimal_commas_read_in_a_semicolon_file() {
    let i = csv("Task Name;Duration\nA;1,5 days\n").unwrap();
    assert_eq!(i.project.tasks[0].duration, 720);
}

#[test]
fn the_project_starts_on_the_earliest_start_given() {
    let i = csv("Task Name,Start,Finish\nA,2026-03-04 08:00,2026-03-05\nB,2026-03-02T08:00:00,\n").unwrap();
    assert_eq!(i.project.start, date(2026, 3, 2));
    assert!(find(&i.report.lines, "Start column").why.contains("works out"));
    let i = csv("Task Name\nA\n").unwrap();
    assert_eq!(i.project.start, Date::EPOCH);
    assert!(find(&i.report.lines, "Project start").why.contains("1970-01-01"));
}

#[test]
fn resource_names_that_cannot_be_read_are_report_lines() {
    let i = csv("Task Name,Resource Names\nA,\"Ann[50%\"\nB,\"Ann, ann\"\nC,Ann[25%]\n").unwrap();
    assert_eq!(i.project.resources.len(), 1);
    assert_eq!(i.project.resource_names(TaskId(3)), "Ann[25%]");
    assert_eq!(find(&i.report.lines, "Resource Names").place, "Row 1 A, Row 2 B");
}

#[test]
fn not_utf8_is_refused() {
    assert_eq!(import(Format::Csv, b"Task Name\n\xff\xfe\n").err(), Some(ImportError::CsvNotUtf8));
    // UTF-16, as some spreadsheets save "Unicode text".
    assert_eq!(import(Format::Csv, b"\xff\xfeT\0a\0").err(), Some(ImportError::CsvNotUtf8));
}

#[test]
fn more_tasks_than_a_project_holds_is_refused() {
    let mut text = String::from("Task Name\n");
    for _ in 0..=MAX_TASKS {
        text.push_str("t\n");
    }
    assert_eq!(csv(&text).err(), Some(ImportError::TooManyTasks));
}

#[test]
fn durations_are_clamped() {
    let i = csv("Task Name,Duration\nA,99999999999999d\n").unwrap();
    assert_eq!(i.project.tasks[0].duration, MAX_MINUTES);
    assert!(find(&i.report.lines, "Duration").why.contains("100,000 days"));
}

#[test]
fn hostile_shapes_finish_quickly() {
    let t = Instant::now();
    // A heading row of a million separators, a row of a million cells, a million blank rows,
    // and a long chain of links closing one big loop.
    let mut text = format!("Task Name{}\n", ",".repeat(1_000_000));
    text.push_str(&format!("A{}\n", ",x".repeat(1_000_000)));
    text.push_str(&"\n".repeat(1_000_000));
    let i = csv(&text).unwrap();
    assert_eq!(i.project.tasks.len(), 1);
    let mut chain = String::from("Task Name,Predecessors\n");
    let n = 20_000;
    chain.push_str(&format!("T1,{n}\n"));
    for k in 2..=n {
        chain.push_str(&format!("T{k},{}\n", k - 1));
    }
    let i = csv(&chain).unwrap();
    assert!(model_schedules(&i.project));
    assert_eq!(i.project.tasks.iter().filter(|t| t.preds.is_empty()).count(), 1);
    // Every task naming every other task's resource: one shared crew of names.
    let mut crew = String::from("Task Name,Resource Names\n");
    let people: Vec<String> = (0..100).map(|k| format!("P{k}")).collect();
    let all = people.join(", ");
    for k in 0..1_000 {
        crew.push_str(&format!("T{k},\"{all}\"\n"));
    }
    let i = csv(&crew).unwrap();
    assert_eq!(i.project.resources.len(), 100);
    assert_eq!(i.project.assignments.len(), 100_000);
    assert!(t.elapsed() < Duration::from_secs(60), "took {:?}", t.elapsed());
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..2048)) {
        let _ = import(Format::Csv, &bytes);
    }

    #[test]
    fn arbitrary_cells_never_panic(
        heading in proptest::collection::vec("(Task Name|Duration|Predecessors|Resource Names|Outline Level|Constraint Type|Constraint Date|Deadline|Start|Fixed Cost|Cost|Type|Effort Driven|Priority|Leveling Delay|x|)", 1..9),
        rows in proptest::collection::vec(proptest::collection::vec("[-0-9a-zA-Z ;,\"\\[\\]%.?+\r\n$£'=@]{0,12}", 0..9), 0..12),
        sep in "[,;]",
    ) {
        let mut text = heading.join(&sep);
        for r in rows {
            text.push('\n');
            text.push_str(&r.join(&sep));
        }
        if let Ok(i) = csv(&text) {
            proptest::prop_assert!(i.project.tasks.iter().all(|t| (0..=MAX_MINUTES).contains(&t.duration)));
            proptest::prop_assert!(i.project.tasks.iter().all(|t| (0..=MAX_MINUTES).contains(&t.leveling_delay) && t.priority <= 1000));
            proptest::prop_assert!(i.project.tasks.iter().all(|t| (0..=projectcraft_interchange::model::MAX_MONEY).contains(&t.fixed_cost)));
            proptest::prop_assert!(projectcraft_interchange::model::schedule(&i.project).is_ok() || i.project.tasks.is_empty());
        }
    }

    #[test]
    fn a_damaged_file_never_panics(at in 0usize..KITCHEN.len(), cut in 0usize..64, junk in proptest::collection::vec(proptest::num::u8::ANY, 0..24)) {
        let at = at.min(KITCHEN.len());
        let end = at.saturating_add(cut).min(KITCHEN.len());
        let mut bytes = KITCHEN[..at].to_vec();
        bytes.extend_from_slice(&junk);
        bytes.extend_from_slice(&KITCHEN[end..]);
        let _ = import(Format::Csv, &bytes);
    }
}
