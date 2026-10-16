//! Autosave and the recovery copy.
//!
//! **Autosave** (on by default) saves the owner's file every [`AUTOSAVE_EVERY`] while there are
//! unsaved changes and the project has a file. Saving never clears undo.
//!
//! The **recovery copy** is always on while there are unsaved changes: `<file>.recovery` beside
//! the owner's file ([`recovery_path`]), or [`UNTITLED_RECOVERY`] in the app's data folder for an
//! untitled project. It's written atomically at most every [`RECOVERY_EVERY`], never replaces the
//! owner's file, and is deleted on a clean save or close (or when undo makes the project clean
//! again). A copy newer than the file is offered back on open (Restore / Discard).
//!
//! Time is passed in ([`App::tick`]): only the desktop shell reads the clock, so tests drive the
//! timers exactly and the wasm build never touches one. Without [`App::keep_recovery`] (the
//! sample, headless screenshots) nothing is written.

use std::path::{Path, PathBuf};
use std::time::Duration;

use projectcraft_engine::Session;
use projectcraft_engine::model::Project;

use crate::{App, i18n};

/// How often autosave saves the owner's file while there are unsaved changes.
pub const AUTOSAVE_EVERY: Duration = Duration::from_secs(120);
/// The most often the recovery copy is rewritten while there are unsaved changes.
pub const RECOVERY_EVERY: Duration = Duration::from_secs(30);
/// What a recovery copy adds to the owner's file name.
pub const RECOVERY_SUFFIX: &str = ".recovery";
/// An untitled project's recovery copy, in the folder given to [`App::keep_recovery`].
pub const UNTITLED_RECOVERY: &str = "untitled.projectcraft.recovery";

/// `plan.projectcraft` → `plan.projectcraft.recovery`, in the same folder.
pub fn recovery_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(RECOVERY_SUFFIX);
    PathBuf::from(s)
}

/// Autosave and recovery-copy state. Absent from an [`App`] that keeps nothing.
#[derive(Debug, Default)]
pub(crate) struct Keeper {
    autosave: bool,
    autosave_changed: bool,
    /// Where an untitled project's recovery copy lives.
    untitled_dir: Option<PathBuf>,
    /// When the project was first seen with unsaved changes, or last autosaved (or tried to be).
    autosave_from: Option<Duration>,
    /// When the recovery copy was last written (or tried to be).
    recovery_at: Option<Duration>,
    /// The project as last written to the recovery copy, so an unchanged one isn't rewritten.
    recovery_of: Option<Project>,
    /// The recovery copy on disk that belongs to the open project.
    written: Option<PathBuf>,
    /// A recovery copy waiting for the owner's Restore / Discard.
    pub(crate) offer: Option<PathBuf>,
}

impl App {
    /// Turns on autosave (if `autosave`) and the recovery copy; an untitled project's copy goes in
    /// `untitled_dir`. Then offers back a recovery copy left by a crash, if there is one.
    pub fn keep_recovery(&mut self, untitled_dir: Option<PathBuf>, autosave: bool) {
        self.keeper = Some(Keeper { autosave, untitled_dir, ..Keeper::default() });
        self.check_recovery();
    }

    /// Whether autosave is on.
    pub fn autosave(&self) -> bool {
        self.keeper.as_ref().is_some_and(|k| k.autosave)
    }

    /// The autosave switch, if the owner changed it since the last call (for the shell to persist).
    pub fn take_autosave_changed(&mut self) -> Option<bool> {
        let k = self.keeper.as_mut()?;
        std::mem::take(&mut k.autosave_changed).then_some(k.autosave)
    }

    /// The recovery copy for the open project: beside its file, or in the untitled folder.
    fn recovery_target(&self) -> Option<PathBuf> {
        let k = self.keeper.as_ref()?;
        match &self.path {
            Some(p) => Some(recovery_path(p)),
            None => k.untitled_dir.as_ref().map(|d| d.join(UNTITLED_RECOVERY)),
        }
    }

    pub(crate) fn recovery_offered(&self) -> bool {
        self.keeper.as_ref().is_some_and(|k| k.offer.is_some())
    }

    /// Offers the open project's recovery copy when it's newer than the file (or the file is gone).
    pub(crate) fn check_recovery(&mut self) {
        let offer = self.recovery_target().filter(|copy| match &self.path {
            Some(file) => is_newer(copy, file),
            None => copy.is_file(),
        });
        if let Some(k) = &mut self.keeper {
            *k = Keeper {
                autosave: k.autosave,
                autosave_changed: k.autosave_changed,
                untitled_dir: k.untitled_dir.take(),
                offer,
                ..Keeper::default()
            };
        }
    }

    /// Deletes the open project's recovery copy (after a clean save, open or close).
    pub(crate) fn drop_recovery(&mut self) {
        let Some(k) = &mut self.keeper else {
            return;
        };
        // A copy that can't be deleted is older than the saved file now, so it's never offered.
        if let Some(copy) = k.written.take() {
            let _ = std::fs::remove_file(copy);
        }
        k.recovery_of = None;
        k.recovery_at = None;
        k.autosave_from = None;
    }

    /// Runs autosave and the recovery copy when they're due. `now` is the time since any fixed
    /// point (the shell passes time since start).
    pub fn tick(&mut self, now: Duration) {
        let dirty = self.is_dirty();
        let Some(k) = &mut self.keeper else {
            return;
        };
        if k.offer.is_some() {
            return; // nothing is written over a copy the owner hasn't answered for
        }
        if !dirty {
            // Clean again (saved, or undone back to the file): the copy is stale.
            self.drop_recovery();
            return;
        }
        let since = *k.autosave_from.get_or_insert(now);
        let autosave_due = k.autosave && now.saturating_sub(since) >= AUTOSAVE_EVERY;
        let recovery_due =
            k.recovery_of.as_ref() != Some(&self.session.project) && k.recovery_at.is_none_or(|t| now.saturating_sub(t) >= RECOVERY_EVERY);
        if autosave_due && let Some(path) = self.path.clone() {
            k.autosave_from = Some(now);
            match self.write_file(&path) {
                Ok(()) => {
                    self.status = i18n::fmt(tl!("Autosaved {path}"), &[("path", &path.display().to_string())]);
                    return;
                }
                Err(e) => {
                    // The recovery copy is the fallback; the autosave failure is what the owner sees.
                    let said = i18n::fmt(tl!("Couldn't autosave: {error}"), &[("error", &e.to_string())]);
                    if recovery_due {
                        self.write_recovery(now);
                    }
                    self.status = said;
                    return;
                }
            }
        }
        if recovery_due {
            self.write_recovery(now);
        }
    }

    fn write_recovery(&mut self, now: Duration) {
        let Some(target) = self.recovery_target() else {
            return;
        };
        let result = target.parent().filter(|d| !d.as_os_str().is_empty()).map_or(Ok(()), projectcraft_engine::create_private_dir_all);
        let result = result.map_err(|e| e.to_string()).and_then(|()| self.session.save_copy_to(&target).map_err(|e| e.to_string()));
        let Some(k) = &mut self.keeper else {
            return;
        };
        k.recovery_at = Some(now);
        match result {
            Ok(()) => {
                k.recovery_of = Some(self.session.project.clone());
                if let Some(old) = k.written.replace(target.clone()).filter(|old| *old != target) {
                    let _ = std::fs::remove_file(old);
                }
            }
            Err(e) => self.status = i18n::fmt(tl!("Couldn't write the recovery copy: {error}"), &[("error", &e)]),
        }
    }

    /// Restore: the recovery copy becomes the open project, as unsaved changes. Read as hostile
    /// as any file; one that can't be read leaves the project as it was and stays on disk.
    fn restore(&mut self, copy: PathBuf) {
        match Session::open(&copy) {
            Ok(session) => {
                self.session = session;
                self.editing = None;
                self.status = tl!("Restored unsaved changes").to_owned();
                if let Some(k) = &mut self.keeper {
                    k.recovery_of = Some(self.session.project.clone());
                    k.written = Some(copy);
                }
            }
            Err(e) => self.status = i18n::fmt(tl!("Couldn't restore: {error}"), &[("error", &e.to_string())]),
        }
    }

    /// Discard: the recovery copy is deleted.
    fn discard(&mut self, copy: &Path) {
        if let Err(e) = std::fs::remove_file(copy)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            self.status = i18n::fmt(tl!("Couldn't delete the recovery copy: {error}"), &[("error", &e.to_string())]);
        }
    }

    /// The Restore / Discard prompt, while a recovery copy is offered.
    pub(crate) fn recovery_prompt(&mut self, ctx: &egui::Context) {
        let Some(copy) = self.keeper.as_ref().and_then(|k| k.offer.clone()) else {
            return;
        };
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(|| tl!("Untitled").to_owned(), |n| n.to_string_lossy().into_owned());
        let mut restore = None;
        egui::Modal::new(egui::Id::new("recovery-copy")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(tl!("Recover unsaved changes?"));
            ui.label(i18n::fmt(
                tl!("ProjectCraft closed before changes to {name} were saved. A recovery copy of them was kept."),
                &[("name", &name)],
            ));
            ui.label(tl!("Restore opens them as unsaved changes. Discard deletes the recovery copy."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Restore")).clicked() {
                    restore = Some(true);
                }
                if ui.button(tl!("Discard")).clicked() {
                    restore = Some(false);
                }
            });
        });
        let Some(restore) = restore else {
            return;
        };
        if let Some(k) = &mut self.keeper {
            k.offer = None;
        }
        if restore { self.restore(copy) } else { self.discard(&copy) }
    }

    /// File > Autosave, when autosave and recovery are kept.
    pub(crate) fn autosave_toggle(&mut self, ui: &mut egui::Ui) {
        let Some(k) = &mut self.keeper else {
            return;
        };
        ui.separator();
        if ui.checkbox(&mut k.autosave, tl!("Autosave")).on_hover_text(tl!("Save the file every 2 minutes while it has unsaved changes")).changed() {
            k.autosave_changed = true;
            k.autosave_from = None;
        }
    }
}

/// True when `copy` was modified after `file`, or `file` is gone. False when `copy` can't be read.
fn is_newer(copy: &Path, file: &Path) -> bool {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (modified(copy), modified(file)) {
        (Some(c), Some(f)) => c > f,
        (Some(_), None) => !file.exists(),
        (None, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use projectcraft_engine::time::Date;
    use serde_json::json;
    use std::time::SystemTime;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("projectcraft-ui-recovery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn project(name: &str) -> Project {
        Project::new(name, Date::EPOCH)
    }

    /// An app with `plan.projectcraft` saved in `dir`, keeping recovery.
    fn app_with_file(dir: &Path, autosave: bool) -> App {
        let file = dir.join("plan.projectcraft");
        Session::new(project("Plan")).save_to(&file).unwrap();
        let mut a = App::new(Session::open(&file).unwrap(), Some(file));
        a.keep_recovery(Some(dir.join("untitled")), autosave);
        a
    }

    /// Sets `path`'s modified time `secs` after `base`.
    fn touch(path: &Path, base: SystemTime, secs: u64) {
        std::fs::File::options().write(true).open(path).unwrap().set_modified(base + S(secs)).unwrap();
    }

    #[test]
    fn autosave_saves_every_two_minutes_and_keeps_undo() {
        let dir = tmp("autosave");
        let file = dir.join("plan.projectcraft");
        let mut a = app_with_file(&dir, true);
        a.tick(S(0));
        a.run("task.add", json!({"name": "One"}));
        a.tick(S(10));
        a.tick(S(129));
        assert!(a.is_dirty(), "not yet two minutes since the change");
        assert_eq!(Session::open(&file).unwrap().project.tasks.len(), 0);
        a.tick(S(130));
        assert!(!a.is_dirty());
        assert!(a.status.contains("Autosaved"), "{}", a.status);
        assert_eq!(Session::open(&file).unwrap().project.tasks.len(), 1);
        assert!(a.session.can_undo(), "autosave never clears undo");
        assert!(!recovery_path(&file).exists(), "a clean save deletes the recovery copy");
        a.run("edit.undo", json!({}));
        assert!(a.is_dirty(), "undo after autosave is a change to save");
        assert!(a.save());
        assert!(a.session.can_redo() && !a.session.can_undo(), "manual save keeps redo too");
    }

    #[test]
    fn autosave_off_or_untitled_only_keeps_a_recovery_copy() {
        let dir = tmp("off");
        let file = dir.join("plan.projectcraft");
        let mut a = app_with_file(&dir, false);
        a.run("task.add", json!({}));
        a.tick(S(0));
        a.tick(S(1000));
        assert!(a.is_dirty());
        assert!(recovery_path(&file).exists());

        let mut u = App::new(Session::new(project("New")), None);
        u.keep_recovery(Some(dir.join("data")), true);
        u.run("task.add", json!({}));
        u.tick(S(0));
        u.tick(S(1000));
        assert!(u.is_dirty(), "no file to autosave to");
        let copy = dir.join("data").join(UNTITLED_RECOVERY);
        assert_eq!(Session::open(&copy).unwrap().project.tasks.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.join("data")), 0o700, "a new recovery folder is owner-only");
            assert_eq!(mode(&copy), 0o600, "the recovery copy is owner-only");
            assert_eq!(mode(&recovery_path(&file)), 0o600);
        }
        // First Save As: the copy is saved over and leaves the app data folder.
        u.file_picked(crate::FileRequest::SaveAs, Some(dir.join("named.projectcraft")));
        assert!(!copy.exists());
        assert!(!recovery_path(&dir.join("named.projectcraft")).exists());
    }

    #[test]
    fn recovery_copy_is_written_at_most_every_30s_and_goes_when_clean() {
        let dir = tmp("copy");
        let copy = recovery_path(&dir.join("plan.projectcraft"));
        let mut a = app_with_file(&dir, true);
        a.tick(S(0));
        assert!(!copy.exists(), "nothing written while clean");
        a.run("task.add", json!({"name": "One"}));
        a.tick(S(1));
        assert_eq!(Session::open(&copy).unwrap().project.tasks.len(), 1, "written as soon as dirty");
        a.run("task.add", json!({"name": "Two"}));
        a.tick(S(30));
        assert_eq!(Session::open(&copy).unwrap().project.tasks.len(), 1, "not again within 30 s");
        a.tick(S(31));
        assert_eq!(Session::open(&copy).unwrap().project.tasks.len(), 2);
        assert!(!dir.join("plan.projectcraft.recovery.bak").exists());
        // Undone back to the file: clean, so the copy is stale and goes.
        a.run("edit.undo", json!({}));
        a.run("edit.undo", json!({}));
        a.tick(S(32));
        assert!(!copy.exists());
        // Dirty again, then a clean close (Don't Save) deletes it.
        a.run("task.add", json!({}));
        a.tick(S(100));
        assert!(copy.exists());
        a.closing = true;
        a.intercept_close(&egui::Context::default());
        assert!(!copy.exists());
        assert_eq!(Session::open(&dir.join("plan.projectcraft")).unwrap().project.tasks.len(), 0, "the owner's file is never touched by the copy");
    }

    #[test]
    fn a_newer_recovery_copy_is_offered_and_restore_opens_it_as_unsaved_changes() {
        let dir = tmp("offer");
        let file = dir.join("plan.projectcraft");
        let copy = recovery_path(&file);
        let mut crashed = Session::new(project("Plan"));
        crashed.run("task.add", &json!({"name": "Lost work"})).unwrap();
        Session::new(project("Plan")).save_to(&file).unwrap();
        crashed.save_copy_to(&copy).unwrap();
        let base = SystemTime::now();
        touch(&file, base, 0);
        touch(&copy, base, 5);

        let mut a = App::new(Session::new(project("Other")), None);
        a.keep_recovery(None, true);
        a.open_path(&file);
        assert!(a.recovery_offered());
        a.tick(S(0));
        a.tick(S(1000));
        assert!(copy.exists(), "nothing touches an offered copy");
        // Restore.
        let offered = a.keeper.as_mut().unwrap().offer.take().unwrap();
        a.restore(offered);
        assert_eq!(a.session.project.tasks[0].name, "Lost work");
        assert!(a.is_dirty(), "restored as unsaved changes");
        assert_eq!(a.path.as_deref(), Some(file.as_path()));
        assert!(a.save());
        assert!(!copy.exists(), "a clean save deletes it");
    }

    #[test]
    fn an_older_copy_is_not_offered_and_discard_deletes() {
        let dir = tmp("discard");
        let file = dir.join("plan.projectcraft");
        let copy = recovery_path(&file);
        Session::new(project("Plan")).save_to(&file).unwrap();
        Session::new(project("Plan")).save_copy_to(&copy).unwrap();
        let base = SystemTime::now();
        touch(&copy, base, 0);
        touch(&file, base, 5);
        let mut a = App::new(Session::new(project("Other")), None);
        a.keep_recovery(None, true);
        a.open_path(&file);
        assert!(!a.recovery_offered(), "older than the file");

        touch(&copy, base, 10);
        a.open_path(&file);
        assert!(a.recovery_offered());
        let offered = a.keeper.as_mut().unwrap().offer.take().unwrap();
        a.discard(&offered);
        assert!(!copy.exists());
        assert!(!a.is_dirty());
    }

    #[test]
    fn untitled_recovery_is_offered_on_start() {
        let dir = tmp("start");
        let mut crashed = Session::new(project("Draft"));
        crashed.run("task.add", &json!({"name": "Unsaved"})).unwrap();
        crashed.save_copy_to(&dir.join(UNTITLED_RECOVERY)).unwrap();
        let mut a = App::new(Session::new(project("New project")), None);
        a.keep_recovery(Some(dir.clone()), true);
        assert!(a.recovery_offered());
        let offered = a.keeper.as_mut().unwrap().offer.take().unwrap();
        a.restore(offered);
        assert_eq!(a.session.project.name, "Draft");
        assert!(a.is_dirty() && a.path.is_none());
    }

    #[test]
    fn a_hostile_recovery_copy_leaves_the_project_and_says_why() {
        let dir = tmp("hostile");
        for junk in
            [&b"\xff\xfe\x00"[..], br#"{"format_version": 999}"#, br#"{"name":"x","start":"2026-01-05","tasks":[{"id":0,"name":"a","duration":1}]}"#]
        {
            std::fs::write(dir.join(UNTITLED_RECOVERY), junk).unwrap();
            let mut a = App::new(Session::new(project("Mine")), None);
            a.keep_recovery(Some(dir.clone()), true);
            let offered = a.keeper.as_mut().unwrap().offer.take().unwrap();
            a.restore(offered);
            assert!(a.status.contains("Couldn't restore"), "{}", a.status);
            assert_eq!(a.session.project.name, "Mine");
            assert!(dir.join(UNTITLED_RECOVERY).exists(), "kept: the owner's data is never deleted unasked");
        }
    }

    #[test]
    fn failed_autosave_and_recovery_writes_say_so_and_keep_the_changes() {
        let dir = tmp("fail");
        // The project's folder and the untitled folder are a file, so nothing can be written.
        std::fs::write(dir.join("blocker"), b"").unwrap();
        let mut a = App::new(Session::new(project("New")), Some(dir.join("blocker").join("plan.projectcraft")));
        a.keep_recovery(Some(dir.join("blocker")), true);
        a.run("task.add", json!({}));
        a.tick(S(0));
        assert!(a.status.contains("Couldn't write the recovery copy"), "{}", a.status);
        a.tick(S(120));
        assert!(a.status.contains("Couldn't autosave"), "{}", a.status);
        assert!(a.is_dirty() && a.session.can_undo());
        a.tick(S(121));
        a.path = None;
        a.tick(S(200));
        assert!(a.status.contains("Couldn't write the recovery copy"), "{}", a.status);
    }

    #[test]
    fn without_keep_recovery_nothing_is_written() {
        // As `--sample` and headless screenshots.
        let dir = tmp("sample");
        let file = dir.join("plan.projectcraft");
        let mut a = App::new(Session::new(project("Sample")), Some(file.clone()));
        a.run("task.add", json!({}));
        a.tick(S(0));
        a.tick(S(10_000));
        assert!(!file.exists() && !recovery_path(&file).exists());
        assert!(!a.autosave() && a.take_autosave_changed().is_none());
    }
}
