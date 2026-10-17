//! CSV export: the task sheet as a spreadsheet opens it. The heading row is `ID`, `Task Name` and
//! the sheet's shown columns, left to right as the column chooser has them, with their headings;
//! each row is a task, every cell exactly as the sheet shows it ([`SheetText`]). A plan with
//! summary tasks gets an `Outline Level` column last, since a CSV can't indent a name.
//!
//! The file is what the CSV importer and spreadsheet programs read best: UTF-8 with a byte order
//! mark (without one, Excel reads a double-clicked CSV in the machine's legacy code page and
//! garbles `café`), commas, CRLF line ends, and RFC 4180 quoting
//! (<https://www.rfc-editor.org/rfc/rfc4180>) of every cell holding a comma, semicolon, quote or
//! line break, or starting or ending with a space, so nothing is split or trimmed.
//!
//! Formula injection: a spreadsheet runs a cell that starts with `=`, `+`, `-` or `@` (or a tab
//! or carriage return before one) as a formula. The cells that hold the owner's own text, Task
//! Name and Resource Names, are written with an apostrophe in front when they start like that,
//! after any apostrophes of their own; spreadsheets show such a cell as text and hide the
//! apostrophe, and the CSV importer takes it off again ([`unguard`]). Every other cell is
//! written by ProjectCraft from a number or a date and can't start like a formula.

use std::borrow::Cow;

use crate::model::{Column, Project, Schedule, SheetText};

/// The byte order mark the file starts with.
const BOM: char = '\u{FEFF}';
/// What a spreadsheet takes for the start of a formula.
const FORMULA_START: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

pub(crate) fn export(p: &Project, schedule: Option<&Schedule>) -> String {
    let sheet = SheetText::new(p, schedule);
    let columns = p.columns.shown();
    let outline = (0..sheet.len()).any(|row| sheet.level(row) > 1);
    let mut out = String::new();
    out.push(BOM);
    let mut headings = vec!["ID", "Task Name"];
    headings.extend(columns.iter().map(|c| c.name()));
    if outline {
        headings.push("Outline Level");
    }
    record(&mut out, headings.into_iter().map(Cow::Borrowed));
    for row in 0..sheet.len() {
        let mut cells: Vec<Cow<str>> = Vec::with_capacity(columns.len() + 3);
        cells.push(Cow::Owned(row.saturating_add(1).to_string()));
        cells.push(guard(sheet.name(row).to_owned()));
        for &c in columns {
            let text = sheet.cell(row, c);
            cells.push(if c == Column::ResourceNames { guard(text) } else { Cow::Owned(text) });
        }
        if outline {
            cells.push(Cow::Owned(sheet.level(row).to_string()));
        }
        record(&mut out, cells.into_iter());
    }
    out
}

/// One record: the cells, quoted where they need it, separated by commas, ending with CRLF.
fn record<'a>(out: &mut String, cells: impl Iterator<Item = Cow<'a, str>>) {
    for (i, cell) in cells.enumerate() {
        if i > 0 {
            out.push(',');
        }
        quote_into(out, &cell);
    }
    out.push_str("\r\n");
}

/// A cell as RFC 4180 writes it: in quotes, each quote doubled, when it holds a separator (either
/// one, for spreadsheets set to semicolons), a quote or a line break, or has spaces at its ends.
fn quote_into(out: &mut String, cell: &str) {
    let edge = |c: Option<char>| c.is_some_and(char::is_whitespace);
    if cell.contains([',', ';', '"', '\n', '\r']) || edge(cell.chars().next()) || edge(cell.chars().next_back()) {
        out.push('"');
        out.push_str(&cell.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

/// Whether a spreadsheet would run `text`, once any apostrophes in front are passed over, as a
/// formula.
pub(crate) fn formula_like(text: &str) -> bool {
    text.trim_start_matches('\'').starts_with(FORMULA_START)
}

/// The owner's text with an apostrophe in front when it starts like a formula.
fn guard(text: String) -> Cow<'static, str> {
    if formula_like(&text) { Cow::Owned(format!("'{text}")) } else { Cow::Owned(text) }
}

/// The text a guarded cell held: the apostrophe [`guard`] put in front taken off. Text that
/// doesn't start like a formula is left alone, so `'Tis` stays as it is.
pub(crate) fn unguard(text: &str) -> &str {
    match text.strip_prefix('\'') {
        Some(rest) if formula_like(rest) => rest,
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted(cell: &str) -> String {
        let mut out = String::new();
        quote_into(&mut out, cell);
        out
    }

    #[test]
    fn cells_are_quoted_only_when_they_need_it() {
        assert_eq!(quoted("Dig"), "Dig");
        assert_eq!(quoted("café Ω 日本"), "café Ω 日本");
        assert_eq!(quoted("a,b"), "\"a,b\"");
        assert_eq!(quoted("a;b"), "\"a;b\"");
        assert_eq!(quoted("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quoted("two\nlines"), "\"two\nlines\"");
        assert_eq!(quoted("cr\r"), "\"cr\r\"");
        assert_eq!(quoted(" lead"), "\" lead\"");
        assert_eq!(quoted("trail "), "\"trail \"");
        assert_eq!(quoted("in side"), "in side");
        assert_eq!(quoted(""), "");
    }

    #[test]
    fn text_that_starts_like_a_formula_is_guarded_and_unguarded() {
        for text in ["=1+1", "+44 call", "-2 days early", "@SUM(A1)", "\t=x", "'=x", "''@x", "=", "'''-"] {
            let g = guard(text.to_owned());
            assert!(g.starts_with('\''), "{text}");
            assert_eq!(unguard(&g), text, "{text}");
        }
        for text in ["Dig", "'Tis the season", "'", "''", "", " =x", "1-2", "a=b", "Ω"] {
            assert_eq!(guard(text.to_owned()), text, "{text}");
            assert_eq!(unguard(text), text, "{text}");
        }
    }
}
