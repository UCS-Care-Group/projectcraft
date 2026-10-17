//! CSV import: comma- or semicolon-separated values, UTF-8 with or without a byte order mark,
//! read as RFC 4180 has it (<https://www.rfc-editor.org/rfc/rfc4180>): a quoted cell may hold the
//! separator, line breaks and doubled quotes; a line ends with CRLF, LF or CR.
//!
//! The first row names the columns, matched to the task sheet's (any case; `_` reads as a space):
//! `Task Name` (or `Name`) is required; `Duration`, `Predecessors` (by row), `Resource Names`,
//! `Outline Level`, `Constraint Type` with `Constraint Date`, `Deadline`, `Fixed Cost`, `Type`,
//! `Effort Driven`, `Priority` and `Leveling Delay` are read when there. `ID` is the row number
//! and is passed over; the earliest `Start` is the project's start, and the symbol of the first
//! `Fixed Cost` or `Cost` amount picks the currency. Every other column is a report line, and so
//! is every cell that can't be read: the task keeps a default (1 day, level 1, As Soon As
//! Possible, no deadline, no link, no resource, no fixed cost, the project's task type, priority
//! 500, no delay) instead.
//!
//! A quoted Task Name keeps the spaces at its ends; every other cell is trimmed. A Task Name or
//! Resource Names cell that ProjectCraft's CSV export guarded against formula injection with an
//! apostrophe in front ([`crate::csv_export`]) reads without it.
//!
//! Rows are counted from the first under the headings, blank rows included, so `Predecessors`
//! reads `3` as the third row; blank rows themselves are not imported.

use std::collections::{HashMap, HashSet};

use crate::links::Refused;
use crate::model::{
    self, Constraint, Date, DurationUnit, Link, MAX_ASSIGNMENTS, MAX_MINUTES, MAX_OUTLINE_LEVEL, MAX_RESOURCES, MAX_TASKS, Outline, Project, Task,
    TaskId,
};
use crate::{Format, Import, ImportError, ImportReport, MAX_IMPORT_BYTES, Outcome, ReportLine};

/// Columns read from the heading row; any past it are one report line.
pub(crate) const MAX_COLUMNS: usize = 1_000;
/// Tasks or columns named in one report line before "and n more".
const NAMED_IN_LINE: usize = 3;
/// The byte order mark a spreadsheet's "CSV UTF-8" starts with.
const BOM: &[u8] = b"\xEF\xBB\xBF";

pub(crate) fn import(bytes: &[u8]) -> Result<Import, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| ImportError::CsvNotUtf8)?;
    let mut reader = Reader::new(text, separator(text));
    let (headings, past) = reader.record(MAX_COLUMNS).ok_or(ImportError::NoTaskName)?;
    let mut sheet = Sheet::of(&headings, past);
    if sheet.at(Field::Name).is_none() {
        return Err(ImportError::NoTaskName);
    }
    let mut rows = Vec::new();
    let mut row = 0usize;
    let (mut blank, mut blank_run) = (0usize, 0usize);
    let mut unclosed = None;
    while let Some((cells, past)) = reader.record(MAX_COLUMNS) {
        row = row.saturating_add(1);
        if reader.unclosed && unclosed.is_none() {
            unclosed = Some(row);
        }
        if cells.iter().all(|c| c.trim().is_empty()) && past == 0 {
            blank_run += 1;
            continue;
        }
        // Blank rows count only between tasks: a spreadsheet's trailing ones are no news.
        blank += std::mem::take(&mut blank_run);
        if rows.len() >= MAX_TASKS {
            return Err(ImportError::TooManyTasks);
        }
        rows.push(sheet.row(row, cells, &reader.quoted, past));
    }
    let mut lines = sheet.lines();
    if let Some(row) = unclosed {
        lines.push(line(
            Outcome::Converted,
            "Quotes",
            &format!("Row {row}"),
            "a quoted cell runs to the end of the file; it is read as ending there",
        ));
    }
    if blank > 0 {
        lines.push(line(Outcome::Dropped, "Blank rows", &plural(blank, "row"), "ProjectCraft has no blank rows; they still count for Predecessors"));
    }
    let mut p = Project::new("Imported project", Date::EPOCH);
    if let Some(c) = currency(&rows, &mut lines) {
        p.currency = c;
    }
    let (tasks, mut notes) = tasks(&rows, &p);
    p.tasks = tasks;
    p.next_id = u32::try_from(p.tasks.len()).unwrap_or(u32::MAX - 1).saturating_add(1);
    p.start = start(&rows, &p, &mut lines);
    fit_outline(&mut p, &mut notes, sheet.at(Field::Duration).is_some(), &rows);
    summaries(&mut p, &rows, &mut notes);
    links(&mut p, &rows, &mut notes);
    let resource_errors = resources(&mut p, &rows, &mut notes);
    lines.extend(task_lines(&p.tasks, &rows, &notes, &resource_errors));
    lines.sort_by_key(|l| l.outcome);
    let count = p.tasks.len();
    Ok(Import { project: p, report: ImportReport { format: Format::Csv, tasks: count, lines } })
}

/// The separator: a semicolon when the heading row has more of them than commas.
fn separator(text: &str) -> u8 {
    let (mut commas, mut semicolons, mut quoted) = (0usize, 0usize, false);
    for b in text.bytes() {
        match b {
            b'"' => quoted = !quoted,
            b',' if !quoted => commas += 1,
            b';' if !quoted => semicolons += 1,
            b'\n' | b'\r' if !quoted => break,
            _ => {}
        }
    }
    if semicolons > commas { b';' } else { b',' }
}

/// Reads records one at a time, so a file of a million blank lines holds no more than one.
struct Reader<'a> {
    text: &'a str,
    pos: usize,
    sep: u8,
    /// The last cell read was quoted and ran to the end of the file.
    unclosed: bool,
    /// Which of the last record's cells (up to the `max` kept) were quoted: their spaces are
    /// the owner's, so a quoted Task Name keeps them.
    quoted: Vec<bool>,
}

/// How a cell ended.
#[derive(PartialEq, Eq)]
enum End {
    Separator,
    Line,
    File,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str, sep: u8) -> Self {
        Reader { text, pos: 0, sep, unclosed: false, quoted: Vec::new() }
    }

    /// The next record's first `max` cells and how many more it had; `None` at the end.
    fn record(&mut self, max: usize) -> Option<(Vec<String>, usize)> {
        if self.pos >= self.text.len() {
            return None;
        }
        let mut cells = Vec::new();
        let mut past = 0usize;
        self.quoted.clear();
        loop {
            let quoted = self.text.as_bytes().get(self.pos) == Some(&b'"');
            let (cell, end) = self.cell();
            if cells.len() < max {
                cells.push(cell);
                self.quoted.push(quoted);
            } else {
                past = past.saturating_add(1);
            }
            if end != End::Separator {
                return Some((cells, past));
            }
        }
    }

    /// One cell. Every byte it stops at is ASCII, so each slice falls on a character boundary.
    fn cell(&mut self) -> (String, End) {
        let (text, bytes) = (self.text, self.text.as_bytes());
        let mut i = self.pos;
        let began = i;
        let mut out = String::new();
        if bytes.get(i) == Some(&b'"') {
            i += 1;
            let mut from = i;
            loop {
                match bytes.get(i) {
                    None => {
                        out.push_str(text.get(from..i).unwrap_or(""));
                        self.unclosed = true;
                        self.pos = i;
                        return (out, End::File);
                    }
                    Some(b'"') => {
                        out.push_str(text.get(from..i).unwrap_or(""));
                        if bytes.get(i + 1) == Some(&b'"') {
                            out.push('"');
                            i += 2;
                            from = i;
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    Some(_) => i += 1,
                }
            }
        }
        // Unquoted, or what follows a closing quote (kept as written, as spreadsheets do).
        let from = i;
        let end = loop {
            match bytes.get(i) {
                None => {
                    self.pos = i;
                    break End::File;
                }
                Some(&b) if b == self.sep => {
                    self.pos = i + 1;
                    break End::Separator;
                }
                Some(b'\n') => {
                    self.pos = i + 1;
                    break End::Line;
                }
                Some(b'\r') => {
                    self.pos = if bytes.get(i + 1) == Some(&b'\n') { i + 2 } else { i + 1 };
                    break End::Line;
                }
                Some(_) => i += 1,
            }
        };
        let rest = text.get(from..i).unwrap_or("");
        // Spaces after a closing quote are padding; inside the quotes they are the cell's own.
        out.push_str(if from > began { rest.trim_end() } else { rest });
        (out, end)
    }
}

/// A column ProjectCraft reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Field {
    Name,
    Duration,
    Predecessors,
    ResourceNames,
    OutlineLevel,
    ConstraintType,
    ConstraintDate,
    Deadline,
    FixedCost,
    Type,
    EffortDriven,
    Priority,
    LevelingDelay,
    /// The row number: passed over.
    Id,
    /// Dates come from the scheduler; the earliest is the project's start.
    Start,
    /// Worked out from the resources and fixed cost; its symbol can pick the currency.
    Cost,
}

impl Field {
    const READ: [Field; 15] = [
        Field::Name,
        Field::Duration,
        Field::Predecessors,
        Field::ResourceNames,
        Field::OutlineLevel,
        Field::ConstraintType,
        Field::ConstraintDate,
        Field::Deadline,
        Field::FixedCost,
        Field::Type,
        Field::EffortDriven,
        Field::Priority,
        Field::LevelingDelay,
        Field::Start,
        Field::Cost,
    ];

    fn of(heading: &str) -> Option<Field> {
        let h: String = heading.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        Some(match h.as_str() {
            "task name" | "name" => Field::Name,
            "duration" => Field::Duration,
            "predecessors" => Field::Predecessors,
            "resource names" => Field::ResourceNames,
            "outline level" => Field::OutlineLevel,
            "constraint type" => Field::ConstraintType,
            "constraint date" => Field::ConstraintDate,
            "deadline" => Field::Deadline,
            "fixed cost" => Field::FixedCost,
            "type" => Field::Type,
            "effort driven" => Field::EffortDriven,
            "priority" => Field::Priority,
            "leveling delay" | "levelling delay" => Field::LevelingDelay,
            "id" => Field::Id,
            "start" => Field::Start,
            "cost" => Field::Cost,
            _ => return None,
        })
    }
}

/// The heading row: where each column ProjectCraft reads is, and what else the file has.
struct Sheet {
    at: HashMap<Field, usize>,
    /// Columns read, as (field, index), for picking cells out of a row.
    read: Vec<(Field, usize)>,
    /// Every heading, trimmed, for the report.
    headings: Vec<String>,
    /// Columns that are a report line: (index, why) in heading order.
    other: Vec<(usize, &'static str)>,
    /// Untitled columns that hold something in some row.
    untitled_used: HashSet<usize>,
    /// Headings past [`MAX_COLUMNS`].
    past: usize,
}

const NOT_READ: &str = "ProjectCraft doesn't read this column from CSV yet";
const DATES: &str = "ProjectCraft works out every task's dates; the file's are not used";
const REPEATED: &str = "the same column again; the first is read";

/// One row's cells in the columns ProjectCraft reads, trimmed; `None` when empty.
struct Row {
    row: usize,
    cells: HashMap<Field, String>,
    /// Cells past the last heading hold something.
    extra: bool,
}

impl Row {
    fn get(&self, f: Field) -> Option<&str> {
        self.cells.get(&f).map(String::as_str)
    }
}

impl Sheet {
    fn of(headings: &[String], past: usize) -> Sheet {
        let mut at = HashMap::new();
        let mut other = Vec::new();
        for (i, h) in headings.iter().enumerate() {
            match Field::of(h) {
                Some(f) if at.contains_key(&f) => other.push((i, REPEATED)),
                Some(f) => {
                    at.insert(f, i);
                    if f == Field::Start {
                        other.push((i, "ProjectCraft works out every task's dates; the earliest start is the project's start"));
                    }
                    if f == Field::Cost {
                        other.push((i, "ProjectCraft works out each task's cost from its resources and fixed cost; the file's are not used"));
                    }
                }
                None if h.trim().is_empty() => {}
                None if h.trim().eq_ignore_ascii_case("finish") => other.push((i, DATES)),
                None if h.trim().eq_ignore_ascii_case("work") => {
                    other.push((i, "ProjectCraft works out each task's work from its duration and Resource Names; the file's are not used"))
                }
                None if h.trim().eq_ignore_ascii_case("unique id") => other.push((i, "tasks are numbered from 1 in the file's order")),
                None => other.push((i, NOT_READ)),
            }
        }
        let read = Field::READ.iter().filter_map(|f| at.get(f).map(|&i| (*f, i))).collect();
        Sheet { at, read, headings: headings.iter().map(|h| h.trim().to_owned()).collect(), other, untitled_used: HashSet::new(), past }
    }

    fn at(&self, f: Field) -> Option<usize> {
        self.at.get(&f).copied()
    }

    fn row(&mut self, row: usize, cells: Vec<String>, quoted: &[bool], past: usize) -> Row {
        let mut out = HashMap::new();
        for &(f, i) in &self.read {
            let Some(raw) = cells.get(i) else { continue };
            let c = raw.trim();
            let kept = f == Field::Name && quoted.get(i) == Some(&true);
            if c.is_empty() && !(kept && !raw.is_empty()) {
                continue;
            }
            let c = match f {
                Field::Name if kept => crate::csv_export::unguard(raw),
                Field::Name | Field::ResourceNames => crate::csv_export::unguard(c),
                _ => c,
            };
            out.insert(f, c.to_owned());
        }
        for (i, c) in cells.iter().enumerate() {
            if !c.trim().is_empty() && self.headings.get(i).is_some_and(|h| h.is_empty()) {
                self.untitled_used.insert(i);
            }
        }
        let extra = past > 0 || cells.iter().skip(self.headings.len()).any(|c| !c.trim().is_empty());
        Row { row, cells: out, extra }
    }

    /// A line per column not read, one for the untitled columns used, one for columns past the cap.
    fn lines(&self) -> Vec<ReportLine> {
        let mut lines = Vec::new();
        for &(i, why) in &self.other {
            let heading: String = self.headings.get(i).map_or("", String::as_str).chars().take(40).collect();
            lines.push(line(Outcome::Dropped, &format!("{heading} column"), &format!("Column {}", letters(i)), why));
        }
        if !self.untitled_used.is_empty() {
            let mut used: Vec<usize> = self.untitled_used.iter().copied().collect();
            used.sort_unstable();
            let named: Vec<String> = used.iter().take(NAMED_IN_LINE).map(|&i| letters(i)).collect();
            let (feature, mut place) = if used.len() == 1 {
                ("Untitled column", format!("Column {}", named.join(", ")))
            } else {
                ("Untitled columns", format!("Columns {}", named.join(", ")))
            };
            if used.len() > NAMED_IN_LINE {
                place = format!("{place} and {} more", used.len() - NAMED_IN_LINE);
            }
            lines.push(line(Outcome::Dropped, feature, &place, "a column without a heading isn't read"));
        }
        if self.past > 0 {
            lines.push(line(
                Outcome::Dropped,
                "More columns",
                &format!("Columns past the {MAX_COLUMNS}th"),
                &format!("ProjectCraft reads the first {MAX_COLUMNS} columns"),
            ));
        }
        lines
    }
}

/// A spreadsheet's column letters: 0 is A, 25 Z, 26 AA.
fn letters(index: usize) -> String {
    let mut n = index.saturating_add(1);
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(char::from(b'A' + rem as u8));
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

/// Something about a row that the report mentions. The order is the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Note {
    NoName,
    NameLines,
    NoDuration,
    BadDuration,
    LongDuration,
    Estimated,
    BadLevel,
    Outline,
    BadPreds,
    MissingPred,
    SelfLink,
    RepeatedLink,
    SummaryLink,
    Loop,
    BadResources,
    TooManyResources,
    BadConstraintType,
    NoConstraintDate,
    BadConstraintDate,
    DateWithoutType,
    BadDeadline,
    BadFixedCost,
    BadType,
    BadEffortDriven,
    BadPriority,
    BadLevelingDelay,
    SummaryFields,
    ExtraCells,
}

impl Note {
    const ALL: [Note; 28] = [
        Note::NoName,
        Note::NameLines,
        Note::NoDuration,
        Note::BadDuration,
        Note::LongDuration,
        Note::Estimated,
        Note::BadLevel,
        Note::Outline,
        Note::BadPreds,
        Note::MissingPred,
        Note::SelfLink,
        Note::RepeatedLink,
        Note::SummaryLink,
        Note::Loop,
        Note::BadResources,
        Note::TooManyResources,
        Note::BadConstraintType,
        Note::NoConstraintDate,
        Note::BadConstraintDate,
        Note::DateWithoutType,
        Note::BadDeadline,
        Note::BadFixedCost,
        Note::BadType,
        Note::BadEffortDriven,
        Note::BadPriority,
        Note::BadLevelingDelay,
        Note::SummaryFields,
        Note::ExtraCells,
    ];

    fn bit(self) -> u32 {
        1 << (self as u8)
    }

    /// Outcome, feature and why.
    fn line(self) -> (Outcome, &'static str, &'static str) {
        use Outcome::*;
        const LINKS: &str = "Links (predecessors)";
        match self {
            Note::NoName => (Converted, "Task Name", "empty; the task is imported without a name"),
            Note::NameLines => (Converted, "Task Name", "line breaks in the name are read as spaces"),
            Note::NoDuration => (Converted, "Duration", "the file gives none; 1 day used"),
            Note::BadDuration => {
                (Converted, "Duration", "unreadable (a number with m, h, d, w or mo; elapsed durations aren't supported yet); 1 day used")
            }
            Note::LongDuration => (Converted, "Duration", "longer than ProjectCraft holds; cut to 100,000 days"),
            Note::Estimated => (Dropped, "Estimated duration (?)", "ProjectCraft has no estimate mark; the duration is kept"),
            Note::BadLevel => (Converted, "Outline Level", "unreadable or out of range (1 to 100); level 1 used"),
            Note::Outline => (Converted, "Outline level", "deeper than the row above allows; moved up to fit the outline"),
            Note::BadPreds => {
                (Dropped, "Predecessors", "unreadable (row numbers, each with an optional type and lag: 3, 5SS+2d); the task has no links")
            }
            Note::MissingPred => (Dropped, LINKS, "a predecessor row that isn't a task; that link is dropped"),
            Note::SelfLink => (Dropped, LINKS, "a task can't be its own predecessor; that link is dropped"),
            Note::RepeatedLink => (Dropped, LINKS, "the same predecessor twice; the first link is kept"),
            Note::SummaryLink => (Dropped, LINKS, "a link between a summary task and its own subtask; that link is dropped"),
            Note::Loop => (Dropped, LINKS, "a link that would close a loop of links; that link is dropped"),
            Note::BadResources => (Dropped, "Resource Names", "unreadable; the task has no resources"),
            Note::TooManyResources => (Dropped, "Resource Names", "more resources or assignments than a project holds; the task has no resources"),
            Note::BadConstraintType => (Converted, "Constraint Type", "unreadable; As Soon As Possible used"),
            Note::NoConstraintDate => (Converted, "Constraint Date", "the constraint needs a date and the file gives none; As Soon As Possible used"),
            Note::BadConstraintDate => (Converted, "Constraint Date", "unreadable (dates are read as YYYY-MM-DD); As Soon As Possible used"),
            Note::DateWithoutType => (Dropped, "Constraint Date", "no Constraint Type goes with it; As Soon As Possible used"),
            Note::BadDeadline => (Dropped, "Deadline", "unreadable (dates are read as YYYY-MM-DD); the task has no deadline"),
            Note::BadFixedCost => (Dropped, "Fixed Cost", "unreadable (an amount in the project's currency, such as £1,250.00); no fixed cost"),
            Note::BadType => (Converted, "Type", "unreadable (Fixed Units, Fixed Duration or Fixed Work); the project's default type used"),
            Note::BadEffortDriven => (Converted, "Effort Driven", "unreadable (Yes or No); the project's default used"),
            Note::BadPriority => (Converted, "Priority", "unreadable or out of range (0 to 1000); 500 used"),
            Note::BadLevelingDelay => (Dropped, "Leveling Delay", "unreadable (a duration such as 2d); no delay"),
            Note::SummaryFields => {
                (Dropped, "Summary task fields", "a summary's Type, Effort Driven and Leveling Delay follow its subtasks; the file's are not used")
            }
            Note::ExtraCells => (Dropped, "Cells past the last column", "a cell without a heading isn't read"),
        }
    }
}

fn line(outcome: Outcome, feature: &str, place: &str, why: &str) -> ReportLine {
    ReportLine { outcome, feature: feature.into(), place: place.into(), why: why.into() }
}

fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// `2026-11-09`, with or without a time after it (`2026-11-09T08:00:00`, `2026-11-09 08:00`).
/// `None` when unreadable: other orders (`09/11/2026`) depend on the saving machine's locale.
fn date_of(text: &str) -> Option<Date> {
    let day = text.trim().split(['T', ' ']).next()?;
    let mut parts = day.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, n: std::ops::RangeInclusive<usize>| n.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if parts.next().is_some() || !digits(y, 4..=4) || !digits(m, 1..=2) || !digits(d, 1..=2) {
        return None;
    }
    Date::from_ymd(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
}

/// A date cell: `None` for empty or `NA` (as Project shows no date).
fn date_cell(text: Option<&str>) -> Option<Result<Date, ()>> {
    let t = text?.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("na") || t.eq_ignore_ascii_case("n/a") {
        return None;
    }
    Some(date_of(t).ok_or(()))
}

/// A duration with a decimal comma (`1,5 days`, as semicolon files from many locales write it)
/// with a decimal point instead.
fn decimal_point(text: &str) -> String {
    if !text.contains('.') { text.replacen(',', ".", 1) } else { text.to_owned() }
}

/// An Effort Driven cell: Yes or No (or true, false, 1, 0), any case.
fn yes_no(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// The currency the amounts are in: the one whose symbol starts the first Fixed Cost or Cost
/// amount ([`model::KNOWN_CURRENCIES`]; the first listed when two share a symbol), with a report
/// line; `None` (the project's default) when the symbol is the default's, unknown or missing.
fn currency(rows: &[Row], lines: &mut Vec<ReportLine>) -> Option<model::Currency> {
    let amount = rows.iter().find_map(|r| r.get(Field::FixedCost).or_else(|| r.get(Field::Cost)))?;
    let symbol: String = amount.chars().take_while(|c| !c.is_ascii_digit() && !matches!(c, '.' | ',' | '-' | '+' | ' ')).collect();
    let default = model::Currency::default();
    if symbol.is_empty() || symbol == default.symbol {
        return None;
    }
    let (code, _, _) = model::KNOWN_CURRENCIES.iter().find(|(_, s, _)| *s == symbol)?;
    let c = model::Currency::new(code, None, None).ok()?;
    lines.push(line(
        Outcome::Converted,
        "Currency",
        "Project",
        &format!("a CSV has no currency; the amounts' symbol, {}, is read as {}", c.symbol, c.code),
    ));
    Some(c)
}

/// A summary's Type, Effort Driven and Leveling Delay follow its subtasks: the project's
/// defaults are kept, and a cell that says otherwise than the sheet shows for a summary
/// (Fixed Duration, No, no delay) is a note.
fn summaries(p: &mut Project, rows: &[Row], notes: &mut [u32]) {
    let outline = Outline::of(p);
    let options = p.options;
    for (i, t) in p.tasks.iter_mut().enumerate() {
        if !outline.is_summary(i) {
            continue;
        }
        let differs = rows.get(i).is_some_and(|r| {
            r.get(Field::Type).is_some_and(|x| model::TaskType::from_name(x).ok() != Some(model::TaskType::FixedDuration))
                || r.get(Field::EffortDriven).is_some_and(|x| yes_no(x) != Some(false))
                || r.get(Field::LevelingDelay).is_some_and(|x| !matches!(model::parse_duration(&decimal_point(x), &options), Ok((0, _))))
        });
        if let Some(n) = notes.get_mut(i) {
            // Said once, as this line, rather than as the cells' own.
            *n &= !(Note::BadType.bit() | Note::BadEffortDriven.bit() | Note::BadLevelingDelay.bit());
            if differs {
                *n |= Note::SummaryFields.bit();
            }
        }
        t.task_type = options.task_type;
        t.effort_driven = options.effort_driven || options.task_type == model::TaskType::FixedWork;
        t.leveling_delay = 0;
    }
}

/// Each row's task, numbered from 1, with the notes on it.
fn tasks(rows: &[Row], p: &Project) -> (Vec<Task>, Vec<u32>) {
    let options = &p.options;
    let mut out = Vec::with_capacity(rows.len());
    let mut notes = Vec::with_capacity(rows.len());
    for (k, r) in rows.iter().enumerate() {
        let mut n = 0u32;
        let mut note = |x: Note| n |= x.bit();
        let raw = r.get(Field::Name).unwrap_or("");
        if raw.is_empty() {
            note(Note::NoName);
        }
        let name: String = if raw.contains(['\n', '\r']) {
            note(Note::NameLines);
            raw.split(['\n', '\r']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
        } else {
            raw.to_owned()
        };
        let (duration, unit) = match r.get(Field::Duration) {
            None => (options.day(), DurationUnit::Days),
            Some(text) => {
                let text = match text.strip_suffix('?') {
                    Some(t) => {
                        note(Note::Estimated);
                        t
                    }
                    None => text,
                };
                match model::parse_duration(&decimal_point(text), options) {
                    Ok((m, unit)) if m >= MAX_MINUTES => {
                        note(Note::LongDuration);
                        (MAX_MINUTES, unit)
                    }
                    Ok((m, unit)) => (m.max(0), unit),
                    Err(_) => {
                        note(Note::BadDuration);
                        (options.day(), DurationUnit::Days)
                    }
                }
            }
        };
        let level = match r.get(Field::OutlineLevel).map(|t| t.parse::<u32>()) {
            None => 1,
            Some(Ok(l)) if (1..=MAX_OUTLINE_LEVEL).contains(&l) => l,
            Some(_) => {
                note(Note::BadLevel);
                1
            }
        };
        let constraint = constraint(r, &mut note);
        let deadline = match date_cell(r.get(Field::Deadline)) {
            None => None,
            Some(Ok(d)) => Some(d),
            Some(Err(())) => {
                note(Note::BadDeadline);
                None
            }
        };
        let fixed_cost = match r.get(Field::FixedCost).map(|t| p.currency.parse(t)) {
            None => 0,
            Some(Ok(m)) => m,
            Some(Err(_)) => {
                note(Note::BadFixedCost);
                0
            }
        };
        let task_type = match r.get(Field::Type).map(model::TaskType::from_name) {
            None => options.task_type,
            Some(Ok(t)) => t,
            Some(Err(_)) => {
                note(Note::BadType);
                options.task_type
            }
        };
        let effort_driven = match r.get(Field::EffortDriven).map(yes_no) {
            None => options.effort_driven,
            Some(Some(on)) => on,
            Some(None) => {
                note(Note::BadEffortDriven);
                options.effort_driven
            }
        };
        let priority = match r.get(Field::Priority).map(|t| t.parse::<i64>().ok().and_then(|n| model::check_priority(n).ok())) {
            None => model::DEFAULT_PRIORITY,
            Some(Some(n)) => n,
            Some(None) => {
                note(Note::BadPriority);
                model::DEFAULT_PRIORITY
            }
        };
        let leveling_delay = match r.get(Field::LevelingDelay).map(|t| model::parse_duration(&decimal_point(t), options)) {
            None => 0,
            Some(Ok((m, _))) => m.clamp(0, MAX_MINUTES),
            Some(Err(_)) => {
                note(Note::BadLevelingDelay);
                0
            }
        };
        if r.extra {
            note(Note::ExtraCells);
        }
        let id = TaskId(u32::try_from(k).unwrap_or(u32::MAX - 1).saturating_add(1));
        out.push(Task {
            id,
            name,
            duration,
            duration_unit: unit,
            preds: Vec::new(),
            constraint,
            outline_level: level,
            deadline,
            fixed_cost,
            task_type,
            effort_driven: effort_driven || task_type == model::TaskType::FixedWork,
            priority,
            leveling_delay,
        });
        notes.push(n);
    }
    (out, notes)
}

/// The row's constraint: a code (`SNET`) or the name Project shows (`Start No Earlier Than`).
fn constraint(r: &Row, note: &mut impl FnMut(Note)) -> Constraint {
    let date = date_cell(r.get(Field::ConstraintDate));
    let Some(kind) = r.get(Field::ConstraintType) else {
        if date.is_some() {
            note(Note::DateWithoutType);
        }
        return Constraint::Asap;
    };
    let words = kind.split_whitespace().collect::<Vec<_>>().join(" ");
    let code = Constraint::CODES.into_iter().find(|&c| {
        c.eq_ignore_ascii_case(&words) || Constraint::from_code(c, Some(Date::EPOCH)).is_ok_and(|k| k.name().eq_ignore_ascii_case(&words))
    });
    let Some(code) = code else {
        note(Note::BadConstraintType);
        return Constraint::Asap;
    };
    // As Soon and As Late As Possible take no date.
    if let Ok(c) = Constraint::from_code(code, None) {
        return c;
    }
    match date {
        None => {
            note(Note::NoConstraintDate);
            Constraint::Asap
        }
        Some(Err(())) => {
            note(Note::BadConstraintDate);
            Constraint::Asap
        }
        Some(Ok(d)) => Constraint::from_code(code, Some(d)).unwrap_or_default(),
    }
}

/// The project's start: the earliest Start in the file, else the earliest constraint date, else
/// 1970-01-01; a report line says which.
fn start(rows: &[Row], p: &Project, lines: &mut Vec<ReportLine>) -> Date {
    let earliest_start = rows.iter().filter_map(|r| r.get(Field::Start).and_then(date_of)).min();
    let earliest_constraint = p.tasks.iter().filter_map(|t| t.constraint.date()).min();
    let (date, why) = match (earliest_start, earliest_constraint) {
        (Some(d), _) => (d, format!("a CSV has no project start; the earliest Start in the file, {d}, is used")),
        (None, Some(d)) => (d, format!("a CSV has no project start; the earliest constraint date, {d}, is used")),
        (None, None) => (Date::EPOCH, "a CSV has no project start; 1970-01-01 is used: set it in Project Information".to_owned()),
    };
    lines.push(line(Outcome::Converted, "Project start", "Project", &why));
    date
}

/// Rows whose level breaks the outline rule move up to fit. A task with no duration that turns
/// out to be a summary needs none: its duration is rolled up.
fn fit_outline(p: &mut Project, notes: &mut [u32], has_duration: bool, rows: &[Row]) {
    let outline = Outline::of(p);
    for (i, (t, n)) in p.tasks.iter_mut().zip(notes.iter_mut()).enumerate() {
        if let Some(&l) = outline.levels.get(i)
            && t.outline_level != l
        {
            t.outline_level = l;
            *n |= Note::Outline.bit();
        }
        let given = rows.get(i).is_some_and(|r| r.get(Field::Duration).is_some());
        if has_duration && !given && !outline.is_summary(i) {
            *n |= Note::NoDuration.bit();
        }
    }
}

/// Each row's Predecessors, by row, as links by Unique ID. Links to rows that aren't tasks, to the
/// task itself, repeated, between a summary and its own subtask, or closing a loop are dropped.
fn links(p: &mut Project, rows: &[Row], notes: &mut [u32]) {
    let options = p.options;
    let outline = Outline::of(p);
    // Rows are increasing: a row number's task is found by search.
    let row_numbers: Vec<usize> = rows.iter().map(|r| r.row).collect();
    let mut all = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let mut preds: Vec<Link> = Vec::new();
        let mut kept = HashSet::new();
        let mut n = 0u32;
        if let Some(text) = r.get(Field::Predecessors) {
            match model::parse_preds(text, &options) {
                Err(_) => n |= Note::BadPreds.bit(),
                Ok(list) => {
                    for l in list {
                        let Some(j) = usize::try_from(l.pred.0).ok().and_then(|row| row_numbers.binary_search(&row).ok()) else {
                            n |= Note::MissingPred.bit();
                            continue;
                        };
                        let Some(pred) = p.tasks.get(j).map(|t| t.id) else { continue };
                        match crate::links::refused(&outline, &kept, j, i, pred) {
                            Some(Refused::SelfLink) => n |= Note::SelfLink.bit(),
                            Some(Refused::Repeated) => n |= Note::RepeatedLink.bit(),
                            Some(Refused::Summary) => n |= Note::SummaryLink.bit(),
                            None => {
                                kept.insert(pred);
                                preds.push(Link { pred, ..l });
                            }
                        }
                    }
                }
            }
        }
        all.push(preds);
        if let Some(x) = notes.get_mut(i) {
            *x |= n;
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
}

/// Each row's Resource Names, read as the column reads them ([`Project::set_resource_names`]).
/// A name not yet a resource becomes one of the type its units say ([`kind_of`]): a work
/// resource, a cost resource for an amount, a material for a quantity. Each task is assigned on a
/// project of its own with only the resources it names, so the work stays in proportion to the
/// row, not the file. Returns the first few reasons a cell couldn't be read, by row index.
fn resources(p: &mut Project, rows: &[Row], notes: &mut [u32]) -> Vec<(usize, String)> {
    let mut errors = Vec::new();
    // Lowercased name → index in `p.resources`.
    let mut known: HashMap<String, usize> = HashMap::new();
    let outline = Outline::of(p);
    for (i, r) in rows.iter().enumerate() {
        let Some(text) = r.get(Field::ResourceNames) else { continue };
        let Some(task) = p.tasks.get(i).cloned() else { continue };
        let mut own = Project::new("", p.start);
        own.options = p.options;
        own.currency = p.currency.clone();
        own.next_resource_id = p.next_resource_id;
        let id = task.id;
        own.tasks = vec![Task { outline_level: 1, preds: Vec::new(), ..task }];
        let mut picked = HashSet::new();
        let mut made = Vec::new();
        for (name, units) in parts_in(text) {
            let lower = name.to_lowercase();
            if let Some(&k) = known.get(&lower) {
                if picked.insert(k)
                    && let Some(res) = p.resources.get(k)
                {
                    own.resources.push(res.clone());
                }
            } else if !own.resources.iter().any(|r| r.name.to_lowercase() == lower) && own.resources.len() < MAX_RESOURCES {
                let (kind, label) = kind_of(units.as_deref(), &own.currency);
                if let Ok(new) = own.add_resource(Some(&name), kind, None) {
                    if let Some(res) = own.resources.iter_mut().find(|r| r.id == new) {
                        res.material_label = label;
                    }
                    made.push(new);
                }
            }
        }
        let made = match own.set_resource_names(id, text) {
            Ok(also) => {
                made.extend(also);
                made
            }
            Err(e) => {
                if let Some(x) = notes.get_mut(i) {
                    *x |= Note::BadResources.bit();
                }
                if errors.len() < NAMED_IN_LINE {
                    errors.push((i, e.to_string()));
                }
                continue;
            }
        };
        if p.resources.len().saturating_add(made.len()) > MAX_RESOURCES || p.assignments.len().saturating_add(own.assignments.len()) > MAX_ASSIGNMENTS
        {
            if let Some(x) = notes.get_mut(i) {
                *x |= Note::TooManyResources.bit();
            }
            continue;
        }
        // A name this task's cell spelled so that it wasn't looked up, made again: refused.
        if own.resources.iter().any(|r| made.contains(&r.id) && known.contains_key(&r.name.to_lowercase())) {
            if let Some(x) = notes.get_mut(i) {
                *x |= Note::BadResources.bit();
            }
            continue;
        }
        for res in own.resources.into_iter().filter(|r| made.contains(&r.id)) {
            known.insert(res.name.to_lowercase(), p.resources.len());
            p.resources.push(res);
        }
        p.next_resource_id = own.next_resource_id;
        p.assignments.extend(own.assignments);
        // A summary's duration is rolled up from its subtasks; any other keeps what assigning gave it.
        if !outline.is_summary(i)
            && let (Some(t), Some(o)) = (p.tasks.get_mut(i), own.tasks.first())
        {
            t.duration = o.duration;
        }
    }
    errors
}

/// The resources named in a Resource Names cell, each cleaned as a name is
/// ([`model::check_name`]) with the text of its first brackets, for looking them up and for
/// making the new ones; the cell itself is read by [`Project::set_resource_names`].
fn parts_in(text: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let (mut name, mut units) = (String::new(), None::<String>);
    let mut depth = 0u32;
    for c in text.chars().chain(std::iter::once(',')) {
        match c {
            '[' => {
                depth = depth.saturating_add(1);
                if depth == 1 && units.is_none() {
                    units = Some(String::new());
                }
                continue;
            }
            ']' => {
                depth = depth.saturating_sub(1);
                continue;
            }
            ',' if depth == 0 => {
                if let Ok(n) = model::check_name(&name) {
                    out.push((n, units.take()));
                }
                name.clear();
                units = None;
                continue;
            }
            _ => {}
        }
        match (&mut units, depth) {
            // The name ends at its brackets.
            (None, 0) => name.push(c),
            (Some(u), 1) => u.push(c),
            _ => {}
        }
    }
    out
}

/// The type of resource a new name's units say it is, and a material's label: a work resource
/// for none or a percentage (`50%`); a cost resource for an amount in the project's currency
/// (`£200.00`); a material for a quantity (`32 boxes`, `2 boxes/h`, labelled `boxes`). Anything
/// else is a work resource, and its units are refused with the cell.
fn kind_of(units: Option<&str>, currency: &model::Currency) -> (model::ResourceType, String) {
    use model::ResourceType::{Cost, Material, Work};
    let Some(u) = units.map(str::trim).filter(|u| !u.is_empty()) else { return (Work, String::new()) };
    if u.ends_with('%') {
        return (Work, String::new());
    }
    if !currency.symbol.is_empty() && u.starts_with(currency.symbol.as_str()) && currency.parse(u).is_ok() {
        return (Cost, String::new());
    }
    if model::parse_assignment_units(Material, u, currency, model::UnitsFormat::Percentage).is_ok() {
        let amount = u.rsplit_once('/').map_or(u, |(a, _)| a).trim();
        let label = amount.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').trim();
        return (Material, label.chars().take(model::MAX_RESOURCE_TEXT).collect::<String>().trim().to_owned());
    }
    (Work, String::new())
}

/// One line per note, naming the first few rows that have it.
fn task_lines(tasks: &[Task], rows: &[Row], notes: &[u32], errors: &[(usize, String)]) -> Vec<ReportLine> {
    let mut out = Vec::new();
    for n in Note::ALL {
        let with: Vec<usize> = notes.iter().enumerate().filter(|(_, bits)| *bits & n.bit() != 0).map(|(i, _)| i).collect();
        if with.is_empty() {
            continue;
        }
        let named: Vec<String> = with
            .iter()
            .take(NAMED_IN_LINE)
            .filter_map(|&i| {
                let row = rows.get(i)?.row;
                let name: String = tasks.get(i)?.name.chars().take(40).collect();
                Some(if name.is_empty() { format!("Row {row}") } else { format!("Row {row} {name}") })
            })
            .collect();
        let mut place = named.join(", ");
        if with.len() > NAMED_IN_LINE {
            place = format!("{place} and {}", plural(with.len() - NAMED_IN_LINE, "more task"));
        }
        let (outcome, feature, why) = n.line();
        let why = match n {
            Note::BadResources if !errors.is_empty() => {
                let said: Vec<String> = errors
                    .iter()
                    .filter_map(|(i, e)| Some(format!("row {}: {}", rows.get(*i)?.row, e.chars().take(80).collect::<String>())))
                    .collect();
                format!("{why} ({})", said.join("; "))
            }
            _ => why.to_owned(),
        };
        out.push(line(outcome, feature, &place, &why));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(text: &str) -> Vec<Vec<String>> {
        let mut r = Reader::new(text, separator(text));
        let mut out = Vec::new();
        while let Some((cells, _)) = r.record(MAX_COLUMNS) {
            out.push(cells);
        }
        out
    }

    #[test]
    fn records_read_as_rfc_4180() {
        assert_eq!(all("a,b\r\nc,d"), [["a", "b"], ["c", "d"]]);
        assert_eq!(all("a,b\n"), [["a", "b"]], "a last line break ends the last record");
        assert_eq!(all("\"a,b\",\"say \"\"hi\"\"\"\n"), [["a,b", "say \"hi\""]]);
        assert_eq!(all("\"two\nlines\",x\rnext,y"), [["two\nlines", "x"], ["next", "y"]]);
        assert_eq!(all("a;b;\"c;d\"\n"), [["a", "b", "c;d"]], "semicolons when they outnumber commas");
        assert_eq!(all("a,b;c\n"), [["a", "b;c"]]);
        assert_eq!(all("\"a\"x,b\n"), [["ax", "b"]], "text after a closing quote is kept");
        assert_eq!(all("a\"b,c\n"), [["a\"b", "c"]], "a quote inside an unquoted cell is kept");
        assert_eq!(all("\n,\n"), [vec![""], vec!["", ""]]);
        assert_eq!(all("héllo,wörld\n"), [["héllo", "wörld"]]);
        let mut r = Reader::new("\"open", b',');
        assert_eq!(r.record(9).map(|(c, _)| c), Some(vec!["open".to_owned()]));
        assert!(r.unclosed);
        assert_eq!(r.record(9), None);
    }

    #[test]
    fn cells_past_the_cap_are_counted() {
        let mut r = Reader::new("a,b,c,d\n", b',');
        assert_eq!(r.record(2), Some((vec!["a".to_owned(), "b".to_owned()], 2)));
    }

    #[test]
    fn column_letters_count_as_a_spreadsheet_does() {
        assert_eq!([0, 25, 26, 51, 52, 701, 702].map(letters), ["A", "Z", "AA", "AZ", "BA", "ZZ", "AAA"]);
    }

    #[test]
    fn dates_read_as_iso_only() {
        assert_eq!(date_of("2026-11-09"), Date::from_ymd(2026, 11, 9));
        assert_eq!(date_of("2026-11-09T08:00:00"), Date::from_ymd(2026, 11, 9));
        assert_eq!(date_of("2026-1-9 08:00"), Date::from_ymd(2026, 1, 9));
        for bad in ["09/11/2026", "2026-13-01", "26-11-09", "202611-09", "-2026-1-1", "2026-11-09-1", "", "Mon 09/11/26"] {
            assert_eq!(date_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_in_a_cell_are_found_for_lookup() {
        let s = |n: &str, u: Option<&str>| (n.to_owned(), u.map(str::to_owned));
        assert_eq!(
            parts_in("Ann[50%], Bob ,Concrete[2 tons/d], Crane[£200.00]x, Odd[[a]b]"),
            [s("Ann", Some("50%")), s("Bob", None), s("Concrete", Some("2 tons/d")), s("Crane", Some("£200.00")), s("Odd", Some("b"))]
        );
        assert_eq!(parts_in(",,[x]"), Vec::<(String, Option<String>)>::new());
    }

    #[test]
    fn a_new_name_s_units_say_its_type() {
        use model::ResourceType::{Cost, Material, Work};
        let gbp = model::Currency::default();
        let k = |u: Option<&str>| kind_of(u, &gbp);
        assert_eq!(k(None), (Work, String::new()));
        assert_eq!(k(Some("50%")), (Work, String::new()));
        assert_eq!(k(Some("£1,200.50")), (Cost, String::new()));
        assert_eq!(k(Some("32 boxes")), (Material, "boxes".to_owned()));
        assert_eq!(k(Some("2 m³/h")), (Material, "m³".to_owned()));
        assert_eq!(k(Some("5")), (Material, String::new()));
        assert_eq!(k(Some("lots")), (Work, String::new()), "refused with the cell");
        assert_eq!(kind_of(Some("$5.00"), &model::Currency::new("USD", None, None).unwrap()), (Cost, String::new()));
    }

    #[test]
    fn constraint_types_read_as_codes_or_names() {
        let row = |kind: &str, date: &str| Row {
            row: 1,
            cells: [(Field::ConstraintType, kind.to_owned()), (Field::ConstraintDate, date.to_owned())].into_iter().collect(),
            extra: false,
        };
        let mut n = 0u32;
        let d = Date::from_ymd(2026, 3, 2).unwrap();
        assert_eq!(constraint(&row("snet", "2026-03-02"), &mut |x| n |= x.bit()), Constraint::StartNoEarlierThan(d));
        assert_eq!(constraint(&row("Must  Finish On", "2026-03-02"), &mut |x| n |= x.bit()), Constraint::MustFinishOn(d));
        assert_eq!(constraint(&row("As Soon As Possible", "NA"), &mut |x| n |= x.bit()), Constraint::Asap);
        assert_eq!(n, 0);
    }
}
