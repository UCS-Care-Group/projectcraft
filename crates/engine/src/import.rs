//! Importing other programs' files (`file.import`): always into a new session, never into the
//! open project, with the import report kept beside it ([`Session::import_report`]).

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde_json::Value;

use crate::interchange::{self, Format, ImportReport, MAX_IMPORT_BYTES};
use crate::{CmdError, Session};

impl Session {
    /// A new session holding `bytes` imported as `format`, scheduled by ProjectCraft, with its report.
    pub fn import(format: Format, bytes: &[u8]) -> Result<Session, CmdError> {
        let i = interchange::import(format, bytes).map_err(|e| CmdError::Io(format!("couldn't import the {}: {e}", format.name())))?;
        // Checked as a `.projectcraft` file is, so nothing an import makes is new to the scheduler.
        let mut project = i.project;
        crate::file::validate(&mut project)?;
        let mut s = Session::new(project);
        s.import_report = Some(i.report);
        Ok(s)
    }

    /// Imports the file at `path` into a new session. Without a `format`, the file's content
    /// says which, else its name: `.xml` is Microsoft Project XML (so its own errors show), `.csv`
    /// is CSV. A CSV has no project name: the file's is used.
    pub fn import_file(path: &Path, format: Option<Format>) -> Result<Session, CmdError> {
        let io = |e: io::Error| CmdError::Io(format!("{}: {e}", path.display()));
        let mut bytes = Vec::new();
        File::open(path).and_then(|f| f.take(MAX_IMPORT_BYTES as u64 + 1).read_to_end(&mut bytes)).map_err(io)?;
        let by_name = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("xml") => Some(Format::Mspdi),
            Some("csv") => Some(Format::Csv),
            _ => None,
        };
        let format = format.or_else(|| interchange::detect(&bytes)).or(by_name).ok_or_else(|| {
            CmdError::Io(format!("{}: not a file ProjectCraft can import (it imports Microsoft Project XML and CSV)", path.display()))
        })?;
        let mut s = Session::import(format, &bytes).map_err(|e| CmdError::Io(format!("{}: {e}", path.display())))?;
        if format == Format::Csv
            && let Some(stem) = path
                .file_stem()
                .map(|n| n.to_string_lossy().chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned())
                .filter(|n| !n.is_empty())
        {
            s.project.name = stem;
        }
        Ok(s)
    }

    /// The report of the import that made this session's project, if one did.
    pub fn import_report(&self) -> Option<&ImportReport> {
        self.import_report.as_ref()
    }

    /// `file.import`: the session becomes the imported file's, a new project with no undo history;
    /// on any error it stays exactly as it was.
    pub(crate) fn import_command(&mut self, params: &Value) -> Result<(), CmdError> {
        let code = crate::str_param(params, "format")?;
        let format =
            Format::from_code(code).ok_or_else(|| CmdError::Params(format!("unknown format `{}`", code.chars().take(20).collect::<String>())))?;
        let path = crate::str_param(params, "path")?;
        *self = Session::import_file(Path::new(path), Some(format))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model;

    const FILE: &str = "<Project xmlns=\"http://schemas.microsoft.com/project\"><Title>Shed</Title><StartDate>2026-11-02T08:00:00</StartDate><Tasks>\
        <Task><UID>0</UID><Name>Shed</Name><OutlineLevel>0</OutlineLevel></Task>\
        <Task><UID>1</UID><ID>1</ID><Name>Base</Name><OutlineLevel>1</OutlineLevel><Duration>PT24H0M0S</Duration></Task>\
        <Task><UID>2</UID><ID>2</ID><Name>Dig</Name><OutlineLevel>2</OutlineLevel><Duration>PT16H0M0S</Duration></Task>\
        <Task><UID>3</UID><ID>3</ID><Name>Pour</Name><OutlineLevel>2</OutlineLevel><Duration>PT8H0M0S</Duration>\
        <PredecessorLink><PredecessorUID>2</PredecessorUID></PredecessorLink></Task>\
        </Tasks></Project>";

    fn tmp(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("projectcraft-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn an_imported_file_is_scheduled_by_projectcraft() {
        let s = Session::import(Format::Mspdi, FILE.as_bytes()).unwrap();
        assert_eq!(s.project.name, "Shed");
        let sched = s.schedule.as_ref().unwrap();
        // Pour follows Dig by its link; the summary rolls both up.
        let dates = |uid| sched.tasks.get(&model::TaskId(uid)).map(|d| (d.start.to_string(), d.finish.to_string())).unwrap();
        assert_eq!(dates(2), ("2026-11-02".into(), "2026-11-03".into()));
        assert_eq!(dates(3), ("2026-11-04".into(), "2026-11-04".into()));
        assert_eq!(dates(1), ("2026-11-02".into(), "2026-11-04".into()));
        assert_eq!(s.project.task(model::TaskId(1)).unwrap().duration, 24 * 60, "the summary's duration is rolled up");
        let r = s.import_report().unwrap();
        assert_eq!(r.tasks, 3);
        assert!(r.lines.iter().any(|l| l.feature == "Project summary task"));
        assert!(!r.lines.iter().any(|l| l.feature.starts_with("Links")), "the link is imported");
        assert!(!s.can_undo());
    }

    #[test]
    fn file_import_replaces_the_session_and_leaves_it_on_error() {
        let xml = tmp("shed.xml", FILE);
        let mut s = crate::sample();
        s.run("task.add", &json!({"name": "Mine"})).unwrap();
        let before = s.project.clone();
        for bad in
            [json!({"format": "mspdi", "path": "/no/such/file.xml"}), json!({"format": "csv", "path": xml.to_str().unwrap()}), json!({"path": "x"})]
        {
            assert!(s.run("file.import", &bad).is_err(), "{bad}");
            assert_eq!(s.project, before);
            assert!(s.can_undo());
        }
        let junk = tmp("junk.xml", "<!DOCTYPE x [<!ENTITY a \"b\">]><Project/>");
        let e = s.run("file.import", &json!({"format": "mspdi", "path": junk.to_str().unwrap()})).unwrap_err();
        assert!(e.to_string().contains("document type definition"), "{e}");
        assert_eq!(s.project, before);
        s.run("file.import", &json!({"format": "mspdi", "path": xml.to_str().unwrap()})).unwrap();
        assert_eq!(s.project.name, "Shed");
        assert!(!s.can_undo(), "an import is a new project, not an edit of the old one");
        assert!(s.import_report().is_some());
    }

    #[test]
    fn a_csv_imports_by_its_name_and_is_named_after_the_file() {
        let csv = tmp("Loft conversion.csv", "\u{feff}Task Name;Duration;Predecessors\r\nPlans;5;\r\nBuild;10;1\r\n");
        let s = Session::import_file(&csv, None).unwrap();
        assert_eq!(s.project.name, "Loft conversion");
        assert_eq!(s.import_report().unwrap().format, Format::Csv);
        let sched = s.schedule.as_ref().unwrap();
        assert_eq!(sched.tasks.get(&model::TaskId(2)).map(|d| d.start.to_string()), Some("1970-01-08".into()), "after Plans' 5 days");
        let mut s = crate::sample();
        s.run("file.import", &json!({"format": "csv", "path": csv.to_str().unwrap()})).unwrap();
        assert_eq!(s.project.tasks.len(), 2);
        // A CSV without a Task Name column says so, and the open project stays.
        let bad = tmp("bad.csv", "Name of task,Duration\nA,1\n");
        let before = s.project.clone();
        let e = s.run("file.import", &json!({"format": "csv", "path": bad.to_str().unwrap()})).unwrap_err();
        assert!(e.to_string().contains("Task Name"), "{e}");
        assert_eq!(s.project, before);
    }

    #[test]
    fn import_file_detects_mspdi_and_refuses_other_files() {
        let xml = tmp("shed-no-ext", FILE);
        assert_eq!(Session::import_file(&xml, None).unwrap().project.tasks.len(), 3);
        let other = tmp("notes.txt", "hello");
        assert!(Session::import_file(&other, None).err().unwrap().to_string().contains("not a file ProjectCraft can import"));
        // A .xml that isn't MSPDI says why.
        let html = tmp("page.xml", "<html/>");
        assert!(Session::import_file(&html, None).err().unwrap().to_string().contains("root element is `html`"));
    }
}
