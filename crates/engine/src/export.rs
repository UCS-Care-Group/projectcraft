//! Exporting the open project to other programs' files (`file.export`): a copy, written
//! atomically, that never changes the project, never takes an undo step, and is never written
//! over a `.projectcraft` file (so never over the open project's own).

use std::io::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::interchange::{self, Format};
use crate::{CmdError, Session};

impl Session {
    /// The project as a file in `format`, with the schedule's dates when it has one.
    pub fn export(&self, format: Format) -> Result<String, CmdError> {
        interchange::export(format, &self.project, self.schedule.as_ref().ok()).map_err(|e| CmdError::Io(e.to_string()))
    }

    /// Writes the project to `path` in `format`, atomically: the file there is always the
    /// previous one or this export, never half of either. Refused unless the name has the
    /// format's extension (`.xml`, `.csv`), so an export never lands on a `.projectcraft` project.
    pub fn export_to(&self, path: &Path, format: Format) -> Result<(), CmdError> {
        check_export_path(path, format)?;
        let text = self.export(format)?;
        crate::file::replace_file(path, false, |f| f.write_all(text.as_bytes())).map_err(|e| CmdError::Io(format!("{}: {e}", path.display())))
    }

    /// `file.export`: writes a copy; the project, its undo history and its schedule stay as they are.
    pub(crate) fn export_command(&self, params: &Value) -> Result<(), CmdError> {
        let code = crate::str_param(params, "format")?;
        let format = Format::from_code(code)
            .filter(|f| f.exports())
            .ok_or_else(|| CmdError::Params(format!("ProjectCraft can't export `{}`", code.chars().take(20).collect::<String>())))?;
        let path = crate::str_param(params, "path")?;
        self.export_to(Path::new(path), format)
    }
}

/// Refuses a path whose name doesn't end in the format's extension: an export is a copy for
/// another program, and must never replace a ProjectCraft project.
pub fn check_export_path(path: &Path, format: Format) -> Result<(), CmdError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case(format.extension()) && path.file_stem().is_some_and(|s| !s.is_empty()) {
        return Ok(());
    }
    let shown: String = path.display().to_string().chars().take(200).collect();
    if ext.eq_ignore_ascii_case(crate::file::EXTENSION) {
        return Err(CmdError::Io(format!("{shown}: an export never writes a ProjectCraft project; name it with .{}", format.extension())));
    }
    Err(CmdError::Io(format!("{shown}: a {} file is named with .{}", format.name(), format.extension())))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::interchange::Outcome;
    use crate::model;

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("projectcraft-export-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_sample_exports_and_imports_as_the_same_project() {
        let s = crate::sample();
        let xml = s.export(Format::Mspdi).unwrap();
        assert!(
            xml.starts_with(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Project xmlns=\"http://schemas.microsoft.com/project\">"
            )
        );
        assert_eq!(xml, s.export(Format::Mspdi).unwrap(), "deterministic");
        let back = Session::import(Format::Mspdi, xml.as_bytes()).unwrap();
        // The schema has no field for a material used at a rate: the packing boxes' 2 boxes/h
        // over the 2-day task go as the 32 boxes it uses.
        let mut expected = s.project.clone();
        for a in &mut expected.assignments {
            if let model::Units::Rate { .. } = a.units {
                a.units = model::Units::Fixed(3200);
            }
        }
        assert_eq!(back.project, expected);
        let report = back.import_report().unwrap();
        assert!(report.lines.is_empty(), "{}", report.text());
        assert_eq!(back.export(Format::Mspdi).unwrap(), xml, "export → import → export is byte for byte the same");
        // The computed dates and slack are in the file: Move day (UID 7) is critical.
        assert!(xml.contains("<Start>2026-11-02T08:00:00</Start>"), "{xml}");
        assert!(xml.contains("<TotalSlack>0</TotalSlack>"));
        assert!(!back.import_report().unwrap().lines.iter().any(|l| l.outcome == Outcome::DatesDiffer));
    }

    #[test]
    fn file_export_writes_a_copy_and_changes_nothing() {
        let mut s = crate::sample();
        s.run("task.add", &json!({"name": "Mine"})).unwrap();
        let before = s.project.clone();
        let path = dir().join("Office move.xml");
        s.run("file.export", &json!({"format": "mspdi", "path": path.to_str().unwrap()})).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), s.export(Format::Mspdi).unwrap());
        assert_eq!(s.project, before);
        s.run("edit.undo", &json!({})).unwrap();
        assert!(!s.project.tasks.iter().any(|t| t.name == "Mine"), "the export took no undo step: undo undid the task.add");
        // Written again over the old copy, atomically.
        s.run("file.export", &json!({"format": "mspdi", "path": path.to_str().unwrap()})).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), s.export(Format::Mspdi).unwrap());
    }

    #[test]
    fn an_export_never_writes_a_project_or_a_nameless_file() {
        let s = crate::sample();
        let d = dir();
        let project = d.join("plan.projectcraft");
        s.save_to(&project).unwrap();
        let saved = std::fs::read(&project).unwrap();
        for bad in [project.clone(), d.join("plan"), d.join("plan.csv"), d.join(".xml"), d.join("plan.XML.projectcraft")] {
            let e = s.export_to(&bad, Format::Mspdi).unwrap_err().to_string();
            assert!(e.contains(".xml"), "{e}");
        }
        assert_eq!(std::fs::read(&project).unwrap(), saved, "the project file is untouched");
        assert!(s.export_to(&d.join("PLAN.XML"), Format::Mspdi).is_ok(), "any case");
        for bad in [project.clone(), d.join("plan.xml"), d.join(".csv"), d.join("plan.csv.projectcraft")] {
            let e = s.export_to(&bad, Format::Csv).unwrap_err().to_string();
            assert!(e.contains(".csv"), "{e}");
        }
        assert_eq!(std::fs::read(&project).unwrap(), saved, "the project file is untouched");
        let mut s = s;
        for bad in
            [json!({"format": "csv", "path": d.join("x.xml").to_str().unwrap()}), json!({"format": "xer", "path": "x.xml"}), json!({"path": "x.xml"})]
        {
            assert!(s.run("file.export", &bad).is_err(), "{bad}");
        }
    }

    /// The sample's task sheet as a CSV: ID, Task Name and the shown columns with their headings,
    /// each cell as the sheet shows it; UTF-8 with a byte order mark, CRLF line ends.
    #[test]
    fn the_sample_exports_as_its_task_sheet() {
        let s = crate::sample();
        let csv = s.export(Format::Csv).unwrap();
        assert_eq!(csv, s.export(Format::Csv).unwrap(), "deterministic");
        assert!(csv.starts_with('\u{FEFF}'), "a byte order mark");
        let lines: Vec<&str> = csv.trim_start_matches('\u{FEFF}').split_terminator("\r\n").collect();
        assert_eq!(lines[0], "ID,Task Name,Duration,Work,Start,Finish,Predecessors,Resource Names,Cost,Fixed Cost,Deadline,Type,Effort Driven");
        assert_eq!(lines.len(), s.project.tasks.len() + 1, "a row a task");
        assert!(!csv.replace("\r\n", "").contains(['\r', '\n']), "every line ends CRLF");
        let sheet = model::SheetText::new(&s.project, s.schedule.as_ref().ok());
        for (row, line) in lines.iter().skip(1).enumerate() {
            assert!(line.starts_with(&format!("{},", row + 1)), "{line}");
            assert!(line.contains(&sheet.cell(row, model::Column::Start)), "{line}");
        }
        // Hidden columns are not written; shown ones follow the column chooser's order.
        let mut s = s;
        s.run("view.setColumns", &json!({"columns": ["Resource Names", "Duration"]})).unwrap();
        let csv = s.export(Format::Csv).unwrap();
        assert!(csv.starts_with("\u{FEFF}ID,Task Name,Resource Names,Duration\r\n"), "{csv}");
        assert!(!csv.contains("Fixed Cost") && !csv.contains("2026-"), "{csv}");
    }

    #[test]
    fn file_export_writes_a_csv_copy() {
        let mut s = crate::sample();
        let before = s.project.clone();
        let path = dir().join("Office move.csv");
        s.run("file.export", &json!({"format": "csv", "path": path.to_str().unwrap()})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\xEF\xBB\xBFID,Task Name,"), "UTF-8 with a byte order mark");
        assert_eq!(bytes, s.export(Format::Csv).unwrap().into_bytes());
        assert_eq!(s.project, before);
        // The file opens again as the same task sheet.
        let back = Session::import_file(&path, None).unwrap();
        assert_eq!(back.project.tasks.iter().map(|t| &t.name).collect::<Vec<_>>(), before.tasks.iter().map(|t| &t.name).collect::<Vec<_>>());
    }
}
