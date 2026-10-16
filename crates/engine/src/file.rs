//! The `.projectcraft` file: versioned JSON of the [`Project`], saved atomically.
//!
//! The file is the project's fields plus `format_version`, a whole number that goes up only
//! when old readers can't read new files. A newer version is refused; an older one is migrated
//! on load ([`migrate`]). M0 files have no `format_version` and count as version 0.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{self, Project};
use crate::{CmdError, Session};

/// The version this build writes and the newest it reads.
pub const FORMAT_VERSION: u64 = 1;
/// Largest file read, so a hostile file can't exhaust memory. ponytail: about 1,000 bytes a task at `MAX_TASKS`.
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

impl Session {
    /// The project as `.projectcraft` JSON, with its `format_version`.
    pub fn save_json(&self) -> Result<String, CmdError> {
        let mut v = serde_json::to_value(&self.project).map_err(|e| CmdError::Io(e.to_string()))?;
        if let Value::Object(m) = &mut v {
            m.insert("format_version".into(), FORMAT_VERSION.into());
        }
        serde_json::to_string_pretty(&v).map_err(|e| CmdError::Io(e.to_string()))
    }

    /// Reads `.projectcraft` JSON: refuses a newer version, migrates an older one, validates the rest.
    pub fn load_json(text: &str) -> Result<Session, CmdError> {
        let bad = |e: String| CmdError::Io(format!("not a ProjectCraft project: {e}"));
        let mut v: Value = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
        let m = v.as_object_mut().ok_or_else(|| bad("expected a JSON object".into()))?;
        let version = match m.remove("format_version") {
            None => 0,
            Some(n) => n.as_u64().ok_or_else(|| bad("`format_version` is not a whole number".into()))?,
        };
        if version > FORMAT_VERSION {
            return Err(CmdError::Io(format!(
                "this file was saved by a newer ProjectCraft (file format {version}; this version reads up to {FORMAT_VERSION}). Update ProjectCraft to open it."
            )));
        }
        migrate(&mut v, version);
        let mut p: Project = serde_json::from_value(v).map_err(|e| bad(e.to_string()))?;
        validate(&mut p)?;
        Ok(Session::new(p))
    }

    /// Opens a `.projectcraft` file.
    pub fn open(path: &Path) -> Result<Session, CmdError> {
        let io = |e: io::Error| CmdError::Io(format!("{}: {e}", path.display()));
        let mut bytes = Vec::new();
        File::open(path).and_then(|f| f.take(MAX_FILE_BYTES.saturating_add(1)).read_to_end(&mut bytes)).map_err(io)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(CmdError::Io(format!("{}: larger than {} MB", path.display(), MAX_FILE_BYTES / 1024 / 1024)));
        }
        let text = String::from_utf8(bytes).map_err(|_| CmdError::Io(format!("{}: not a ProjectCraft project (not UTF-8 text)", path.display())))?;
        Session::load_json(&text)
    }

    /// Saves atomically: the file at `path` is always the previous save or this one, never half of
    /// either. The previous save is kept as `<path>.bak`.
    pub fn save_to(&self, path: &Path) -> Result<(), CmdError> {
        let text = self.save_json()?;
        replace_file(path, |f| f.write_all(text.as_bytes())).map_err(|e| CmdError::Io(format!("{}: {e}", path.display())))
    }
}

/// Brings a file of an older `version` up to [`FORMAT_VERSION`], one step at a time.
/// Migration code stays forever for every released version.
fn migrate(_v: &mut Value, version: u64) {
    if version == 0 {
        // M0 → 1: same fields; only `format_version` is new.
    }
}

/// Refuses what the editing commands could never have produced; repairs `next_id` so ids are never reused.
fn validate(p: &mut Project) -> Result<(), CmdError> {
    let bad = |e: String| CmdError::Io(format!("not a valid ProjectCraft project: {e}"));
    if p.tasks.len() > model::MAX_TASKS {
        return Err(bad(format!("more than {} tasks", model::MAX_TASKS)));
    }
    let mut ids = HashSet::with_capacity(p.tasks.len());
    for t in &p.tasks {
        if t.id.0 == 0 || !ids.insert(t.id) {
            return Err(bad(format!("task id {} is zero or used twice", t.id)));
        }
        model::check_days(t.duration).map_err(|e| bad(format!("task {}: {e}", t.id)))?;
    }
    for t in &p.tasks {
        for l in &t.preds {
            if !ids.contains(&l.pred) {
                return Err(bad(format!("task {} links to missing task {}", t.id, l.pred)));
            }
            model::check_days(l.lag.saturating_abs()).map_err(|e| bad(format!("task {}: lag {e}", t.id)))?;
        }
    }
    let max = p.tasks.iter().map(|t| t.id.0).max().unwrap_or(0);
    p.next_id = p.next_id.max(max.saturating_add(1)).max(1);
    Ok(())
}

/// Writes a temp file beside `path` with `write`, fsyncs it, copies the old file to `.bak`, then
/// renames the temp file over `path`. On any error `path` is untouched and the temp file removed.
fn replace_file(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let tmp = sibling(path, ".tmp");
    let result = (|| {
        let mut f = File::create(&tmp)?;
        write(&mut f)?;
        f.sync_all()?;
        drop(f);
        if path.exists() {
            fs::copy(path, sibling(path, ".bak"))?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    // Make the rename itself durable. Directories can't be opened on every platform; best effort.
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

/// `plan.projectcraft` → `plan.projectcraft<suffix>`, in the same folder.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Constraint, Date, Link, LinkKind, Task, TaskId};
    use serde_json::json;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("projectcraft-file-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A project with every field set to something other than its default.
    fn full() -> Project {
        let mut p = Project::new("Every field", Date::from_ymd(2026, 3, 2).unwrap());
        p.calendar.working = [true, false, true, false, true, true, false];
        p.calendar.holidays.insert(Date::from_ymd(2026, 4, 3).unwrap());
        p.tasks = vec![
            Task { id: TaskId(2), name: "Dig «foundations» ✓".into(), duration: 4, preds: vec![], constraint: Constraint::Asap },
            Task {
                id: TaskId(5),
                name: "Pour".into(),
                duration: 0,
                preds: vec![Link { pred: TaskId(2), kind: LinkKind::SF, lag: -3 }],
                constraint: Constraint::StartNoEarlierThan(Date::from_ymd(2026, 3, 9).unwrap()),
            },
        ];
        p.next_id = 9;
        p
    }

    #[test]
    fn every_field_round_trips_with_a_format_version() {
        let s = Session::new(full());
        let text = s.save_json().unwrap();
        let v = serde_json::from_str::<Value>(&text).unwrap();
        assert_eq!(v["format_version"], json!(FORMAT_VERSION));
        assert_eq!(v["tasks"][1]["constraint"], json!({"SNET": "2026-03-09"}));
        assert!(v["tasks"][0].get("constraint").is_none(), "ASAP is not written");
        assert_eq!(Session::load_json(&text).unwrap().project, full());
    }

    #[test]
    fn a_newer_version_is_refused_clearly() {
        let mut v = serde_json::from_str::<Value>(&Session::new(full()).save_json().unwrap()).unwrap();
        v["format_version"] = json!(FORMAT_VERSION + 1);
        let e = Session::load_json(&v.to_string()).err().unwrap().to_string();
        assert!(e.contains("newer ProjectCraft"), "{e}");
        v["format_version"] = json!("1");
        assert!(Session::load_json(&v.to_string()).is_err());
    }

    #[test]
    fn an_m0_file_loads_through_migration() {
        // As M0 wrote it: no format_version.
        let m0 = r#"{"name":"Old","start":"2026-01-05","calendar":{"working":[true,true,true,true,true,false,false],"holidays":[]},
            "tasks":[{"id":1,"name":"A","duration":2,"preds":[]},{"id":2,"name":"B","duration":1,"preds":[{"pred":1,"kind":"FS","lag":0}]}],"next_id":3}"#;
        let s = Session::load_json(m0).unwrap();
        assert_eq!(s.project.tasks.len(), 2);
        // No constraint field: As Soon As Possible, and none is written back.
        assert!(s.project.tasks.iter().all(|t| t.constraint == Constraint::Asap));
        assert!(!s.save_json().unwrap().contains("constraint"));
        assert!(s.schedule.is_ok());
    }

    #[test]
    fn validation_refuses_what_commands_never_make_and_repairs_next_id() {
        let mut p = full();
        p.next_id = 1;
        let s = Session::load_json(&Session::new(p).save_json().unwrap()).unwrap();
        assert_eq!(s.project.next_id, 6, "a deleted or existing id is never reused");
        for broken in [
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1}, {"id": 1, "name": "b", "duration": 1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 0, "name": "a", "duration": 1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": -1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "preds": [{"pred": 7}]}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1}, {"id": 2, "name": "b", "duration": 1, "preds": [{"pred": 1, "lag": i64::MIN}]}]}),
            json!([1, 2]),
        ] {
            assert!(Session::load_json(&broken.to_string()).is_err(), "{broken}");
        }
        // A loop of links loads; the scheduler reports it.
        let looped = json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "preds": [{"pred": 2}]}, {"id": 2, "name": "b", "duration": 1, "preds": [{"pred": 1}]}]});
        assert!(Session::load_json(&looped.to_string()).unwrap().schedule.is_err());
    }

    #[test]
    fn save_is_atomic_and_keeps_the_previous_save_as_bak() {
        let dir = tmp_dir("save");
        let path = dir.join("plan.projectcraft");
        let mut s = Session::new(full());
        s.save_to(&path).unwrap();
        assert!(!sibling(&path, ".bak").exists());
        s.run("task.add", &json!({"name": "Snag list", "duration": 3})).unwrap();
        s.save_to(&path).unwrap();
        assert_eq!(Session::open(&path).unwrap().project, s.project);
        assert_eq!(Session::open(&sibling(&path, ".bak")).unwrap().project, full());
        assert!(!sibling(&path, ".tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_interrupted_save_leaves_the_previous_file_intact() {
        let dir = tmp_dir("interrupted");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let before = fs::read(&path).unwrap();

        // The writer fails halfway through.
        let failed = replace_file(&path, |f| {
            f.write_all(b"{\"name\": \"half")?;
            Err(io::Error::other("disk full"))
        });
        assert!(failed.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!sibling(&path, ".tmp").exists());

        // The process died after writing the temp file, before the rename.
        fs::write(sibling(&path, ".tmp"), b"{\"name\": \"half").unwrap();
        assert_eq!(Session::open(&path).unwrap().project, full());
        Session::new(full()).save_to(&path).unwrap();
        assert_eq!(Session::open(&path).unwrap().project, full());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn opening_a_missing_or_binary_file_is_an_error() {
        let dir = tmp_dir("open");
        assert!(Session::open(&dir.join("missing.projectcraft")).is_err());
        fs::write(dir.join("bin.projectcraft"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(Session::open(&dir.join("bin.projectcraft")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    proptest::proptest! {
        #[test]
        fn load_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            let _ = Session::load_json(&String::from_utf8_lossy(&bytes));
        }

        #[test]
        fn load_never_panics_on_mutated_files(cut in 0usize..2000, junk in "\\PC{0,8}") {
            let text = Session::new(full()).save_json().unwrap();
            let at = text.char_indices().map(|(i, _)| i).nth(cut).unwrap_or(text.len());
            let _ = Session::load_json(&format!("{}{junk}{}", &text[..at], &text[at..]));
        }
    }
}
