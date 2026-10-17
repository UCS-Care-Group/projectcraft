//! MSPDI links and lags, elapsed spans, and the comparison of the file's dates with ProjectCraft's,
//! from Microsoft's published schema documentation: `PredecessorLink`
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/predecessorlink-element>)
//! with its `PredecessorUID`, `Type` (0 FF, 1 FS, 2 SF, 3 SS), `CrossProject`, `LinkLag` (in
//! tenths of a minute) and `LagFormat` (the duration formats of
//! <https://learn.microsoft.com/office-project/xml-data-interchange/durationformat-element>).
//!
//! ProjectCraft counts durations and lags in working time only. An elapsed span (`2ed`, which
//! counts every hour of the calendar) becomes the same number of working units (`2d`); a percent
//! lag (`50%`, of the predecessor's duration) becomes that share of the predecessor's duration.
//! Each is a report line. A percent lag's `LinkLag` holds tenths of a percent (`500` is 50%).
//!
//! The file's Start and Finish of each task are never used for scheduling: once the project is
//! built it is scheduled, and each task whose dates differ from the file's is a report line.

use std::collections::{HashMap, HashSet};

use crate::links::Refused;
use crate::model::{self, Date, DurationUnit, Link, LinkKind, MAX_MINUTES, Options, Outline, Project};
use crate::mspdi::{line, plural, rows_named};
use crate::{Outcome, ReportLine};

/// Links read from one file; more are only counted.
pub(crate) const MAX_FILE_LINKS: usize = 1_000_000;
/// Report lines naming one converted lag each, and lines naming one task whose dates differ,
/// before the rest share a line.
const MAX_EACH_LINES: usize = 1_000;

/// One `PredecessorLink`, as the file has it.
#[derive(Debug, Default, Clone)]
pub(crate) struct FileLink {
    pred: Option<i64>,
    kind: Option<i64>,
    cross: bool,
    /// `LinkLag`: `None` if absent, `Some(None)` if unreadable.
    lag: Option<Option<i64>>,
    format: Option<i64>,
}

/// What the file says about one task's links and dates, kept beside the task as it is read.
#[derive(Debug, Default)]
pub(crate) struct FileRow {
    /// The task's Unique ID in the file.
    pub(crate) uid: Option<i64>,
    links: Vec<FileLink>,
    /// The link being read.
    link: Option<FileLink>,
    start: Option<Date>,
    finish: Option<Date>,
}

impl FileRow {
    pub(crate) fn open_link(&mut self) {
        self.link = Some(FileLink::default());
    }

    /// Keeps the link just read, while the file's `total` is under [`MAX_FILE_LINKS`]; a link past
    /// it is counted in `overflow`.
    pub(crate) fn close_link(&mut self, total: &mut usize, overflow: &mut usize) {
        let Some(l) = self.link.take() else { return };
        if *total >= MAX_FILE_LINKS {
            *overflow = overflow.saturating_add(1);
        } else {
            *total += 1;
            self.links.push(l);
        }
    }

    /// A field of the link being read.
    pub(crate) fn link_field(&mut self, name: &str, text: &str) {
        let Some(l) = self.link.as_mut() else { return };
        match name {
            "PredecessorUID" => l.pred = int(text),
            "Type" => l.kind = int(text),
            "CrossProject" => l.cross = matches!(text.trim(), "1" | "true"),
            "LinkLag" => l.lag = Some(lag_tenths(text)),
            "LagFormat" => l.format = int(text),
            _ => {}
        }
    }

    /// The task's `Start` or `Finish` in the file.
    pub(crate) fn date_field(&mut self, name: &str, text: &str) {
        let d = text.trim().get(..10).and_then(|d| d.parse().ok());
        match name {
            "Start" => self.start = d,
            "Finish" => self.finish = d,
            _ => {}
        }
    }
}

fn int(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// A `LinkLag`: an integer, or a decimal some programs write, rounded (and saturated).
fn lag_tenths(text: &str) -> Option<i64> {
    let t = text.trim();
    t.parse::<i64>().ok().or_else(|| t.parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f.round() as i64))
}

/// `n` divided by `d` (positive), rounded half away from zero.
fn div_round(n: i128, d: i128) -> i128 {
    let q = (n.abs() + d / 2) / d.max(1);
    if n < 0 { -q } else { q }
}

fn saturate(n: i128) -> i64 {
    i64::try_from(n).unwrap_or(if n < 0 { i64::MIN } else { i64::MAX })
}

/// Minutes of calendar time in one elapsed `unit`. An elapsed month is 30 elapsed days.
fn elapsed_minutes_in(unit: DurationUnit) -> i64 {
    match unit {
        DurationUnit::Minutes => 1,
        DurationUnit::Hours => 60,
        DurationUnit::Days => 1_440,
        DurationUnit::Weeks => 7 * 1_440,
        DurationUnit::Months => 30 * 1_440,
    }
}

/// An elapsed span of `minutes`, shown in `unit`, as the same number of working `unit`s at
/// `options` (`2ed` is `2d`), rounded to the minute. Not clamped.
pub(crate) fn elapsed_to_working(minutes: i64, unit: DurationUnit, options: &Options) -> i64 {
    saturate(div_round(i128::from(minutes) * i128::from(options.minutes_in(unit)), i128::from(elapsed_minutes_in(unit))))
}

/// An elapsed span as Project shows it: `2ed`, `36eh`.
fn elapsed_text(minutes: i64, unit: DurationUnit) -> String {
    format!("{}e{}", model::days_text(minutes, elapsed_minutes_in(unit)), unit.suffix())
}

/// What a `LagFormat` says the lag is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LagForm {
    Working(DurationUnit),
    Elapsed(DurationUnit),
    Percent,
}

/// A `LagFormat` read: estimated formats (35 to 52) as their plain ones; none or unknown as days.
fn lag_form(format: Option<i64>) -> LagForm {
    use DurationUnit::*;
    let f = format.map(|f| if (35..=52).contains(&f) { f - 32 } else { f });
    match f {
        Some(3) => LagForm::Working(Minutes),
        Some(4) => LagForm::Elapsed(Minutes),
        Some(5) => LagForm::Working(Hours),
        Some(6) => LagForm::Elapsed(Hours),
        Some(8) => LagForm::Elapsed(Days),
        Some(9) => LagForm::Working(Weeks),
        Some(10) => LagForm::Elapsed(Weeks),
        Some(11) => LagForm::Working(Months),
        Some(12) => LagForm::Elapsed(Months),
        Some(19 | 20) => LagForm::Percent,
        _ => LagForm::Working(Days),
    }
}

/// Something about a task's links that the report mentions, one line per kind. The order is the
/// report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Note {
    Missing,
    SelfLink,
    Repeated,
    Summary,
    External,
    Loop,
    BadType,
    BadLag,
    LongLag,
}

impl Note {
    const ALL: [Note; 9] =
        [Note::Missing, Note::SelfLink, Note::Repeated, Note::Summary, Note::External, Note::Loop, Note::BadType, Note::BadLag, Note::LongLag];

    fn bit(self) -> u32 {
        1 << (self as u8)
    }

    fn line(self) -> (Outcome, &'static str, &'static str) {
        use Outcome::*;
        const LINKS: &str = "Links (predecessors)";
        match self {
            Note::Missing => (Dropped, LINKS, "a predecessor that isn't a task in the file; that link is dropped"),
            Note::SelfLink => (Dropped, LINKS, "a task can't be its own predecessor; that link is dropped"),
            Note::Repeated => (Dropped, LINKS, "the same predecessor twice; the first link is kept"),
            Note::Summary => (Dropped, LINKS, "a link between a summary task and its own subtask; that link is dropped"),
            Note::External => {
                (Dropped, LINKS, "a predecessor in another project; ProjectCraft links tasks within a project, so that link is dropped")
            }
            Note::Loop => (Dropped, LINKS, "a link that would close a loop of links; that link is dropped"),
            Note::BadType => (Converted, "Link type", "not one of FF, FS, SF and SS; finish-to-start used"),
            Note::BadLag => (Converted, "Lag", "unreadable; no lag used"),
            Note::LongLag => (Converted, "Lag", "longer than ProjectCraft holds; cut to 100,000 days"),
        }
    }
}

/// A converted lag: the link's rows (predecessor, task) and why.
struct Converted {
    elapsed: bool,
    from: usize,
    to: usize,
    why: String,
}

/// Builds each task's links from the file's, by row of `p` (whose outline is settled), with the
/// report lines: one per converted lag, and one per kind of link dropped or changed. `overflow`
/// links were past [`MAX_FILE_LINKS`] and never kept.
pub(crate) fn links(p: &mut Project, rows: &[FileRow], overflow: usize) -> Vec<ReportLine> {
    let options = p.options;
    let outline = Outline::of(p);
    // The first row with each Unique ID in the file.
    let mut row_of: HashMap<i64, usize> = HashMap::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        if let Some(uid) = r.uid {
            row_of.entry(uid).or_insert(i);
        }
    }
    let mut notes = vec![0u32; p.tasks.len()];
    let mut converted: Vec<Converted> = Vec::new();
    let mut all: Vec<Vec<Link>> = Vec::with_capacity(p.tasks.len());
    for (i, r) in rows.iter().enumerate().take(p.tasks.len()) {
        let mut n = 0u32;
        let mut preds = Vec::new();
        let mut kept = HashSet::new();
        for l in &r.links {
            if l.cross {
                n |= Note::External.bit();
                continue;
            }
            let Some((j, pred)) = l.pred.and_then(|u| row_of.get(&u)).and_then(|&j| p.tasks.get(j).map(|t| (j, t.id))) else {
                n |= Note::Missing.bit();
                continue;
            };
            match crate::links::refused(&outline, &kept, j, i, pred) {
                Some(Refused::SelfLink) => n |= Note::SelfLink.bit(),
                Some(Refused::Repeated) => n |= Note::Repeated.bit(),
                Some(Refused::Summary) => n |= Note::Summary.bit(),
                None => {
                    let kind = match l.kind {
                        None | Some(1) => LinkKind::FS,
                        Some(0) => LinkKind::FF,
                        Some(2) => LinkKind::SF,
                        Some(3) => LinkKind::SS,
                        Some(_) => {
                            n |= Note::BadType.bit();
                            LinkKind::FS
                        }
                    };
                    let (lag, lag_unit) = lag(p, l, j, i, &options, &mut n, &mut converted);
                    kept.insert(pred);
                    preds.push(Link { pred, kind, lag, lag_unit });
                }
            }
        }
        all.push(preds);
        if let Some(x) = notes.get_mut(i) {
            *x = n;
        }
    }
    for (t, preds) in p.tasks.iter_mut().zip(all) {
        t.preds = preds;
    }
    for i in crate::links::drop_loops(p) {
        if let Some(x) = notes.get_mut(i) {
            *x |= Note::Loop.bit();
        }
    }
    let mut out = Vec::new();
    if overflow > 0 {
        out.push(line(
            Outcome::Dropped,
            "Links (predecessors)",
            &plural(overflow, "link"),
            "the file holds more than 1,000,000 links; the rest are dropped",
        ));
    }
    for note in Note::ALL {
        let named: Vec<usize> = notes.iter().enumerate().filter(|(_, b)| *b & note.bit() != 0).map(|(i, _)| i).collect();
        if !named.is_empty() {
            let (outcome, feature, why) = note.line();
            out.push(line(outcome, feature, &rows_named(&p.tasks, &named), why));
        }
    }
    for elapsed in [true, false] {
        let (feature, why) = if elapsed {
            ("Elapsed lag", "converted to working time, as the lags before them")
        } else {
            ("Percent lag", "converted to working time from their predecessors' durations, as the lags before them")
        };
        let each: Vec<&Converted> = converted.iter().filter(|c| c.elapsed == elapsed).collect();
        for c in each.iter().take(MAX_EACH_LINES) {
            let place = format!("{} to {}", rows_named(&p.tasks, &[c.from]), rows_named(&p.tasks, &[c.to]));
            out.push(line(Outcome::Converted, feature, &place, &c.why));
        }
        if each.len() > MAX_EACH_LINES {
            out.push(line(Outcome::Converted, feature, &plural(each.len() - MAX_EACH_LINES, "more link"), why));
        }
    }
    out
}

/// The lag of link `l` from row `from` to row `to`, in working minutes (clamped) and its unit;
/// an elapsed or percent lag is converted and noted in `converted`.
fn lag(p: &Project, l: &FileLink, from: usize, to: usize, options: &Options, n: &mut u32, converted: &mut Vec<Converted>) -> (i64, DurationUnit) {
    let tenths = match l.lag {
        None => 0,
        Some(Some(t)) => t,
        Some(None) => {
            *n |= Note::BadLag.bit();
            0
        }
    };
    let minutes = saturate(div_round(i128::from(tenths), 10));
    let (lag, unit) = match lag_form(l.format) {
        LagForm::Working(unit) => (minutes, unit),
        LagForm::Elapsed(unit) => {
            let lag = elapsed_to_working(minutes, unit, options);
            if lag != 0 {
                let shown = model::duration_text(lag.clamp(-MAX_MINUTES, MAX_MINUTES), unit, options);
                converted.push(Converted {
                    elapsed: true,
                    from,
                    to,
                    why: format!(
                        "{} read as {shown}: ProjectCraft counts lags in working time, so the elapsed span is the same number of working units",
                        elapsed_text(minutes, unit)
                    ),
                });
            }
            (lag, unit)
        }
        LagForm::Percent => {
            let (duration, unit) = p.tasks.get(from).map_or((0, DurationUnit::Days), |t| (t.duration, t.duration_unit));
            let lag = saturate(div_round(i128::from(duration) * i128::from(tenths), 1_000));
            if tenths != 0 {
                let percent = model::days_text(tenths, 10);
                converted.push(Converted {
                    elapsed: false,
                    from,
                    to,
                    why: format!(
                        "{percent}% of the predecessor's {} read as {}: ProjectCraft counts lags in working time",
                        model::duration_text(duration, unit, options),
                        model::duration_text(lag.clamp(-MAX_MINUTES, MAX_MINUTES), unit, options)
                    ),
                });
            }
            (lag, unit)
        }
    };
    if !(-MAX_MINUTES..=MAX_MINUTES).contains(&lag) {
        *n |= Note::LongLag.bit();
    }
    (lag.clamp(-MAX_MINUTES, MAX_MINUTES), unit)
}

/// Schedules `p` and compares each task's Start and Finish with the file's (where the file gives
/// them), a line for each task whose dates differ, up to [`MAX_EACH_LINES`] and a line for the rest.
pub(crate) fn date_lines(p: &Project, rows: &[FileRow]) -> Vec<ReportLine> {
    const FEATURE: &str = "Start and finish";
    if rows.iter().all(|r| r.start.is_none() && r.finish.is_none()) {
        return Vec::new();
    }
    let s = match model::schedule(p) {
        Ok(s) => s,
        Err(e) => {
            return vec![line(
                Outcome::DatesDiffer,
                FEATURE,
                "Project",
                &format!("ProjectCraft couldn't schedule the project to compare its dates: {e}"),
            )];
        }
    };
    let mut out = Vec::new();
    let mut more = 0usize;
    for (row, (t, r)) in p.tasks.iter().zip(rows).enumerate() {
        let Some(ours) = s.tasks.get(&t.id) else { continue };
        let differs = r.start.is_some_and(|d| d != ours.start) || r.finish.is_some_and(|d| d != ours.finish);
        if !differs {
            continue;
        }
        if out.len() >= MAX_EACH_LINES {
            more += 1;
            continue;
        }
        let shown = |d: Option<Date>| d.map_or_else(|| "no date".to_owned(), |d| d.to_string());
        out.push(line(
            Outcome::DatesDiffer,
            FEATURE,
            &rows_named(&p.tasks, &[row]),
            &format!("the file has {} to {}; ProjectCraft works out {} to {}", shown(r.start), shown(r.finish), ours.start, ours.finish),
        ));
    }
    if more > 0 {
        out.push(line(Outcome::DatesDiffer, FEATURE, &plural(more, "more task"), "their dates differ from the file's too"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_spans_become_the_same_number_of_working_units() {
        let o = Options::default();
        assert_eq!(elapsed_to_working(2 * 1_440, DurationUnit::Days, &o), 2 * 480);
        assert_eq!(elapsed_to_working(-1_440, DurationUnit::Days, &o), -480);
        assert_eq!(elapsed_to_working(90, DurationUnit::Hours, &o), 90);
        assert_eq!(elapsed_to_working(10_080, DurationUnit::Weeks, &o), 2_400);
        assert_eq!(elapsed_to_working(30 * 1_440, DurationUnit::Months, &o), 20 * 480);
        let huge = elapsed_to_working(i64::MAX, DurationUnit::Months, &o);
        assert!(huge > 0 && huge < i64::MAX, "no overflow: {huge}");
        assert_eq!(elapsed_to_working(i64::MIN, DurationUnit::Days, &o), i64::MIN / 3 - 1, "a third (480 of 1,440), rounded away from zero");
        assert_eq!(elapsed_text(2_880, DurationUnit::Days), "2ed");
        assert_eq!(elapsed_text(90, DurationUnit::Hours), "1.5eh");
    }

    #[test]
    fn lag_formats_read_as_their_form() {
        assert_eq!(lag_form(Some(7)), LagForm::Working(DurationUnit::Days));
        assert_eq!(lag_form(Some(8)), LagForm::Elapsed(DurationUnit::Days));
        assert_eq!(lag_form(Some(40)), LagForm::Elapsed(DurationUnit::Days), "estimated elapsed days");
        assert_eq!(lag_form(Some(51)), LagForm::Percent);
        assert_eq!(lag_form(Some(20)), LagForm::Percent);
        assert_eq!(lag_form(None), LagForm::Working(DurationUnit::Days));
        assert_eq!(lag_form(Some(-3)), LagForm::Working(DurationUnit::Days));
    }

    #[test]
    fn lags_read_as_integers_or_decimals() {
        assert_eq!(lag_tenths(" 4800 "), Some(4800));
        assert_eq!(lag_tenths("4800.4"), Some(4800));
        assert_eq!(lag_tenths("1e400"), None);
        assert_eq!(lag_tenths("NaN"), None);
        assert_eq!(lag_tenths("soon"), None);
        assert_eq!(lag_tenths("-1e30"), Some(i64::MIN));
        assert_eq!(div_round(15, 10), 2);
        assert_eq!(div_round(-15, 10), -2);
        assert_eq!(div_round(14, 10), 1);
    }

    #[test]
    fn links_past_the_cap_are_only_counted() {
        let mut r = FileRow::default();
        let (mut total, mut overflow) = (MAX_FILE_LINKS - 1, 0);
        for _ in 0..3 {
            r.open_link();
            r.close_link(&mut total, &mut overflow);
        }
        assert_eq!((r.links.len(), total, overflow), (1, MAX_FILE_LINKS, 2));
        // A close without an open does nothing.
        r.close_link(&mut total, &mut overflow);
        assert_eq!(overflow, 2);
    }
}
