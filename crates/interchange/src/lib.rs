//! ProjectCraft interchange: other programs' files in, a new [`Project`] and an [`ImportReport`]
//! out. Pure functions of bytes; no files, no clock, no UI.
//!
//! Microsoft Project XML (MSPDI, Project's "Save as XML") is read from Microsoft's published
//! schema documentation (<https://learn.microsoft.com/office-project/xml-data-interchange/>). A CSV
//! (comma- or semicolon-separated values, UTF-8) is read as a heading row matched to the task
//! sheet's column names, then a task a row.
//! An import always makes a new project; its dates are always worked out by ProjectCraft's
//! scheduler, never taken from the file. Everything in the file that the project can't hold yet is
//! listed in the report as converted, dropped or not yet imported; nothing is kept hidden. The
//! file's own Start and Finish dates are only compared: each task whose dates differ is a line.
//!
//! Every file is hostile: DTDs and entities are refused, the file is capped at
//! [`MAX_IMPORT_BYTES`] and its nesting at [`MAX_DEPTH`], and the model's caps (tasks) and clamps
//! (durations, lags) apply.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod csv;
mod links;
mod mspdi;
mod mspdi_links;
mod xml;

pub use projectcraft_model as model;

use model::Project;

/// Largest file imported: 100 MB.
pub const MAX_IMPORT_BYTES: usize = 100 * 1024 * 1024;
/// Deepest element nesting read. MSPDI itself goes about eight deep.
pub const MAX_DEPTH: usize = 64;

/// A format ProjectCraft imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Microsoft Project XML (MSPDI).
    Mspdi,
    /// Comma- or semicolon-separated values with a `Task Name` column.
    Csv,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::Mspdi, Format::Csv];

    /// Its short name, as the `file.import` command's `format` param takes it.
    pub fn code(self) -> &'static str {
        match self {
            Format::Mspdi => "mspdi",
            Format::Csv => "csv",
        }
    }

    /// The name shown to the owner.
    pub fn name(self) -> &'static str {
        match self {
            Format::Mspdi => "Microsoft Project XML",
            Format::Csv => "CSV",
        }
    }

    pub fn from_code(code: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.code().eq_ignore_ascii_case(code.trim()))
    }
}

/// Why a file couldn't be imported at all. Nothing is imported when one of these is returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the file is larger than {} MB", MAX_IMPORT_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("the file isn't UTF-8 text; save it from Microsoft Project as XML (UTF-8)")]
    NotUtf8,
    #[error("the file has a document type definition (<!DOCTYPE>); ProjectCraft refuses those in imported files")]
    Dtd,
    #[error("the file uses the entity `&{0};`; ProjectCraft refuses entities other than XML's own five")]
    Entity(String),
    #[error("the file nests elements more than {MAX_DEPTH} deep")]
    TooDeep,
    #[error("the file isn't Microsoft Project XML (its root element is `{0}`)")]
    NotMspdi(String),
    #[error("the file holds more than {} tasks", model::MAX_TASKS)]
    TooManyTasks,
    #[error("the file isn't well-formed XML: {0}")]
    Xml(String),
    #[error("the file isn't UTF-8 text; save it as CSV UTF-8")]
    CsvNotUtf8,
    #[error("the file has no `Task Name` column; its first row must name the columns, one of them Task Name")]
    NoTaskName,
}

/// What happened to one thing in the file that ProjectCraft couldn't take over as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    /// Imported, changed to something ProjectCraft holds (said how).
    Converted,
    /// Left out, and ProjectCraft has no plan to hold it soon.
    Dropped,
    /// Left out for now: a later version imports it.
    NotYetImported,
    /// A task whose Start or Finish, as ProjectCraft schedules it, isn't the file's.
    DatesDiffer,
}

impl Outcome {
    /// The words the report shows.
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Converted => "Converted",
            Outcome::Dropped => "Dropped",
            Outcome::NotYetImported => "Not yet imported",
            Outcome::DatesDiffer => "Dates differ",
        }
    }
}

/// One line of the import report: what, where in the file, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLine {
    pub outcome: Outcome,
    /// The feature: "Links", "Calendar exceptions", "Resources".
    pub feature: String,
    /// Where: "Project", "3 resources", "ID 4 Order furniture and 2 more tasks".
    pub place: String,
    /// Why, and what was done instead.
    pub why: String,
}

/// The list shown after an import of everything converted, dropped or not yet imported, and of
/// every task whose dates differ from the file's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub format: Format,
    /// Tasks imported.
    pub tasks: usize,
    pub lines: Vec<ReportLine>,
}

impl ImportReport {
    /// The report as plain text, a line each (for the CLI and the clipboard).
    pub fn text(&self) -> String {
        let mut out = format!("Imported {} tasks from {}.\n", self.tasks, self.format.name());
        if self.lines.is_empty() {
            out.push_str("Everything in the file was imported.\n");
        }
        for l in &self.lines {
            out.push_str(&format!("{}: {} ({}): {}\n", l.outcome.label(), l.feature, l.place, l.why));
        }
        out
    }
}

/// A new project and the report of what happened on the way.
#[derive(Debug, Clone)]
pub struct Import {
    pub project: Project,
    pub report: ImportReport,
}

/// Imports `bytes` as `format` into a new project.
pub fn import(format: Format, bytes: &[u8]) -> Result<Import, ImportError> {
    match format {
        Format::Mspdi => mspdi::import(bytes),
        Format::Csv => csv::import(bytes),
    }
}

/// The format `bytes` are in, judged by their content, if one ProjectCraft imports. A CSV has
/// nothing to tell it by: it is known by its name (`.csv`).
pub fn detect(bytes: &[u8]) -> Option<Format> {
    mspdi::detect(bytes).then_some(Format::Mspdi)
}
