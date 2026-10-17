//! MSPDI links, lags and the comparison of the file's dates with ProjectCraft's, against
//! hand-written files built from Microsoft's published schema docs: `PredecessorLink`
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/predecessorlink-element>),
//! its `Type` (0 FF, 1 FS, 2 SF, 3 SS), `LinkLag` (tenths of a minute) and `LagFormat` (the
//! duration formats: 7 days, 8 elapsed days, 19 percent …).
//!
//! The worked examples run on the Standard week (Monday to Friday, 8:00–12:00 and 13:00–17:00)
//! from Monday 5 January 2026; task A takes 2 days (Monday 5 and Tuesday 6), task B 1 day.

use std::time::{Duration, Instant};

use projectcraft_interchange::model::{self, Date, DurationUnit, LinkKind, MAX_MINUTES, TaskId};
use projectcraft_interchange::{Format, Import, ImportError, Outcome, ReportLine, import};

fn mspdi(text: &str) -> Result<Import, ImportError> {
    import(Format::Mspdi, text.as_bytes())
}

fn day(d: u32) -> Date {
    Date::from_ymd(2026, 1, d).unwrap()
}

/// A task in a test file.
#[derive(Default, Clone)]
struct T {
    uid: i64,
    name: &'static str,
    duration: &'static str,
    format: Option<i64>,
    /// Each link's inner XML.
    links: Vec<String>,
    start: Option<&'static str>,
    finish: Option<&'static str>,
    level: i64,
}

fn t(uid: i64, name: &'static str, duration: &'static str) -> T {
    T { uid, name, duration, level: 1, ..T::default() }
}

impl T {
    /// A link from `pred` of `kind` with `LinkLag` `lag` in `LagFormat` `format`.
    fn link(mut self, pred: i64, kind: i64, lag: i64, format: i64) -> T {
        self.links.push(format!("<PredecessorUID>{pred}</PredecessorUID><Type>{kind}</Type><LinkLag>{lag}</LinkLag><LagFormat>{format}</LagFormat>"));
        self
    }

    fn raw_link(mut self, inner: &str) -> T {
        self.links.push(inner.to_owned());
        self
    }

    fn dates(mut self, start: &'static str, finish: &'static str) -> T {
        self.start = Some(start);
        self.finish = Some(finish);
        self
    }
}

fn file(tasks: &[T]) -> String {
    let mut s = String::from("<Project xmlns=\"http://schemas.microsoft.com/project\"><StartDate>2026-01-05T08:00:00</StartDate><Tasks>");
    for (n, t) in tasks.iter().enumerate() {
        s.push_str(&format!(
            "<Task><UID>{}</UID><ID>{}</ID><Name>{}</Name><OutlineLevel>{}</OutlineLevel><Duration>{}</Duration>",
            t.uid,
            n + 1,
            t.name,
            t.level,
            t.duration
        ));
        if let Some(f) = t.format {
            s.push_str(&format!("<DurationFormat>{f}</DurationFormat>"));
        }
        if let Some(d) = t.start {
            s.push_str(&format!("<Start>{d}</Start>"));
        }
        if let Some(d) = t.finish {
            s.push_str(&format!("<Finish>{d}</Finish>"));
        }
        for l in &t.links {
            s.push_str(&format!("<PredecessorLink>{l}</PredecessorLink>"));
        }
        s.push_str("</Task>");
    }
    s.push_str("</Tasks></Project>");
    s
}

fn lines<'a>(i: &'a Import, feature: &str) -> Vec<&'a ReportLine> {
    i.report.lines.iter().filter(|l| l.feature == feature).collect()
}

/// A (2 days) and B (1 day) linked by `kind` with `lag` in `format`: B's start and finish.
fn b_dates(kind: i64, lag: i64, format: i64) -> (Date, Date, Import) {
    let i = mspdi(&file(&[t(1, "A", "PT16H0M0S"), t(2, "B", "PT8H0M0S").link(1, kind, lag, format)])).unwrap();
    let s = model::schedule(&i.project).unwrap();
    let b = s.tasks[&TaskId(2)];
    (b.start, b.finish, i)
}

#[test]
fn finish_to_start_without_lag() {
    let (start, finish, i) = b_dates(1, 0, 7);
    assert_eq!((start, finish), (day(7), day(7)), "B starts the working day after A finishes");
    let l = i.project.tasks[1].preds[0];
    assert_eq!((l.pred, l.kind, l.lag, l.lag_unit), (TaskId(1), LinkKind::FS, 0, DurationUnit::Days));
    assert!(lines(&i, "Elapsed lag").is_empty() && lines(&i, "Percent lag").is_empty());
}

#[test]
fn finish_to_start_with_a_lag_of_one_day() {
    // 1 day of 8 hours is 480 minutes, 4800 tenths of a minute.
    let (start, finish, i) = b_dates(1, 4800, 7);
    assert_eq!((start, finish), (day(8), day(8)));
    assert_eq!((i.project.tasks[1].preds[0].lag, i.project.tasks[1].preds[0].lag_unit), (480, DurationUnit::Days));
}

#[test]
fn finish_to_start_with_a_lead_of_one_day() {
    let (start, finish, _) = b_dates(1, -4800, 7);
    assert_eq!((start, finish), (day(6), day(6)), "a lead overlaps B with A's last day");
}

#[test]
fn start_to_start_with_a_lag_of_four_hours() {
    // SS+4h: B starts Monday at 13:00 and works 8 hours, to Tuesday noon.
    let (start, finish, i) = b_dates(3, 2400, 5);
    assert_eq!((start, finish), (day(5), day(6)));
    let l = i.project.tasks[1].preds[0];
    assert_eq!((l.kind, l.lag, l.lag_unit), (LinkKind::SS, 240, DurationUnit::Hours));
}

#[test]
fn finish_to_finish_with_a_lag_of_one_day() {
    // A finishes Tuesday; B finishes a day later, on Wednesday, so starts on Wednesday.
    let (start, finish, i) = b_dates(0, 4800, 7);
    assert_eq!((start, finish), (day(7), day(7)));
    assert_eq!(i.project.tasks[1].preds[0].kind, LinkKind::FF);
}

#[test]
fn start_to_finish_with_a_lag_of_three_days() {
    // A starts Monday 8:00; three days on is Thursday 8:00, when B must finish: B works Wednesday.
    let (start, finish, i) = b_dates(2, 14_400, 7);
    assert_eq!((start, finish), (day(7), day(7)));
    assert_eq!(i.project.tasks[1].preds[0].kind, LinkKind::SF);
}

#[test]
fn an_elapsed_lag_becomes_the_same_span_of_working_time_and_is_reported() {
    // FS+2ed: 2 elapsed days are 2,880 minutes, 28,800 tenths; read as 2 working days.
    let (start, finish, i) = b_dates(1, 28_800, 8);
    let l = i.project.tasks[1].preds[0];
    assert_eq!((l.lag, l.lag_unit), (960, DurationUnit::Days));
    // A finishes Tuesday; two working days on is Thursday evening; B works Friday.
    assert_eq!((start, finish), (day(9), day(9)));
    let r = lines(&i, "Elapsed lag");
    assert_eq!(r.len(), 1, "{:#?}", i.report.lines);
    assert_eq!(r[0].outcome, Outcome::Converted);
    assert_eq!(r[0].place, "ID 1 A to ID 2 B");
    assert!(r[0].why.contains("2ed") && r[0].why.contains("2d"), "{}", r[0].why);
    // Elapsed hours and weeks keep their unit too.
    let (_, _, h) = b_dates(1, 36_000, 6);
    assert_eq!((h.project.tasks[1].preds[0].lag, h.project.tasks[1].preds[0].lag_unit), (3600, DurationUnit::Hours), "60eh is 60h");
    let (_, _, w) = b_dates(1, 100_800, 10);
    assert_eq!((w.project.tasks[1].preds[0].lag, w.project.tasks[1].preds[0].lag_unit), (2400, DurationUnit::Weeks), "1ew is 1w");
}

#[test]
fn a_percent_lag_becomes_working_time_from_the_predecessor_s_duration_and_is_reported() {
    // FS+50%: half of A's 2 days is 1 day. LinkLag holds tenths of a percent.
    let (start, finish, i) = b_dates(1, 500, 19);
    let l = i.project.tasks[1].preds[0];
    assert_eq!((l.lag, l.lag_unit), (480, DurationUnit::Days));
    assert_eq!((start, finish), (day(8), day(8)));
    let r = lines(&i, "Percent lag");
    assert_eq!(r.len(), 1, "{:#?}", i.report.lines);
    assert_eq!((r[0].outcome, r[0].place.as_str()), (Outcome::Converted, "ID 1 A to ID 2 B"));
    assert!(r[0].why.contains("50%") && r[0].why.contains("1d"), "{}", r[0].why);
    // An elapsed percent is a percent of the duration too.
    let (_, _, e) = b_dates(3, 250, 20);
    assert_eq!(e.project.tasks[1].preds[0].lag, 240);
    assert_eq!(lines(&e, "Percent lag").len(), 1);
}

#[test]
fn one_report_line_per_converted_lag() {
    let i = mspdi(&file(&[
        t(1, "A", "PT16H0M0S"),
        t(2, "B", "PT8H0M0S").link(1, 1, 28_800, 8),
        t(3, "C", "PT8H0M0S").link(1, 3, 500, 19).link(2, 1, 600, 4),
        t(4, "D", "PT8H0M0S").link(3, 1, 4800, 7),
    ]))
    .unwrap();
    let places = |f| lines(&i, f).iter().map(|l| l.place.clone()).collect::<Vec<_>>();
    assert_eq!(places("Elapsed lag"), ["ID 1 A to ID 2 B", "ID 2 B to ID 3 C"]);
    assert_eq!(places("Percent lag"), ["ID 1 A to ID 3 C"]);
}

#[test]
fn links_follow_the_file_s_unique_ids_even_when_renumbered() {
    // Repeated Unique IDs are renumbered; links still find the first task with the UID.
    let i = mspdi(&file(&[t(5, "A", "PT8H"), t(9, "B", "PT8H").link(5, 1, 0, 7), t(9, "C", "PT8H").link(5, 3, 0, 7)])).unwrap();
    let preds: Vec<Vec<u32>> = i.project.tasks.iter().map(|t| t.preds.iter().map(|l| l.pred.0).collect()).collect();
    assert_eq!(preds, [vec![], vec![1], vec![1]]);
}

#[test]
fn links_that_can_t_be_kept_are_dropped_and_reported() {
    let summary = T { level: 1, ..t(1, "Phase", "PT8H") };
    let sub = T { level: 2, ..t(2, "Inside", "PT8H") }.link(1, 1, 0, 7);
    let i = mspdi(&file(&[
        summary,
        sub,
        t(3, "Self", "PT8H").link(3, 1, 0, 7),
        t(4, "Twice", "PT8H").link(3, 1, 0, 7).link(3, 3, 0, 7),
        t(5, "Missing", "PT8H").link(77, 1, 0, 7).raw_link("<Type>1</Type>"),
        t(6, "Elsewhere", "PT8H")
            .raw_link("<PredecessorUID>3</PredecessorUID><CrossProject>1</CrossProject><CrossProjectName>Other.mpp\\5</CrossProjectName>"),
        t(7, "Loop A", "PT8H").link(8, 1, 0, 7),
        t(8, "Loop B", "PT8H").link(7, 1, 0, 7),
    ]))
    .unwrap();
    let got: Vec<(Outcome, &str, &str)> = lines(&i, "Links (predecessors)").iter().map(|l| (l.outcome, l.place.as_str(), l.why.as_str())).collect();
    let places: Vec<&str> = got.iter().map(|g| g.1).collect();
    assert_eq!(places, ["ID 5 Missing", "ID 3 Self", "ID 4 Twice", "ID 2 Inside", "ID 6 Elsewhere", "ID 8 Loop B"], "{got:#?}");
    assert!(got.iter().all(|g| g.0 == Outcome::Dropped && !g.2.is_empty()));
    assert_eq!(i.project.tasks[3].preds.len(), 1, "the first of the repeated links is kept");
    assert_eq!(i.project.tasks[6].preds.len(), 1);
    assert!(i.project.tasks[7].preds.is_empty());
    assert!(model::schedule(&i.project).is_ok());
}

#[test]
fn odd_link_fields_are_read_as_far_as_they_go_and_reported() {
    let i = mspdi(&file(&[
        t(1, "A", "PT8H"),
        t(2, "No type", "PT8H").raw_link("<PredecessorUID>1</PredecessorUID>"),
        t(3, "Odd type", "PT8H").link(1, 9, 0, 7),
        t(4, "Odd lag", "PT8H").raw_link("<PredecessorUID>1</PredecessorUID><Type>1</Type><LinkLag>soon</LinkLag><LagFormat>7</LagFormat>"),
        t(5, "Huge lag", "PT8H").link(1, 1, i64::MAX, 7),
        t(6, "Huge lead", "PT8H").link(1, 1, -999_999_999_999_999, 8),
    ]))
    .unwrap();
    let preds: Vec<(LinkKind, i64)> = i.project.tasks.iter().skip(1).map(|t| (t.preds[0].kind, t.preds[0].lag)).collect();
    assert_eq!(preds, [(LinkKind::FS, 0), (LinkKind::FS, 0), (LinkKind::FS, 0), (LinkKind::FS, MAX_MINUTES), (LinkKind::FS, -MAX_MINUTES)]);
    let got = |f| lines(&i, f).iter().map(|l| (l.outcome, l.place.clone())).collect::<Vec<_>>();
    assert_eq!(got("Link type"), [(Outcome::Converted, "ID 3 Odd type".to_string())]);
    assert_eq!(got("Lag"), [(Outcome::Converted, "ID 4 Odd lag".to_string()), (Outcome::Converted, "ID 5 Huge lag, ID 6 Huge lead".to_string())]);
    assert!(model::schedule(&i.project).is_ok());
}

#[test]
fn an_elapsed_duration_becomes_the_same_span_of_working_time() {
    // 3ed: Project writes the elapsed span, 72 hours; read as 3 working days.
    let task = T { format: Some(8), ..t(1, "Cure", "PT72H0M0S") };
    let weeks = T { format: Some(10), ..t(2, "Settle", "PT168H0M0S") };
    let hours = T { format: Some(38), ..t(3, "Dry", "PT30H0M0S") };
    let i = mspdi(&file(&[task, weeks, hours])).unwrap();
    let d: Vec<(i64, DurationUnit)> = i.project.tasks.iter().map(|t| (t.duration, t.duration_unit)).collect();
    assert_eq!(d, [(3 * 480, DurationUnit::Days), (2400, DurationUnit::Weeks), (30 * 60, DurationUnit::Hours)]);
    let l = lines(&i, "Elapsed duration");
    assert_eq!(l.len(), 1);
    assert_eq!((l[0].outcome, l[0].place.as_str()), (Outcome::Converted, "ID 1 Cure, ID 2 Settle, ID 3 Dry"));
}

#[test]
fn tasks_whose_dates_differ_from_the_file_s_are_reported_one_line_each() {
    let i = mspdi(&file(&[
        t(1, "A", "PT16H0M0S").dates("2026-01-05T08:00:00", "2026-01-06T17:00:00"),
        t(2, "B", "PT8H0M0S").link(1, 1, 0, 7).dates("2026-01-08T08:00:00", "2026-01-08T17:00:00"),
        t(3, "C", "PT8H0M0S").link(1, 1, 0, 7).dates("2026-01-07T08:00:00", "2026-01-07T17:00:00"),
        t(4, "No dates", "PT8H0M0S"),
    ]))
    .unwrap();
    let d = lines(&i, "Start and finish");
    assert_eq!(d.len(), 1, "{:#?}", i.report.lines);
    assert_eq!((d[0].outcome, d[0].place.as_str()), (Outcome::DatesDiffer, "ID 2 B"));
    assert_eq!(d[0].why, "the file has 2026-01-08 to 2026-01-08; ProjectCraft works out 2026-01-07 to 2026-01-07");
    assert_eq!(i.report.lines.last().map(|l| l.outcome), Some(Outcome::DatesDiffer), "date lines come last");
    assert!(i.report.text().contains("Dates differ: Start and finish (ID 2 B): the file has"), "{}", i.report.text());
}

#[test]
fn no_date_lines_when_the_dates_agree() {
    let i = mspdi(&file(&[
        t(1, "A", "PT16H0M0S").dates("2026-01-05T08:00:00", "2026-01-06T17:00:00"),
        t(2, "B", "PT8H0M0S").link(1, 1, 0, 7).dates("2026-01-07T08:00:00", "2026-01-07T17:00:00"),
        t(3, "Only a start", "PT8H0M0S").link(2, 1, 0, 7).raw_link("<PredecessorUID>1</PredecessorUID>"),
    ]))
    .unwrap();
    assert!(lines(&i, "Start and finish").is_empty(), "{:#?}", i.report.lines);
    let only_start = mspdi(&file(&[T { start: Some("2026-01-05T08:00:00"), ..t(1, "A", "PT8H") }])).unwrap();
    assert!(lines(&only_start, "Start and finish").is_empty());
}

#[test]
fn date_lines_are_bounded() {
    // Every task's file dates are a year out: one line each up to the cap, then one more line.
    let tasks: Vec<T> = (1..=1_500).map(|n| t(n, "T", "PT8H").dates("2027-01-05T08:00:00", "2027-01-05T17:00:00")).collect();
    let i = mspdi(&file(&tasks)).unwrap();
    let d = lines(&i, "Start and finish");
    assert_eq!(d.len(), 1_001);
    assert_eq!(d[1_000].place, "500 more tasks");
}

#[test]
fn many_links_and_long_chains_import_fast() {
    // A chain of 3,000 tasks, each also linked back to every one of the 20 before it, and a loop.
    let mut tasks: Vec<T> = Vec::new();
    for n in 1..=3_000i64 {
        let mut task = t(n, "T", "PT8H");
        for back in 1..=20 {
            if n > back {
                task = task.link(n - back, 1, 0, 7);
            }
        }
        if n == 1 {
            task = task.link(3_000, 1, 0, 7);
        }
        tasks.push(task);
    }
    let text = file(&tasks);
    let time = Instant::now();
    let i = mspdi(&text).unwrap();
    assert!(time.elapsed() < Duration::from_secs(10), "{:?}", time.elapsed());
    // Kept in row order: the links into the last task are the ones that close the loop.
    assert_eq!(i.project.tasks[0].preds.len(), 1);
    assert!(i.project.tasks[2_999].preds.is_empty());
    assert!(lines(&i, "Links (predecessors)").iter().any(|l| l.place == "ID 3000 T"), "{:#?}", i.report.lines);
    assert!(model::schedule(&i.project).is_ok());
}
