//! Open, Save, Save As, the recent-files list and the unsaved-changes prompt. Open also imports
//! Microsoft Project XML (`.xml`) and CSV (`.csv`) as a new, untitled project, then shows the
//! import report; Save never writes the imported file, it asks for a `.projectcraft` name.
//!
//! The UI never shows a native dialog itself: it asks for one with a [`FileRequest`] that the
//! desktop shell takes ([`App::take_file_request`]), shows with `rfd`, and answers through
//! [`App::file_picked`]. So this crate stays free of `rfd`, and nothing programmatic (the CLI,
//! tests, later the control channel) can ever open a dialog. When the shell can't show a system
//! dialog it calls [`App::file_unavailable`] and the built-in browser takes over.

use std::path::{Path, PathBuf};

use projectcraft_engine::Session;
use projectcraft_engine::interchange::Format;

use crate::{App, Selection, i18n};

/// Longest the recent-files list gets.
pub const MAX_RECENT: usize = 10;
/// The extension of a ProjectCraft project.
pub const EXTENSION: &str = "projectcraft";
/// The extensions of Microsoft Project XML and CSV, which Open imports.
pub const IMPORT_EXTENSIONS: &[&str] = &["xml", "csv"];

/// Whether Open imports `path` (Microsoft Project XML or CSV) rather than opening it as a project.
pub fn is_import(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| IMPORT_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A native dialog the UI wants shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRequest {
    Open,
    SaveAs,
    /// File › Export: a copy in another program's format, never the project's own file.
    Export(Format),
}

/// What to do once unsaved changes are saved or dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Then {
    /// Close the window.
    Close,
    /// Open this file, or ask which with the Open dialog.
    Open(Option<PathBuf>),
}

/// `path` moved to the front of `list`, without duplicates, at most [`MAX_RECENT`] long.
pub fn push_recent(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_RECENT);
}

/// The recent-files list from its settings file: one path a line; blank lines, duplicates and
/// lines past [`MAX_RECENT`] dropped.
pub fn parse_recent(text: &str) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();
    for line in text.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        let p = PathBuf::from(line);
        if !list.contains(&p) && list.len() < MAX_RECENT {
            list.push(p);
        }
    }
    list
}

/// The settings-file text for `list`. A path with a line break in it can't be written and is left out.
pub fn recent_text(list: &[PathBuf]) -> String {
    let mut text = String::new();
    for p in list.iter().filter_map(|p| p.to_str()).filter(|s| !s.contains(['\n', '\r'])) {
        text.push_str(p);
        text.push('\n');
    }
    text
}

impl App {
    /// True when the project differs from what was last opened or saved, or was imported and
    /// not saved yet.
    pub fn is_dirty(&self) -> bool {
        self.unsaved_import || self.session.project != self.saved
    }

    /// The dialog the UI is waiting for, if any; the shell shows it and calls [`App::file_picked`].
    pub fn take_file_request(&mut self) -> Option<FileRequest> {
        self.file_request.take()
    }

    /// The recent-files list, if it changed since the last call (for the shell to persist).
    pub fn take_recent_changed(&mut self) -> Option<&[PathBuf]> {
        std::mem::take(&mut self.recent_changed).then_some(self.recent.as_slice())
    }

    /// The name Save As suggests: the file's own, or the project's name made fit for a file name.
    pub fn save_as_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| format!("{}.{EXTENSION}", safe_name(&self.session.project.name)), |n| n.to_string_lossy().into_owned())
    }

    /// The answer to a [`FileRequest`]: the chosen path, or `None` when cancelled (or no dialog
    /// could be shown).
    pub fn file_picked(&mut self, request: FileRequest, path: Option<PathBuf>) {
        match (request, path) {
            (FileRequest::Open, Some(p)) => self.open_path(&p),
            (FileRequest::SaveAs, Some(p)) if is_import(&p) => {
                // Never write a project over (or as) Microsoft Project XML or CSV.
                self.after_save = None;
                self.status = i18n::fmt(
                    tl!("Couldn't save: {path} is a Microsoft Project XML or CSV name; ProjectCraft saves .projectcraft files"),
                    &[("path", &p.display().to_string())],
                );
            }
            (FileRequest::SaveAs, Some(p)) => {
                let p = with_extension(p);
                if self.save_to(&p) {
                    self.path = Some(p);
                    if let Some(then) = self.after_save.take() {
                        self.proceed(then);
                    }
                }
            }
            (FileRequest::Export(format), Some(p)) => self.export_to(format, p),
            (_, None) => self.after_save = None,
        }
    }

    /// The name File › Export suggests: the project's file name (or name) with the format's
    /// extension.
    pub fn export_name(&self, format: Format) -> String {
        let name = self.save_as_name();
        let stem = Path::new(&name).file_stem().map_or_else(|| safe_name(&self.session.project.name), |s| s.to_string_lossy().into_owned());
        format!("{stem}.{}", format.extension())
    }

    /// Writes a copy of the project to `path` in `format` through `file.export`: a name without
    /// an extension gets the format's. The project, its file and its undo history are unchanged;
    /// the project's own file is never written.
    fn export_to(&mut self, format: Format, path: PathBuf) {
        let path = if path.extension().is_some() { path } else { path.with_extension(format.extension()) };
        if self.path.as_deref() == Some(path.as_path()) {
            self.status = i18n::fmt(tl!("Couldn't export: {path} is the project's own file"), &[("path", &path.display().to_string())]);
            return;
        }
        let shown = path.display().to_string();
        if self.run("file.export", serde_json::json!({"format": format.code(), "path": shown})) {
            self.status = i18n::fmt(tl!("Exported a copy as {format}: {path}"), &[("format", format.name()), ("path", &shown)]);
        } else {
            self.status = i18n::fmt(tl!("Couldn't export: {error}"), &[("error", &self.status)]);
        }
    }

    /// Opens `path` in place of the current project, or imports it when it is Microsoft Project
    /// XML or CSV ([`App::import_path`]). A file that can't be read, or that a newer ProjectCraft wrote,
    /// leaves the current project as it was and says why.
    pub fn open_path(&mut self, path: &Path) {
        if is_import(path) {
            self.import_path(path);
            return;
        }
        match Session::open(path) {
            Ok(session) => {
                // The project being left was saved or its changes dropped: its recovery copy goes.
                self.drop_recovery();
                self.saved = session.project.clone();
                self.session = session;
                self.path = Some(path.to_path_buf());
                self.editing = None;
                self.sel = Selection::default();
                self.resources = crate::resource_sheet::ResourceSheet::default();
                self.unsaved_import = false;
                self.import_report = None;
                self.status = i18n::fmt(tl!("Opened {path}"), &[("path", &path.display().to_string())]);
                if let Some(notice) = crate::calendar_notice(&self.session.project) {
                    self.status = format!("{} · {notice}", self.status);
                }
                self.sweep_temps(path);
                push_recent(&mut self.recent, path);
                self.recent_changed = true;
                self.check_recovery();
            }
            Err(e) => {
                self.status = i18n::fmt(tl!("Couldn't open: {error}"), &[("error", &e.to_string())]);
                if !path.exists() && self.recent.iter().any(|p| p == path) {
                    self.recent.retain(|p| p != path);
                    self.recent_changed = true;
                }
            }
        }
    }

    /// Imports `path` (Microsoft Project XML or CSV) as a new, untitled project in place of the current
    /// one, scheduled by ProjectCraft, and shows the import report. The file itself is never
    /// written: Save asks for a `.projectcraft` name. One that can't be imported leaves the current
    /// project as it was and says why.
    pub fn import_path(&mut self, path: &Path) {
        match Session::import_file(path, None) {
            Ok(session) => {
                self.drop_recovery();
                self.saved = session.project.clone();
                self.import_report = session.import_report().cloned().map(|r| (crate::import_report::file_name(path), r));
                self.session = session;
                self.path = None;
                self.unsaved_import = true;
                self.editing = None;
                self.sel = Selection::default();
                self.resources = crate::resource_sheet::ResourceSheet::default();
                self.status = i18n::fmt(
                    tl!("Imported {path} as a new project; Save keeps it as a ProjectCraft project"),
                    &[("path", &path.display().to_string())],
                );
                push_recent(&mut self.recent, path);
                self.recent_changed = true;
            }
            Err(e) => {
                self.status = i18n::fmt(tl!("Couldn't import: {error}"), &[("error", &e.to_string())]);
                if !path.exists() && self.recent.iter().any(|p| p == path) {
                    self.recent.retain(|p| p != path);
                    self.recent_changed = true;
                }
            }
        }
    }

    /// Writes the project to `path` (atomically) and marks it clean. Undo history is kept: saving,
    /// manual or auto, never clears it. A clean save deletes the recovery copy.
    pub(crate) fn write_file(&mut self, path: &Path) -> Result<(), projectcraft_engine::CmdError> {
        self.session.save_to(path)?;
        self.saved = self.session.project.clone();
        self.unsaved_import = false;
        push_recent(&mut self.recent, path);
        self.recent_changed = true;
        self.drop_recovery();
        Ok(())
    }

    /// Removes the temporary files that unfinished saves left beside `path`, after an open or a
    /// save; the status bar says so when one couldn't be removed.
    pub(crate) fn sweep_temps(&mut self, path: &Path) {
        if projectcraft_engine::sweep_stale_temps(path).failed {
            self.status = format!("{} · {}", self.status, tl!("Couldn't remove a temporary file left by an unfinished save"));
        }
    }

    /// Saves to `path`; true when it worked. Undo history is kept.
    fn save_to(&mut self, path: &Path) -> bool {
        match self.write_file(path) {
            Ok(()) => {
                self.status = i18n::fmt(tl!("Saved {path}"), &[("path", &path.display().to_string())]);
                self.sweep_temps(path);
                true
            }
            Err(e) => {
                self.status = i18n::fmt(tl!("Couldn't save: {error}"), &[("error", &e.to_string())]);
                false
            }
        }
    }

    /// Save: to the project's file, or through Save As when it has none.
    pub(crate) fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_to(&p),
            None => {
                self.file_request = Some(FileRequest::SaveAs);
                false
            }
        }
    }

    /// Opening another file or closing: asks first when there are unsaved changes.
    pub(crate) fn request(&mut self, then: Then) {
        if self.is_dirty() {
            self.prompt = Some(then);
        } else {
            self.proceed(then);
        }
    }

    fn proceed(&mut self, then: Then) {
        match then {
            Then::Close => self.closing = true,
            Then::Open(Some(p)) => self.open_path(&p),
            Then::Open(None) => self.file_request = Some(FileRequest::Open),
        }
    }

    /// The window's close button: held back while there are unsaved changes, until the owner answers.
    pub(crate) fn intercept_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            if self.is_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.prompt = Some(Then::Close);
                // Closed while minimized: bring the window back so the prompt can be answered.
                if ctx.input(|i| i.viewport().minimized == Some(true)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            } else {
                self.closing = true;
            }
        }
        if self.closing {
            // A clean close: saved, or the owner chose Don't Save.
            self.drop_recovery();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The Save / Don't Save / Cancel prompt, while one is open.
    pub(crate) fn unsaved_prompt(&mut self, ctx: &egui::Context) {
        let Some(then) = self.prompt.clone() else {
            return;
        };
        let name =
            self.path.as_deref().and_then(Path::file_name).map_or_else(|| self.session.project.name.clone(), |n| n.to_string_lossy().into_owned());
        let mut answer = None;
        egui::Modal::new(egui::Id::new("unsaved-changes")).show(ctx, |ui| {
            ui.heading(tl!("Save changes?"));
            ui.label(i18n::fmt(tl!("Do you want to save the changes you made to {name}?"), &[("name", &name)]));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Save")).clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.button(tl!("Don't Save")).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        match answer {
            None => {}
            Some(Answer::Cancel) => self.prompt = None,
            Some(Answer::Discard) => {
                self.prompt = None;
                self.proceed(then);
            }
            Some(Answer::Save) => {
                self.prompt = None;
                if self.path.is_some() {
                    if self.save() {
                        self.proceed(then);
                    }
                } else {
                    self.after_save = Some(then);
                    self.file_request = Some(FileRequest::SaveAs);
                }
            }
        }
    }

    /// The window title: the file's name, with a `*` while there are unsaved changes.
    pub(crate) fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(|| tl!("Untitled").to_owned(), |n| n.to_string_lossy().into_owned());
        let title = format!("{}{}", if self.is_dirty() { "*" } else { "" }, i18n::fmt(tl!("{name} - ProjectCraft"), &[("name", &name)]));
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// The File menu: Open, Open Recent, Save, Save As.
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tl!("File"), |ui| {
            if ui.add(egui::Button::new(tl!("Open…")).shortcut_text(ui.ctx().format_shortcut(&OPEN))).clicked() {
                self.request(Then::Open(None));
            }
            ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
                ui.menu_button(tl!("Open Recent"), |ui| {
                    let mut chosen = None;
                    for p in &self.recent {
                        let label = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                        if ui.button(label).on_hover_text(p.display().to_string()).clicked() {
                            chosen = Some(p.clone());
                        }
                    }
                    if let Some(p) = chosen {
                        self.request(Then::Open(Some(p)));
                    }
                });
            });
            ui.separator();
            if ui.add(egui::Button::new(tl!("Save")).shortcut_text(ui.ctx().format_shortcut(&SAVE))).clicked() {
                self.save();
            }
            if ui.add(egui::Button::new(tl!("Save As…")).shortcut_text(ui.ctx().format_shortcut(&SAVE_AS))).clicked() {
                self.file_request = Some(FileRequest::SaveAs);
            }
            ui.menu_button(tl!("Export"), |ui| {
                for format in Format::EXPORTS {
                    let label = i18n::fmt(tl!("{format}…"), &[("format", format.name())]);
                    if ui.button(label).on_hover_text(tl!("Write a copy for another program; the project stays as it is")).clicked() {
                        self.file_request = Some(FileRequest::Export(format));
                    }
                }
            });
            self.autosave_toggle(ui);
        });
    }

    /// Ctrl+O, Ctrl+S, Ctrl+Shift+S (Cmd on macOS). Ignored while the prompt is open.
    pub(crate) fn file_keys(&mut self, ctx: &egui::Context) {
        if self.prompt.is_some() || self.recovery_offered() || self.browser.is_some() || self.import_report.is_some() {
            return;
        }
        // Save As first: Ctrl+S would match Ctrl+Shift+S too.
        let (open, save_as, save) = ctx.input_mut(|i| (i.consume_shortcut(&OPEN), i.consume_shortcut(&SAVE_AS), i.consume_shortcut(&SAVE)));
        if open {
            self.request(Then::Open(None));
        }
        if save_as {
            self.file_request = Some(FileRequest::SaveAs);
        } else if save {
            self.save();
        }
    }
}

enum Answer {
    Save,
    Discard,
    Cancel,
}

const OPEN: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
const SAVE: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
const SAVE_AS: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::S);

/// A project name made fit for a file name: path separators and other characters some systems
/// refuse become spaces; an empty result is "Untitled".
fn safe_name(name: &str) -> String {
    let s: String = name.chars().take(100).map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let s = s.trim().trim_matches('.').trim();
    if s.is_empty() { "Untitled".into() } else { s.to_owned() }
}

/// A Save As name without an extension gets `.projectcraft` (some dialogs don't add it).
fn with_extension(p: PathBuf) -> PathBuf {
    if p.extension().is_some() { p } else { p.with_extension(EXTENSION) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use projectcraft_engine::model::Project;
    use serde_json::json;

    fn app() -> App {
        App::new(Session::new(Project::new("Test", projectcraft_engine::time::Date::EPOCH)), None)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("projectcraft-ui-files-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// File › Export › Microsoft Project XML… asks for a name, then writes a copy: the project,
    /// its file and its undo history stay as they are, and the project's own file is never
    /// written.
    #[test]
    fn file_export_writes_a_copy_and_never_the_project() {
        use egui_kittest::kittest::Queryable;
        let d = tmp("export");
        let mut a = App::new(projectcraft_engine::sample(), None);
        let own = d.join("Office move.projectcraft");
        a.file_picked(FileRequest::SaveAs, Some(own.clone()));
        a.run("task.add", json!({"name": "Mine"}));
        let saved = std::fs::read(&own).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), a);
        h.run_steps(2);
        h.get_by_label("File").click();
        h.run_steps(3);
        assert!(h.query_by_label_contains("Save As").is_some(), "the File menu is open");
        h.get_by_label_contains("Export").click();
        h.run_steps(2);
        h.get_by_label("Microsoft Project XML…").click();
        h.run_steps(2);
        assert_eq!(h.state_mut().take_file_request(), Some(FileRequest::Export(Format::Mspdi)));
        assert_eq!(h.state().export_name(Format::Mspdi), "Office move.xml");
        let before = h.state().session.project.clone();
        let dirty = h.state().is_dirty();
        // A name without an extension gets .xml.
        h.state_mut().file_picked(FileRequest::Export(Format::Mspdi), Some(d.join("Office move")));
        let xml = d.join("Office move.xml");
        assert_eq!(std::fs::read_to_string(&xml).unwrap(), h.state().session.export(Format::Mspdi).unwrap());
        assert!(h.state().status().starts_with("Exported a copy as Microsoft Project XML"), "{}", h.state().status());
        assert_eq!(h.state().session.project, before);
        assert_eq!(h.state().path.as_deref(), Some(own.as_path()), "the project keeps its own file");
        assert_eq!(h.state().is_dirty(), dirty, "an export doesn't save the project");
        // The project's own file is refused, and so is any .projectcraft.
        for bad in [own.clone(), d.join("Other.projectcraft")] {
            h.state_mut().file_picked(FileRequest::Export(Format::Mspdi), Some(bad.clone()));
            assert!(h.state().status().starts_with("Couldn't export"), "{}", h.state().status());
        }
        assert_eq!(std::fs::read(&own).unwrap(), saved);
        assert!(!d.join("Other.projectcraft").exists());
        // Undo still undoes the task added before the export.
        h.state_mut().run("edit.undo", json!({}));
        assert!(!h.state().session.project.tasks.iter().any(|t| t.name == "Mine"));
    }

    #[test]
    fn safe_name_strips_what_file_systems_refuse() {
        assert_eq!(safe_name("Office move"), "Office move");
        assert_eq!(safe_name("a/b\\c:d"), "a b c d");
        assert_eq!(safe_name("  ..  "), "Untitled");
        assert_eq!(safe_name(""), "Untitled");
        assert_eq!(safe_name(&"x".repeat(500)).len(), 100);
    }

    #[test]
    fn recent_list_moves_to_front_dedups_and_caps() {
        let mut list = Vec::new();
        for i in 0..12 {
            push_recent(&mut list, Path::new(&format!("/p/{i}.projectcraft")));
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0], PathBuf::from("/p/11.projectcraft"));
        push_recent(&mut list, Path::new("/p/5.projectcraft"));
        assert_eq!(list[0], PathBuf::from("/p/5.projectcraft"));
        assert_eq!(list.iter().filter(|p| **p == Path::new("/p/5.projectcraft")).count(), 1);
    }

    #[test]
    fn recent_text_round_trips_and_parse_is_forgiving() {
        let list = vec![PathBuf::from("/a b/x.projectcraft"), PathBuf::from("C:\\y.projectcraft")];
        assert_eq!(parse_recent(&recent_text(&list)), list);
        let junk = "\n/a\n/a\n\r\n".to_owned() + &"/z\n".repeat(3) + &(0..20).map(|i| format!("/{i}\n")).collect::<String>();
        let parsed = parse_recent(&junk);
        assert!(parsed.len() <= MAX_RECENT);
        assert_eq!(parsed[0], PathBuf::from("/a"));
        assert_eq!(recent_text(&[PathBuf::from("/bad\nname")]), "");
    }

    #[test]
    fn edits_make_dirty_save_as_makes_clean_and_undo_back_is_clean() {
        let dir = tmp("dirty");
        let mut a = app();
        assert!(!a.is_dirty());
        a.run("task.add", json!({"name": "One"}));
        assert!(a.is_dirty());
        assert!(!a.save(), "no file yet: asks for Save As");
        assert_eq!(a.take_file_request(), Some(FileRequest::SaveAs));
        a.file_picked(FileRequest::SaveAs, Some(dir.join("plan")));
        assert_eq!(a.path.as_deref(), Some(dir.join("plan.projectcraft").as_path()));
        assert!(!a.is_dirty());
        assert!(a.session.can_undo(), "saving keeps undo");
        assert_eq!(a.take_recent_changed().map(<[PathBuf]>::to_vec), Some(vec![dir.join("plan.projectcraft")]));
        assert!(a.take_recent_changed().is_none());
        a.run("task.add", json!({}));
        a.run("edit.undo", json!({}));
        assert!(!a.is_dirty());
    }

    #[test]
    fn opening_with_unsaved_changes_prompts_and_dont_save_opens() {
        let dir = tmp("prompt");
        let file = dir.join("other.projectcraft");
        let mut other = Session::new(Project::new("Other", projectcraft_engine::time::Date::EPOCH));
        other.run("task.add", &json!({"name": "Theirs"})).unwrap();
        other.save_to(&file).unwrap();

        let mut a = app();
        a.run("task.add", json!({"name": "Mine"}));
        a.request(Then::Open(Some(file.clone())));
        assert_eq!(a.prompt, Some(Then::Open(Some(file.clone()))));
        assert_eq!(a.session.project.name, "Test", "nothing opened before the owner answers");
        // Don't Save.
        let then = a.prompt.take().unwrap();
        a.proceed(then);
        assert_eq!(a.session.project.name, "Other");
        assert!(!a.is_dirty());
        assert_eq!(a.recent.first(), Some(&file));
    }

    #[test]
    fn save_then_close_without_a_file_goes_through_save_as() {
        let dir = tmp("close");
        let mut a = app();
        a.run("task.add", json!({}));
        a.request(Then::Close);
        assert!(!a.closing);
        // Save, with no file yet: Save As, then close.
        a.prompt = None;
        a.after_save = Some(Then::Close);
        a.file_request = Some(FileRequest::SaveAs);
        assert_eq!(a.take_file_request(), Some(FileRequest::SaveAs));
        a.file_picked(FileRequest::SaveAs, None);
        assert!(!a.closing, "cancelled Save As cancels the close");
        assert!(a.after_save.is_none());
        a.after_save = Some(Then::Close);
        a.file_picked(FileRequest::SaveAs, Some(dir.join("c.projectcraft")));
        assert!(a.closing);
        assert!(dir.join("c.projectcraft").exists());
    }

    #[test]
    fn clean_project_opens_and_closes_without_asking() {
        let mut a = app();
        a.request(Then::Open(None));
        assert!(a.prompt.is_none());
        assert_eq!(a.take_file_request(), Some(FileRequest::Open));
        a.request(Then::Close);
        assert!(a.closing);
    }

    #[test]
    fn unreadable_newer_and_missing_files_leave_the_project_and_say_why() {
        let dir = tmp("bad");
        let newer = dir.join("newer.projectcraft");
        std::fs::write(&newer, r#"{"format_version": 999}"#).unwrap();
        let junk = dir.join("junk.projectcraft");
        std::fs::write(&junk, [0xff, 0xfe, 0x00]).unwrap();
        let missing = dir.join("gone.projectcraft");
        let mut a = app();
        a.recent = vec![missing.clone(), junk.clone()];
        a.run("task.add", json!({"name": "Keep"}));
        for (p, says) in [(&newer, "newer ProjectCraft"), (&junk, "not a ProjectCraft project"), (&missing, "Couldn't open")] {
            a.open_path(p);
            assert!(a.status.contains(says), "{}: {}", p.display(), a.status);
            assert_eq!(a.session.project.tasks.len(), 1);
        }
        assert_eq!(a.recent, vec![junk], "the missing file is pruned; the unreadable one stays");
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_temp_that_cannot_be_removed_is_noted_in_the_status_bar() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("sweep");
        let file = dir.join("plan.projectcraft");
        Session::new(Project::new("Plan", projectcraft_engine::time::Date::EPOCH)).save_to(&file).unwrap();
        let temp = std::fs::File::create(dir.join(".plan.projectcraft.4242-0.craft_tmp")).unwrap();
        temp.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60)).unwrap();
        let mut a = app();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let locked = std::fs::File::create(dir.join("probe")).is_err(); // root can write anyway
        a.open_path(&file);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(a.status.starts_with("Opened"), "{}", a.status);
        assert_eq!(a.status.contains("temporary file"), locked, "{}", a.status);
        a.open_path(&file);
        assert!(!a.status.contains("temporary file"), "removed once it can be: {}", a.status);
        assert!(!dir.join(".plan.projectcraft.4242-0.craft_tmp").exists());
    }
}
