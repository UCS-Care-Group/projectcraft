//! The `.projectcraft` file: versioned JSON of the [`Project`], saved atomically.
//!
//! The file is the project's fields plus `format_version`, a whole number that goes up only
//! when old readers can't read new files. A newer version is refused; an older one is migrated
//! on load ([`migrate`]). M0 files have no `format_version` and count as version 0. Version 2
//! counts durations and lags in working minutes; version 1 counted whole working days.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{self, Project};
use crate::{CmdError, Session};

/// The extension of a ProjectCraft project file.
pub(crate) const EXTENSION: &str = "projectcraft";
/// The version this build writes and the newest it reads.
pub const FORMAT_VERSION: u64 = 2;
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
        migrate(&mut v, version)?;
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
        replace_file(path, true, |f| f.write_all(text.as_bytes())).map_err(|e| CmdError::Io(format!("{}: {e}", path.display())))
    }

    /// Saves a copy atomically, as [`Session::save_to`] but without a `.bak` (for the recovery copy).
    pub fn save_copy_to(&self, path: &Path) -> Result<(), CmdError> {
        let text = self.save_json()?;
        replace_file(path, false, |f| f.write_all(text.as_bytes())).map_err(|e| CmdError::Io(format!("{}: {e}", path.display())))
    }
}

/// Brings a file of an older `version` up to [`FORMAT_VERSION`], one step at a time.
/// Migration code stays forever for every released version.
///
/// A new field with a default (`constraint`, `outline_level`, `resources`) needs no new version: older files
/// read as the default, and the outline is kept to its rule on load ([`Session::new`]).
fn migrate(v: &mut Value, version: u64) -> Result<(), CmdError> {
    if version == 0 {
        // M0 → 1: same fields; only `format_version` is new.
    }
    // Calendar `holidays` (M0 and early version 1) became `exceptions` without a version bump:
    // the calendar reads both (`model::Calendar`), and writes only `exceptions`.
    // Calendar working times and project `options` arrived the same way: a file without them
    // reads as the default hours and the default options.
    if version < 2 {
        days_to_minutes(v)?;
    }
    Ok(())
}

/// Version 1 → 2: durations and lags were whole working days; they become working minutes, at
/// the file's own hours per day (the default 8 when it has none, or none in range). A day count
/// past [`model::MAX_DAYS`] is refused, as version 1 refused it. Anything that isn't a whole
/// number is left for loading to refuse.
fn days_to_minutes(v: &mut Value) -> Result<(), CmdError> {
    let day = v
        .pointer("/options/minutes_per_day")
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok())
        .map_or_else(|| model::Options::default().day(), |m| model::Options { minutes_per_day: m, ..model::Options::default() }.day());
    let convert = |n: &mut Value, what: &dyn Fn() -> String| -> Result<(), CmdError> {
        let Some(days) = n.as_i64() else { return Ok(()) };
        model::check_days(days.saturating_abs()).map_err(|e| CmdError::Io(format!("not a valid ProjectCraft project: {}: {e}", what())))?;
        *n = Value::from(days * day);
        Ok(())
    };
    let Some(tasks) = v.get_mut("tasks").and_then(Value::as_array_mut) else { return Ok(()) };
    for t in tasks {
        let id = t.get("id").map_or_else(String::new, Value::to_string);
        if let Some(d) = t.get_mut("duration") {
            convert(d, &|| format!("task {id}"))?;
        }
        for l in t.get_mut("preds").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(lag) = l.get_mut("lag") {
                convert(lag, &|| format!("task {id}: lag"))?;
            }
        }
    }
    Ok(())
}

/// Refuses what the editing commands could never have produced; repairs `next_id` so ids are never reused.
pub(crate) fn validate(p: &mut Project) -> Result<(), CmdError> {
    let bad = |e: String| CmdError::Io(format!("not a valid ProjectCraft project: {e}"));
    if p.tasks.len() > model::MAX_TASKS {
        return Err(bad(format!("more than {} tasks", model::MAX_TASKS)));
    }
    p.options.check().map_err(|e| bad(format!("project options: {e}")))?;
    p.currency.check().map_err(|e| bad(format!("currency: {e}")))?;
    if p.calendar.exceptions.len() > model::MAX_EXCEPTIONS {
        return Err(bad(format!("more than {} calendar exceptions", model::MAX_EXCEPTIONS)));
    }
    let mut ids = HashSet::with_capacity(p.tasks.len());
    for t in &p.tasks {
        if t.id.0 == 0 || !ids.insert(t.id) {
            return Err(bad(format!("task id {} is zero or used twice", t.id)));
        }
        model::check_minutes(t.duration).map_err(|e| bad(format!("task {}: {e}", t.id)))?;
        model::check_money(t.fixed_cost).map_err(|e| bad(format!("task {}: fixed cost: {e}", t.id)))?;
        model::check_priority(i64::from(t.priority)).map_err(|e| bad(format!("task {}: {e}", t.id)))?;
        model::check_minutes(t.leveling_delay).map_err(|e| bad(format!("task {}: leveling delay: {e}", t.id)))?;
    }
    for t in &p.tasks {
        for l in &t.preds {
            if !ids.contains(&l.pred) {
                return Err(bad(format!("task {} links to missing task {}", t.id, l.pred)));
            }
            model::check_minutes(l.lag.saturating_abs()).map_err(|e| bad(format!("task {}: lag {e}", t.id)))?;
        }
    }
    let max = p.tasks.iter().map(|t| t.id.0).max().unwrap_or(0);
    p.next_id = p.next_id.max(max.saturating_add(1)).max(1);
    // Assignments from before task types: their work and duration from the task's duration.
    p.fill_assignment_work();
    // Resources: count, Unique IDs, names and every field in range; `next_resource_id` repaired.
    p.check_resources().map_err(|e| bad(e.to_string()))
}

/// Saves `path` through craft-atomic: `write` fills an owner-only staging file beside it, which
/// is flushed and renamed over `path`. When `bak`, the previous file is copied to `<path>.bak`
/// once the new content is written, before it replaces the old. On any error `path` is untouched.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn replace_file(path: &Path, bak: bool, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    replace_file_with(&craft_atomic::RealFs, craft_atomic::RenameRetry::platform(), path, bak, write)
}

/// The web build has no file system to save to.
#[cfg(target_arch = "wasm32")]
pub(crate) fn replace_file(_: &Path, _: bool, _: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    Err(no_files())
}

/// [`replace_file`] through `fs`, so a test can make the disk fail at any step.
#[cfg(not(target_arch = "wasm32"))]
fn replace_file_with(
    fs: &dyn craft_atomic::AtomicFs,
    retry: craft_atomic::RenameRetry,
    path: &Path,
    bak: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    owner_only(path);
    private().atomic_write_with_custom(fs, retry, path, |f| {
        write(f)?;
        if bak && path.exists() {
            // Staged owner-only from its first byte, so the `.bak` is never readable by others.
            let bak = sibling(path, ".bak");
            owner_only(&bak);
            private().atomic_write_with(&bak, |to| io::copy(&mut File::open(path)?, to).map(drop))?;
        }
        Ok(())
    })
}

/// New files staged and created owner-only (0600 on Unix), never wider for a moment.
#[cfg(not(target_arch = "wasm32"))]
fn private() -> craft_atomic::WriteOptions {
    craft_atomic::WriteOptions::default().with_new_file_permissions(craft_atomic::NewFilePermissions::Private)
}

/// craft-atomic keeps the mode of a file it replaces, so an existing file wider than owner-only
/// (an old `.bak`, a plan another program saved) is narrowed to 0600 first. Best effort: a file
/// another account owns can't be narrowed, and keeps its mode.
#[cfg(not(target_arch = "wasm32"))]
fn owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = fs::metadata(path)
            && m.is_file()
            && m.permissions().mode() & 0o077 != 0
        {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A save's staging file older than this is left over from a save that never finished: a save
/// takes seconds, so no save still running staged its file an hour ago. Age, not "is its
/// process running", because std can't ask that on every platform and process ids are reused.
#[cfg(not(target_arch = "wasm32"))]
const STALE_TEMP_SECS: u64 = 60 * 60;
/// Most folder entries looked at in one sweep, so a huge folder can't stall an open or a save.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SWEEP_ENTRIES: usize = 10_000;
/// craft-atomic names a staging file `.<first 64 chars of the name>.<pid>-<n>.craft_tmp`.
#[cfg(not(target_arch = "wasm32"))]
const TEMP_STEM_CHARS: usize = 64;

/// What a [`sweep_stale_temps`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempSweep {
    /// Staging files removed.
    pub removed: usize,
    /// Something couldn't be looked at or removed; whatever it was is left where it is.
    pub failed: bool,
}

/// Removes the staging files that saves of `path` (and of its `.bak`) left beside it when the
/// program was killed mid-save and that are older than an hour. Only regular files named exactly
/// as craft-atomic names that file's staging files are touched; a symbolic link is never followed
/// or removed. Best effort, never an error: a failure leaves the file where it is and is reported
/// in [`TempSweep::failed`]. Call it after opening or saving `path`.
#[cfg(not(target_arch = "wasm32"))]
pub fn sweep_stale_temps(path: &Path) -> TempSweep {
    sweep_stale_temps_older_than(path, std::time::Duration::from_secs(STALE_TEMP_SECS))
}

/// The web build has no files to sweep.
#[cfg(target_arch = "wasm32")]
pub fn sweep_stale_temps(_: &Path) -> TempSweep {
    TempSweep::default()
}

/// [`sweep_stale_temps`] with the age a staging file must reach to be removed.
#[cfg(not(target_arch = "wasm32"))]
fn sweep_stale_temps_older_than(path: &Path, age: std::time::Duration) -> TempSweep {
    let mut out = TempSweep::default();
    // craft-atomic stages beside the file a symbolic link points to, so look there.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(name) = target.file_name().and_then(|n| n.to_str()) else { return out };
    let bak = format!("{name}.bak");
    // A longer name is cut short in the staging name, where another file's could match it.
    if bak.chars().count() > TEMP_STEM_CHARS {
        return out;
    }
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            out.failed = e.kind() != io::ErrorKind::NotFound;
            return out;
        }
    };
    let now = std::time::SystemTime::now();
    for entry in entries.take(MAX_SWEEP_ENTRIES) {
        let Ok(entry) = entry else {
            out.failed = true;
            continue;
        };
        let file_name = entry.file_name();
        let Some(entry_name) = file_name.to_str() else { continue };
        if !is_temp_of(entry_name, name) && !is_temp_of(entry_name, &bak) {
            continue;
        }
        // `file_type` and `symlink_metadata` describe the entry itself, never a link's target.
        let Ok(meta) = fs::symlink_metadata(entry.path()) else { continue };
        if !entry.file_type().is_ok_and(|t| t.is_file()) || !meta.is_file() {
            continue;
        }
        // A modified time in the future (the clock moved back) counts as fresh.
        let stale = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|d| d >= age);
        if !stale {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => out.removed = out.removed.saturating_add(1),
            // Another sweep got there first.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => out.failed = true,
        }
    }
    out
}

/// Whether `entry` is a craft-atomic staging file for a file named `name`: `.<name>.<pid>-<n>.craft_tmp`.
#[cfg(not(target_arch = "wasm32"))]
fn is_temp_of(entry: &str, name: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
    entry
        .strip_prefix('.')
        .and_then(|e| e.strip_prefix(name))
        .and_then(|e| e.strip_prefix('.'))
        .and_then(|e| e.strip_suffix(".craft_tmp"))
        .and_then(|e| e.split_once('-'))
        .is_some_and(|(pid, n)| digits(pid) && digits(n))
}

#[cfg(target_arch = "wasm32")]
fn no_files() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "the web build saves no files")
}

/// Writes `bytes` to `path` atomically, owner-only (0600) on Unix. For small app files such as settings.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    owner_only(path);
    private().atomic_write(path, bytes)
}

/// The web build has no file system to write to.
#[cfg(target_arch = "wasm32")]
pub fn write_private(_: &Path, _: &[u8]) -> io::Result<()> {
    Err(no_files())
}

/// Creates `dir` and any missing parents; each folder it creates is owner-only (0700) on Unix.
/// Folders that already exist are left as they are.
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
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
    use crate::model::{AccrueAt, Constraint, Date, Link, LinkKind, Resource, ResourceId, ResourceType, Task, TaskId};
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
        p.calendar.exceptions.insert(Date::from_ymd(2026, 4, 3).unwrap(), false);
        p.calendar.exceptions.insert(Date::from_ymd(2026, 4, 4).unwrap(), true);
        p.calendar.set_weekday_times(4, vec![model::parse_working_time("7:30-15:30").unwrap()]).unwrap();
        p.calendar.set_exception(Date::from_ymd(2026, 4, 11).unwrap(), vec![model::parse_working_time("9:00-13:00").unwrap()]).unwrap();
        p.options = model::Options {
            minutes_per_day: 450,
            minutes_per_week: 2250,
            days_per_month: 21,
            task_type: model::TaskType::FixedDuration,
            effort_driven: true,
            units_format: model::UnitsFormat::Decimal,
        };
        p.tasks = vec![
            Task {
                id: TaskId(2),
                name: "Dig «foundations» ✓".into(),
                duration: 4,
                duration_unit: model::DurationUnit::Hours,
                preds: vec![],
                constraint: Constraint::Asap,
                outline_level: 1,
                deadline: None,
                fixed_cost: 0,
                task_type: model::TaskType::FixedUnits,
                effort_driven: false,
                priority: 500,
                leveling_delay: 0,
            },
            // A subtask of task 2, which is a summary storing its rolled-up duration.
            Task {
                id: TaskId(7),
                name: "Excavate".into(),
                duration: 4,
                duration_unit: model::DurationUnit::Days,
                preds: vec![],
                constraint: Constraint::Asap,
                outline_level: 2,
                deadline: None,
                fixed_cost: 123_456,
                task_type: model::TaskType::FixedWork,
                effort_driven: true,
                priority: 750,
                leveling_delay: 960,
            },
            Task {
                id: TaskId(5),
                name: "Pour".into(),
                duration: 0,
                duration_unit: model::DurationUnit::Days,
                preds: vec![Link { pred: TaskId(2), kind: LinkKind::SF, lag: -3, lag_unit: model::DurationUnit::Minutes }],
                constraint: Constraint::StartNoEarlierThan(Date::from_ymd(2026, 3, 9).unwrap()),
                outline_level: 1,
                deadline: Date::from_ymd(2026, 3, 20),
                fixed_cost: 0,
                task_type: model::TaskType::FixedDuration,
                effort_driven: true,
                priority: 500,
                leveling_delay: 0,
            },
        ];
        p.next_id = 9;
        p.resources = vec![
            Resource {
                id: ResourceId(3),
                name: "Zoë «Site» Lead".into(),
                initials: "ZL".into(),
                group: "Site".into(),
                kind: ResourceType::Work,
                max_units: 5_050,
                material_label: String::new(),
                standard_rate: 4_550,
                overtime_rate: 6_000,
                cost_per_use: 1_000,
                accrue_at: AccrueAt::Start,
                calendar: model::ResourceCalendar {
                    week: [None, None, None, None, Some(vec![]), None, None],
                    exceptions: [(Date::from_ymd(2026, 3, 10).unwrap(), vec![])].into(),
                },
            },
            Resource {
                material_label: "tons".into(),
                accrue_at: AccrueAt::End,
                ..Resource::new(ResourceId(1), "Concrete".into(), ResourceType::Material)
            },
            Resource::new(ResourceId(4), "Travel".into(), ResourceType::Cost),
        ];
        p.next_resource_id = 6;
        p.currency = model::Currency::new("BHD", Some("BD"), None).unwrap();
        p.assignments = vec![model::Assignment { task: TaskId(7), resource: ResourceId(3), units: model::Units::Work(5_050), work: 2, duration: 4 }];
        p
    }

    #[test]
    fn every_field_round_trips_with_a_format_version() {
        let s = Session::new(full());
        let text = s.save_json().unwrap();
        let v = serde_json::from_str::<Value>(&text).unwrap();
        assert_eq!(v["format_version"], json!(FORMAT_VERSION));
        assert_eq!(v["tasks"][2]["constraint"], json!({"SNET": "2026-03-09"}));
        assert!(v["tasks"][0].get("constraint").is_none(), "ASAP is not written");
        assert_eq!(v["tasks"][1]["outline_level"], json!(2));
        assert!(v["tasks"][0].get("outline_level").is_none(), "level 1 is not written");
        assert_eq!(v["resources"][0]["max_units"], json!(5_050));
        assert_eq!(v["resources"][1]["type"], json!("Material"));
        assert!(v["resources"][2].get("max_units").is_none(), "100% is not written");
        assert_eq!(v["next_resource_id"], json!(6));
        assert_eq!(Session::load_json(&text).unwrap().project, full());
    }

    #[test]
    fn resources_are_validated_on_load_and_unique_ids_never_reused() {
        let mut p = full();
        p.next_resource_id = 0;
        let s = Session::load_json(&Session::new(p).save_json().unwrap()).unwrap();
        assert_eq!(s.project.next_resource_id, 5, "an existing Unique ID is never handed out again");
        // A file from before resources has none and writes none.
        let old = json!({"name": "x", "start": "2026-01-05", "tasks": [], "next_id": 1, "format_version": 1});
        let s = Session::load_json(&old.to_string()).unwrap();
        assert!(s.project.resources.is_empty());
        assert!(!s.save_json().unwrap().contains("resource"));
        let r = |extra: Value| {
            let mut one = json!({"id": 1, "name": "Ann"});
            if let (Some(o), Some(e)) = (one.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            json!({"name": "x", "start": "2026-01-05", "resources": [one]}).to_string()
        };
        assert!(Session::load_json(&r(json!({}))).is_ok());
        for broken in [
            json!({"id": 0}),
            json!({"name": "  "}),
            json!({"name": "A[1]"}),
            json!({"max_units": model::MAX_UNITS + 1}),
            json!({"max_units": -1}),
            json!({"standard_rate": -5}),
            json!({"cost_per_use": model::MAX_MONEY + 1}),
            json!({"type": "Person"}),
            json!({"accrue_at": "Never"}),
            json!({"initials": "x".repeat(300)}),
        ] {
            assert!(Session::load_json(&r(broken.clone())).is_err(), "{broken}");
        }
        let twice = json!({"name": "x", "start": "2026-01-05", "resources": [{"id": 2, "name": "A"}, {"id": 2, "name": "B"}]});
        assert!(Session::load_json(&twice.to_string()).is_err());
        let same_name = json!({"name": "x", "start": "2026-01-05", "resources": [{"id": 1, "name": "Ann"}, {"id": 2, "name": "ANN"}]});
        assert!(Session::load_json(&same_name.to_string()).is_err());
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

    /// A version-1 file counted whole working days. It opens with the same dates, its durations
    /// and lags in working minutes at its own hours per day, and saves as version 2 that reads
    /// back the same.
    #[test]
    fn a_version_1_file_migrates_to_minutes_with_the_same_dates() {
        let v1 = |options: Value| {
            json!({"name": "Old", "start": "2026-01-05", "format_version": 1, "options": options,
                "tasks": [{"id": 1, "name": "A", "duration": 3}, {"id": 2, "name": "B", "duration": 2, "preds": [{"pred": 1, "kind": "FS", "lag": 2}]},
                    {"id": 3, "name": "M", "duration": 0, "preds": [{"pred": 2, "kind": "SS", "lag": -1}]}], "next_id": 4})
            .to_string()
        };
        let d = |s: &str| s.parse::<Date>().unwrap();
        for (options, day) in [(json!({}), 480), (json!({"minutes_per_day": 450}), 450), (json!({"minutes_per_day": 1440}), 1440)] {
            let s = Session::load_json(&v1(options.clone())).unwrap();
            let t = &s.project.tasks;
            assert_eq!((t[0].duration, t[1].duration, t[2].duration), (3 * day, 2 * day, 0), "{options}");
            assert_eq!((t[1].preds[0].lag, t[2].preds[0].lag), (2 * day, -day));
            assert_eq!(s.project.preds_text_rows(&t[1].preds), "1FS+2", "shown as days, as before");
            let sch = s.schedule.as_ref().unwrap();
            if day == 480 {
                // The calendar's 8-hour days: A Mon 5 – Wed 7; B after two days' lag, Mon 12 – Tue
                // 13; M one day before B starts, Fri 9.
                let shown = |id: u32| (sch.tasks[&TaskId(id)].start, sch.tasks[&TaskId(id)].finish);
                assert_eq!(shown(1), (d("2026-01-05"), d("2026-01-07")));
                assert_eq!(shown(2), (d("2026-01-12"), d("2026-01-13")));
                assert_eq!(shown(3), (d("2026-01-09"), d("2026-01-09")));
            }
            let text = s.save_json().unwrap();
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["format_version"], json!(2));
            let again = Session::load_json(&text).unwrap();
            assert_eq!(again.project, s.project, "version 2 is not migrated again");
            assert_eq!(again.schedule.as_ref().unwrap(), sch);
        }
        // A day count version 1 refused is still refused, not let through as minutes.
        let mut big = serde_json::from_str::<Value>(&v1(json!({}))).unwrap();
        big["tasks"][0]["duration"] = json!(model::MAX_DAYS + 1);
        assert!(Session::load_json(&big.to_string()).err().unwrap().to_string().contains("task 1"));
        big["tasks"][0]["duration"] = json!(model::MAX_DAYS);
        assert_eq!(Session::load_json(&big.to_string()).unwrap().project.tasks[0].duration, model::MAX_DAYS * 480);
        big["tasks"][1]["preds"][0]["lag"] = json!(i64::MIN);
        assert!(Session::load_json(&big.to_string()).is_err());
        // A version-2 file's values are minutes already.
        let v2 = json!({"name": "New", "start": "2026-01-05", "format_version": 2, "tasks": [{"id": 1, "name": "A", "duration": 240}], "next_id": 2});
        let s = Session::load_json(&v2.to_string()).unwrap();
        assert_eq!(s.project.tasks[0].duration, 240);
        assert_eq!(s.schedule.as_ref().unwrap().tasks[&TaskId(1)].finish_at.minute, 12 * 60, "half a day: Monday 8:00–12:00");
    }

    #[test]
    fn calendar_exceptions_are_written_as_such_and_old_holidays_load_as_non_working() {
        let text = Session::new(full()).save_json().unwrap();
        let v = serde_json::from_str::<Value>(&text).unwrap();
        assert_eq!(
            v["calendar"]["exceptions"],
            json!([{"date": "2026-04-03", "working": false}, {"date": "2026-04-04", "working": true}, {"date": "2026-04-11", "working": true, "times": [{"from": 540, "to": 780}]}])
        );
        assert!(v["calendar"].get("holidays").is_none());
        // M0 and early version 1 files wrote non-working days as `holidays`.
        for version in [json!(null), json!(1)] {
            let mut old = json!({"name": "Old", "start": "2026-01-05", "calendar": {"working": [true, true, true, true, true, false, false], "holidays": ["2026-01-07"]},
                "tasks": [{"id": 1, "name": "A", "duration": 3}], "next_id": 2});
            if !version.is_null() {
                old["format_version"] = version;
            }
            let s = Session::load_json(&old.to_string()).unwrap();
            assert_eq!(s.project.calendar.exceptions.get(&Date::from_ymd(2026, 1, 7).unwrap()), Some(&false));
            // Mon 5, Tue 6, (Wed 7 off), Thu 8.
            assert_eq!(s.schedule.as_ref().unwrap().tasks[&TaskId(1)].finish, Date::from_ymd(2026, 1, 8).unwrap());
            assert!(!s.save_json().unwrap().contains("holidays"));
        }
        // Too many exceptions, or a bad exception date, is refused.
        let many: Vec<Value> = (0..=model::MAX_EXCEPTIONS as i64)
            .map(|n| json!({"date": Date::from_ymd(2000, 1, 1).unwrap().add_days(n).unwrap().to_string(), "working": true}))
            .collect();
        let big =
            json!({"name": "x", "start": "2026-01-05", "calendar": {"working": [true, true, true, true, true, true, true], "exceptions": many}});
        assert!(Session::load_json(&big.to_string()).is_err());
        let junk = json!({"name": "x", "start": "2026-01-05", "calendar": {"working": [true, true, true, true, true, true, true], "exceptions": [{"date": "2026-02-30"}]}});
        assert!(Session::load_json(&junk.to_string()).is_err());
    }

    #[test]
    fn working_times_and_options_are_additive_and_bad_ones_are_refused() {
        let v = serde_json::from_str::<Value>(&Session::new(full()).save_json().unwrap()).unwrap();
        assert_eq!(v["calendar"]["times"][0], json!([{"from": 480, "to": 720}, {"from": 780, "to": 1020}]));
        assert_eq!(v["calendar"]["times"][1], json!([]));
        assert_eq!(v["calendar"]["times"][4], json!([{"from": 450, "to": 930}]));
        assert_eq!(
            v["options"],
            json!({"minutes_per_day": 450, "minutes_per_week": 2250, "days_per_month": 21, "task_type": "FixedDuration", "effort_driven": true, "units_format": "Decimal"})
        );
        // The task-type options are written only when they differ from Project's defaults.
        let plain = serde_json::from_str::<Value>(&Session::new(Project::new("x", Date::EPOCH)).save_json().unwrap()).unwrap();
        assert_eq!(plain["options"], json!({"minutes_per_day": 480, "minutes_per_week": 2400, "days_per_month": 20}));
        // A file from before working times: the default hours on every working weekday, default options.
        let old = json!({"name": "Old", "start": "2026-01-05", "format_version": 1, "calendar": {"working": [true, true, true, true, true, false, false],
            "exceptions": [{"date": "2026-01-10", "working": true}]}, "tasks": [{"id": 1, "name": "A", "duration": 3}], "next_id": 2});
        let s = Session::load_json(&old.to_string()).unwrap();
        let c = &s.project.calendar;
        assert_eq!(c.weekday_times(0), &model::DEFAULT_WORKING_TIMES);
        assert!(c.weekday_times(5).is_empty());
        assert_eq!(c.times_on(Date::from_ymd(2026, 1, 10).unwrap()), &model::DEFAULT_WORKING_TIMES);
        assert_eq!(s.project.options, model::Options::default());
        let partial = json!({"name": "x", "start": "2026-01-05", "options": {"minutes_per_day": 420}});
        assert_eq!(Session::load_json(&partial.to_string()).unwrap().project.options.minutes_per_week, 2400);
        // Times win over the flags: a week whose only times are Saturday's works Saturday only.
        let week = |day: Value| json!({"name": "x", "start": "2026-01-05", "calendar": {"working": [true, true, true, true, true, true, true], "times": [[], [], [], [], [], day, []]}});
        let sat = Session::load_json(&week(json!([{"from": 540, "to": 780}])).to_string()).unwrap();
        assert_eq!(sat.project.calendar.working, [false, false, false, false, false, true, false]);
        // Hostile times and options are refused, never trusted.
        for bad in [
            json!([{"from": 720, "to": 480}]),
            json!([{"from": 0, "to": 1441}]),
            json!([{"from": 480, "to": 780}, {"from": 720, "to": 900}]),
            json!([{"from": -1, "to": 5}]),
            json!([{"from": 70000, "to": 5}]),
            Value::Array(vec![json!({"from": 0, "to": 1}); 11]),
        ] {
            assert!(Session::load_json(&week(bad.clone()).to_string()).is_err(), "{bad}");
        }
        let off_with_times = json!({"name": "x", "start": "2026-01-05", "calendar": {"working": [true, true, true, true, true, true, true],
            "exceptions": [{"date": "2026-01-12", "working": false, "times": [{"from": 540, "to": 780}]}]}});
        assert!(Session::load_json(&off_with_times.to_string()).is_err());
        for bad in [json!({"minutes_per_day": 0}), json!({"minutes_per_week": 999_999}), json!({"days_per_month": 32}), json!({"days_per_month": -1})]
        {
            let p = json!({"name": "x", "start": "2026-01-05", "options": bad});
            assert!(Session::load_json(&p.to_string()).is_err(), "{p}");
        }
    }

    #[test]
    fn validation_refuses_what_commands_never_make_and_repairs_next_id() {
        let mut p = full();
        p.next_id = 1;
        let s = Session::load_json(&Session::new(p).save_json().unwrap()).unwrap();
        assert_eq!(s.project.next_id, 8, "a deleted or existing id is never reused");
        for broken in [
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1}, {"id": 1, "name": "b", "duration": 1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 0, "name": "a", "duration": 1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": -1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "priority": 1001}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "priority": -1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "leveling_delay": -1}]}),
            json!({"name": "x", "start": "2026-01-05", "tasks": [{"id": 1, "name": "a", "duration": 1, "leveling_delay": model::MAX_MINUTES + 1}]}),
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

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn saves_and_bak_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("modes");
        let path = dir.join("plan.projectcraft");
        // An existing file and stale temp, both world-readable, end owner-only.
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(sibling(&path, ".tmp"), b"").unwrap();
        fs::set_permissions(sibling(&path, ".tmp"), fs::Permissions::from_mode(0o666)).unwrap();
        let s = Session::new(full());
        s.save_to(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&sibling(&path, ".bak")), 0o600);
        s.save_to(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&sibling(&path, ".bak")), 0o600);
        let copy = dir.join("plan.projectcraft.recovery");
        s.save_copy_to(&copy).unwrap();
        assert_eq!(mode(&copy), 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_and_folders_are_owner_only() {
        let dir = tmp_dir("private");
        let nested = dir.join("a").join("b");
        create_private_dir_all(&nested).unwrap();
        assert_eq!(mode(&dir.join("a")), 0o700);
        assert_eq!(mode(&nested), 0o700);
        let file = nested.join("recent-files.txt");
        write_private(&file, b"one\n").unwrap();
        write_private(&file, b"two\n").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two\n");
        assert_eq!(mode(&file), 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copy_is_saved_without_a_bak() {
        let dir = tmp_dir("copy");
        let path = dir.join("plan.projectcraft.recovery");
        let s = Session::new(full());
        s.save_copy_to(&path).unwrap();
        s.save_copy_to(&path).unwrap();
        assert_eq!(Session::open(&path).unwrap().project, full());
        assert!(!sibling(&path, ".bak").exists() && !sibling(&path, ".tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_interrupted_save_leaves_the_previous_file_intact() {
        let dir = tmp_dir("interrupted");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let before = fs::read(&path).unwrap();

        // The writer fails halfway through.
        let failed = replace_file(&path, true, |f| {
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

    /// A disk that fails at one step of a save: the flush of the staged file, or the rename.
    struct FailingDisk {
        sync: bool,
        rename: bool,
    }

    impl craft_atomic::AtomicFs for FailingDisk {
        fn create_new(&self, path: &Path, permissions: craft_atomic::NewFilePermissions) -> io::Result<File> {
            craft_atomic::RealFs.create_new(path, permissions)
        }
        fn sync(&self, file: &File) -> io::Result<()> {
            if self.sync { Err(io::Error::new(io::ErrorKind::StorageFull, "injected: disk full")) } else { craft_atomic::RealFs.sync(file) }
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.rename { Err(io::Error::other("injected: power cut before the rename")) } else { craft_atomic::RealFs.rename(from, to) }
        }
        fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
            craft_atomic::RealFs.hard_link(from, to)
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            craft_atomic::RealFs.remove(path)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            craft_atomic::RealFs.sync_dir(dir)
        }
        fn set_permissions(&self, file: &File, perms: fs::Permissions) -> io::Result<()> {
            craft_atomic::RealFs.set_permissions(file, perms)
        }
    }

    #[test]
    fn a_save_interrupted_by_the_disk_leaves_the_old_file_whole() {
        let dir = tmp_dir("injected");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let mut s = Session::new(full());
        s.run("task.add", &json!({"name": "Snag list", "duration": 3})).unwrap();
        let text = s.save_json().unwrap();
        let no_retry = craft_atomic::RenameRetry { attempts: 0, first_delay_ms: 0 };
        for disk in [FailingDisk { sync: true, rename: false }, FailingDisk { sync: false, rename: true }] {
            let failed = replace_file_with(&disk, no_retry, &path, true, |f| f.write_all(text.as_bytes()));
            assert!(failed.is_err());
            assert_eq!(fs::read(&path).unwrap(), before, "the old file, never a broken one");
            assert_eq!(Session::open(&path).unwrap().project, full());
            let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(left.len(), 2, "only the file and its .bak: {left:?}");
        }
        // The next save on a working disk lands whole.
        replace_file_with(&FailingDisk { sync: false, rename: false }, no_retry, &path, true, |f| f.write_all(text.as_bytes())).unwrap();
        assert_eq!(Session::open(&path).unwrap().project, s.project);
        assert_eq!(Session::open(&sibling(&path, ".bak")).unwrap().project, full());
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

    /// Writes `dir/name`, last modified `secs_ago` seconds ago.
    fn aged(dir: &Path, name: &str, secs_ago: u64) -> PathBuf {
        let p = dir.join(name);
        let f = File::create(&p).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago)).unwrap();
        p
    }

    #[test]
    fn stale_save_temps_are_swept_and_nothing_else() {
        let dir = tmp_dir("sweep");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let old = 2 * STALE_TEMP_SECS;
        let stale = aged(&dir, ".plan.projectcraft.4242-0.craft_tmp", old);
        let stale_bak = aged(&dir, ".plan.projectcraft.bak.4242-1.craft_tmp", old);
        let kept = [
            aged(&dir, &format!(".plan.projectcraft.{}-7.craft_tmp", std::process::id()), 5), // a save in progress
            aged(&dir, ".plan.projectcraft.4243-0.craft_tmp", 60),                            // recent
            aged(&dir, ".other.projectcraft.4242-0.craft_tmp", old),                          // another file's
            aged(&dir, ".plan.projectcraft.recovery.4242-0.craft_tmp", old),                  // another file's
            aged(&dir, ".plan.projectcraft.x-0.craft_tmp", old),                              // not our pattern
            aged(&dir, ".plan.projectcraft.4242-0.craft_tmp.keep", old),
            aged(&dir, "plan.projectcraft.4242-0.craft_tmp", old),
            aged(&dir, "notes.txt", old),
        ];
        fs::create_dir(dir.join(".plan.projectcraft.4244-0.craft_tmp")).unwrap(); // a folder, not a file
        assert_eq!(sweep_stale_temps(&path), TempSweep { removed: 2, failed: false });
        assert!(!stale.exists() && !stale_bak.exists());
        for k in &kept {
            assert!(k.exists(), "{} kept", k.display());
        }
        assert!(dir.join(".plan.projectcraft.4244-0.craft_tmp").is_dir());
        // A missing folder or file is no failure.
        assert_eq!(sweep_stale_temps(&dir.join("nope").join("plan.projectcraft")), TempSweep::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_long_name_is_never_swept() {
        let dir = tmp_dir("sweep-long");
        let name = format!("{}.projectcraft", "a".repeat(60));
        let stem: String = name.chars().take(TEMP_STEM_CHARS).collect();
        let t = aged(&dir, &format!(".{stem}.4242-0.craft_tmp"), 2 * STALE_TEMP_SECS);
        assert_eq!(sweep_stale_temps(&dir.join(&name)), TempSweep::default());
        assert!(t.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_named_like_a_temp_is_not_followed_or_removed() {
        let dir = tmp_dir("sweep-link");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let target = aged(&dir, "precious.txt", 2 * STALE_TEMP_SECS);
        let link = dir.join(".plan.projectcraft.4242-0.craft_tmp");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(sweep_stale_temps_older_than(&path, std::time::Duration::ZERO), TempSweep::default());
        assert!(fs::symlink_metadata(&link).is_ok() && target.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_temp_that_cannot_be_removed_is_reported_and_left() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("sweep-locked");
        let path = dir.join("plan.projectcraft");
        Session::new(full()).save_to(&path).unwrap();
        let stale = aged(&dir, ".plan.projectcraft.4242-0.craft_tmp", 2 * STALE_TEMP_SECS);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        // Root can remove from a read-only folder; there is nothing to check then.
        if File::create(dir.join("probe")).is_err() {
            assert_eq!(sweep_stale_temps(&path), TempSweep { removed: 0, failed: true });
            assert!(stale.exists());
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
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
