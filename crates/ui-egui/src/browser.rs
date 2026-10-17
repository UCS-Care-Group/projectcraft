//! The built-in file browser for Open and Save As, as WordCraft's: shown when the system's file
//! dialog can't be (a Linux desktop with neither a file chooser portal nor zenity, where `rfd`
//! answers "cancelled" at once and nothing would ever show).
//!
//! A folder field with Up and Home, the folder's subfolders and its `.projectcraft` files (and, to
//! open, Microsoft Project XML `.xml` files, imported), a file name field, Open or Save, and Cancel. It only lists folders the owner moves to and never reads
//! a file: the chosen path goes to [`App::file_picked`], exactly as the system dialog's answer.

use std::path::{Path, PathBuf};

use egui::{Align2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::files::{EXTENSION, FileRequest, IMPORT_EXTENSION};
use crate::theme::Tokens;
use crate::{App, i18n};

/// At most this many entries are listed; a bigger folder shows the first ones and says so.
pub(crate) const MAX_LISTED: usize = 5000;
/// At most this many folder entries are read, so a huge folder can't stall the window.
const MAX_SCANNED: usize = 100_000;
const ROW: f32 = 22.0;

/// One subfolder or project file in the listing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The name as shown (a name that isn't valid UTF-8 shows replacement characters).
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) dir: bool,
}

/// A folder's subfolders (first) and project files, by name.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    pub(crate) entries: Vec<Entry>,
    /// The folder holds more than was listed.
    pub(crate) truncated: bool,
}

/// Whether `path` has one of `extensions` (any case).
fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The files a request lists: projects, and for Open also the files ProjectCraft imports.
pub(crate) fn listed_extensions(request: FileRequest) -> &'static [&'static str] {
    match request {
        FileRequest::Open => &[EXTENSION, IMPORT_EXTENSION],
        FileRequest::SaveAs => &[EXTENSION],
    }
}

/// The subfolders and files with one of `extensions` in `dir`, at most `cap` of them, folders
/// first and each by name. Hidden entries (a leading dot) are left out; entries that can't be read
/// are skipped.
pub(crate) fn list_dir(dir: &Path, cap: usize, extensions: &[&str]) -> std::io::Result<Listing> {
    let mut listing = Listing::default();
    for (i, item) in std::fs::read_dir(dir)?.enumerate() {
        if i >= MAX_SCANNED {
            listing.truncated = true;
            break;
        }
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        // A symbolic link to a folder is a folder.
        let dir = match item.file_type() {
            Ok(t) if t.is_symlink() => std::fs::metadata(&path).is_ok_and(|m| m.is_dir()),
            Ok(t) => t.is_dir(),
            Err(_) => continue,
        };
        if !dir && !has_extension(&path, extensions) {
            continue;
        }
        if listing.entries.len() >= cap {
            listing.truncated = true;
            break;
        }
        listing.entries.push(Entry { name, path, dir });
    }
    listing.entries.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(listing)
}

/// The owner's home folder.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// What was typed in a field, as a path: relative to `dir`, absolute, or under the home folder
/// (`~/…`). `None` when nothing was typed.
pub(crate) fn resolve(dir: &Path, typed: &str) -> Option<PathBuf> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if typed == "~" {
        return home();
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return home().map(|h| h.join(rest));
    }
    Some(dir.join(typed))
}

/// What Open or Save (or Enter, or a double click) did.
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// The browser stays open: it moved to a folder, or has something to say.
    Stay,
    /// The file to open or save to.
    Pick(PathBuf),
}

/// A message under the listing.
#[derive(Clone, Debug, PartialEq)]
enum Note {
    None,
    NotFound(String),
    Replace(PathBuf),
}

/// The open browser.
pub(crate) struct FileBrowser {
    pub(crate) request: FileRequest,
    /// Opened because the system dialog couldn't be shown: say so, and how to get it.
    unavailable: bool,
    pub(crate) dir: PathBuf,
    /// The folder field (edited by the owner, applied on Enter).
    location: String,
    /// The file name field.
    pub(crate) name: String,
    listing: Result<Listing, String>,
    /// The highlighted entry.
    selected: Option<PathBuf>,
    note: Note,
    focus_name: bool,
}

impl FileBrowser {
    pub(crate) fn new(request: FileRequest, start: PathBuf, name: String, unavailable: bool) -> Self {
        let mut b = FileBrowser {
            request,
            unavailable,
            dir: start.clone(),
            location: String::new(),
            name,
            listing: Ok(Listing::default()),
            selected: None,
            note: Note::None,
            focus_name: true,
        };
        b.navigate(start);
        b
    }

    /// Show the folder `dir`; an unreadable one says why, and Up still works.
    pub(crate) fn navigate(&mut self, dir: PathBuf) {
        self.listing = list_dir(&dir, MAX_LISTED, listed_extensions(self.request)).map_err(|e| e.to_string());
        self.location = dir.to_string_lossy().into_owned();
        self.dir = dir;
        self.selected = None;
        self.note = Note::None;
    }

    fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.navigate(parent);
        }
    }

    /// Open or Save: a highlighted or typed folder is opened; otherwise the typed name is the
    /// pick. Save As adds `.projectcraft` to a name without an extension and asks once more
    /// before replacing a file; Open wants a file that exists.
    pub(crate) fn accept(&mut self) -> Step {
        if let Some(dir) = self.selected.take().filter(|p| p.is_dir()) {
            self.navigate(dir);
            return Step::Stay;
        }
        let Some(mut path) = resolve(&self.dir, &self.name) else { return Step::Stay };
        if path.is_dir() {
            self.name.clear();
            self.navigate(path);
            return Step::Stay;
        }
        match self.request {
            FileRequest::Open => {
                if path.is_file() {
                    return Step::Pick(path);
                }
                self.note = Note::NotFound(self.name.trim().to_owned());
                Step::Stay
            }
            FileRequest::SaveAs => {
                if path.extension().is_none() {
                    path.set_extension(EXTENSION);
                }
                if path.exists() && self.note != Note::Replace(path.clone()) {
                    self.note = Note::Replace(path);
                    return Step::Stay;
                }
                Step::Pick(path)
            }
        }
    }

    /// A click on an entry highlights it; a file's name goes into the file name field.
    fn select(&mut self, e: &Entry) {
        self.selected = Some(e.path.clone());
        if !e.dir {
            self.name.clone_from(&e.name);
        }
        self.note = Note::None;
    }
}

impl App {
    /// The system file dialog couldn't be shown for `request` (no file chooser portal and no
    /// zenity on Linux): carry on in the built-in browser, which says why it is there.
    pub fn file_unavailable(&mut self, request: FileRequest) {
        self.open_file_browser(request, None, true);
    }

    /// Opens the built-in browser for `request` in `start` (by default: the folder it showed last,
    /// the project's folder, the home folder).
    pub fn open_file_browser(&mut self, request: FileRequest, start: Option<PathBuf>, unavailable: bool) {
        let start = start
            .filter(|d| d.is_dir())
            .or_else(|| self.browser_dir.clone().filter(|d| d.is_dir()))
            .or_else(|| self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf).filter(|d| d.is_dir()))
            .or_else(home)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        let name = match request {
            FileRequest::Open => String::new(),
            FileRequest::SaveAs => self.save_as_name(),
        };
        self.browser = Some(Box::new(FileBrowser::new(request, start, name, unavailable)));
    }

    /// Closes the browser with its answer (`None`: cancelled), as the system dialog's.
    pub(crate) fn close_file_browser(&mut self, picked: Option<PathBuf>) {
        let Some(b) = self.browser.take() else { return };
        self.browser_dir = Some(b.dir.clone());
        self.file_picked(b.request, picked);
    }

    /// The built-in browser, while it is open.
    pub(crate) fn file_browser(&mut self, ctx: &egui::Context) {
        let Some(mut b) = self.browser.take() else { return };
        let t = Tokens::get(ctx);
        let mut done: Option<Option<PathBuf>> = None;
        egui::Modal::new(egui::Id::new("file-browser")).show(ctx, |ui| {
            ui.set_width(620.0);
            done = body(ui, &mut b, &t);
        });
        self.browser = Some(b);
        if let Some(picked) = done {
            self.close_file_browser(picked);
        }
    }
}

/// The browser's contents; `Some` once it closes (`Some(None)`: cancelled).
fn body(ui: &mut Ui, b: &mut FileBrowser, t: &Tokens) -> Option<Option<PathBuf>> {
    ui.heading(match b.request {
        FileRequest::Open => tl!("Open"),
        FileRequest::SaveAs => tl!("Save As"),
    });
    if b.unavailable {
        ui.label(
            RichText::new(tl!(
                "The system file dialog isn't available, so ProjectCraft shows its own. To get the system dialog, install a file chooser portal (such as xdg-desktop-portal-gtk) or zenity."
            ))
            .color(t.text_dim),
        );
    }
    ui.add_space(6.0);
    let mut location_done = false;
    ui.horizontal(|ui| {
        if ui.add_enabled(b.dir.parent().is_some(), egui::Button::new(tl!("Up"))).on_hover_text(tl!("Up one level")).clicked() {
            b.up();
        }
        if ui.button(tl!("Home")).on_hover_text(tl!("Home folder")).clicked()
            && let Some(h) = home()
        {
            b.navigate(h);
        }
        let r = ui.add(egui::TextEdit::singleline(&mut b.location).desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            location_done = true;
            let typed = b.location.clone();
            if let Some(dir) = resolve(&b.dir, &typed) {
                b.navigate(dir);
            }
        }
    });
    ui.add_space(4.0);
    let mut pick = false;
    let mut go: Option<PathBuf> = None;
    let mut clicked: Option<Entry> = None;
    egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.input_border)).inner_margin(2.0).show(ui, |ui| {
        let entries: &[Entry] = b.listing.as_ref().map(|l| l.entries.as_slice()).unwrap_or(&[]);
        egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show_rows(
            ui,
            ROW,
            entries.len(),
            |ui, range| {
                for e in entries.get(range).unwrap_or(&[]) {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    let chosen = b.selected.as_ref() == Some(&e.path);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, chosen, &e.name));
                    if chosen {
                        ui.painter().rect_filled(r, 2.0, t.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 2.0, t.hover);
                    }
                    let icon = Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(16.0, 16.0));
                    paint_icon(ui.painter(), icon, e.dir, t);
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    ui.painter().with_clip_rect(r).text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, &e.name, font, t.text);
                    let resp = resp.on_hover_text(e.path.display().to_string());
                    if resp.double_clicked() {
                        if e.dir {
                            go = Some(e.path.clone());
                        } else {
                            clicked = Some(e.clone());
                            pick = true;
                        }
                    } else if resp.clicked() {
                        clicked = Some(e.clone());
                    }
                }
            },
        );
    });
    if let Some(e) = clicked {
        b.select(&e);
    }
    if let Some(dir) = go {
        b.navigate(dir);
    }
    let message = match (&b.listing, &b.note) {
        (_, Note::NotFound(name)) => Some((i18n::fmt(tl!("{name} wasn't found."), &[("name", name)]), t.bar_critical)),
        (_, Note::Replace(p)) => {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some((i18n::fmt(tl!("{name} already exists. Choose Save again to replace it."), &[("name", &name)]), t.warning))
        }
        (Err(e), _) => Some((i18n::fmt(tl!("This folder can't be read: {error}"), &[("error", e)]), t.bar_critical)),
        (Ok(l), _) if l.truncated => {
            Some((i18n::fmt(tl!("Showing the first {count} items."), &[("count", &l.entries.len().to_string())]), t.text_dim))
        }
        (Ok(l), _) if l.entries.is_empty() => Some((tl!("No folders or projects here.").to_owned(), t.text_dim)),
        _ => None,
    };
    if let Some((text, color)) = message {
        ui.label(RichText::new(text).color(color));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = ui.label(tl!("File name:"));
        let r = ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(f32::INFINITY)).labelled_by(label.id);
        if b.focus_name {
            r.request_focus();
            b.focus_name = false;
        }
        if r.changed() {
            b.selected = None;
            b.note = Note::None;
        }
    });
    let kinds = match b.request {
        FileRequest::Open => tl!("ProjectCraft projects (*.projectcraft) and Microsoft Project XML (*.xml), imported as a new project"),
        FileRequest::SaveAs => tl!("ProjectCraft projects (*.projectcraft)"),
    };
    ui.label(RichText::new(kinds).color(t.text_dim));
    ui.add_space(8.0);
    let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let enter = !location_done && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        cancel |= ui.button(tl!("Cancel")).clicked();
        let ok = match b.request {
            FileRequest::Open => tl!("Open"),
            FileRequest::SaveAs => tl!("Save"),
        };
        pick |= ui.add(egui::Button::new(RichText::new(ok).color(t.surface)).fill(t.accent)).clicked() || enter;
    });
    if cancel {
        return Some(None);
    }
    if pick && let Step::Pick(path) = b.accept() {
        return Some(Some(path));
    }
    None
}

/// A folder (a tabbed body) or a project file (a page with a folded corner), drawn in code.
fn paint_icon(p: &egui::Painter, r: Rect, dir: bool, t: &Tokens) {
    let stroke = Stroke::new(1.0, t.text_dim);
    if dir {
        let tab = Rect::from_min_size(pos2(r.min.x + 1.0, r.min.y + 2.0), vec2(6.0, 3.0));
        let body = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 4.0), pos2(r.max.x - 1.0, r.max.y - 2.0));
        p.rect_filled(tab, 1.0, t.bar);
        p.rect_filled(body, 1.5, t.bar);
    } else {
        let (l, rt, top, bot) = (r.min.x + 3.0, r.max.x - 3.0, r.min.y + 1.0, r.max.y - 1.0);
        let fold = 4.0;
        let page = vec![pos2(l, top), pos2(rt - fold, top), pos2(rt, top + fold), pos2(rt, bot), pos2(l, bot)];
        p.add(egui::Shape::convex_polygon(page, t.surface, stroke));
        p.line_segment([pos2(rt - fold, top), pos2(rt - fold, top + fold)], stroke);
        p.line_segment([pos2(rt - fold, top + fold), pos2(rt, top + fold)], stroke);
        // A small Gantt bar marks it as a project.
        p.rect_filled(Rect::from_min_size(pos2(l + 2.0, top + 8.0), vec2(5.0, 2.0)), 0.0, t.bar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use projectcraft_engine::model::Project;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("projectcraft-ui-browser-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Folders first, then project files (any case), each by name; hidden entries and other
    /// files are left out, and a big folder is cut off and says so.
    #[test]
    fn listing_filters_sorts_and_caps() {
        let dir = scratch("list");
        for f in ["b.projectcraft", "A.PROJECTCRAFT", "notes.txt", ".hidden.projectcraft", "c.projectcraft"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("zeta")).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        let l = list_dir(&dir, 100, &[EXTENSION]).unwrap();
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zeta", "A.PROJECTCRAFT", "b.projectcraft", "c.projectcraft"]);
        assert!(!l.truncated);
        let capped = list_dir(&dir, 2, &[EXTENSION]).unwrap();
        assert_eq!(capped.entries.len(), 2);
        assert!(capped.truncated);
        assert!(list_dir(&dir.join("missing"), 10, &[EXTENSION]).is_err(), "an unreadable folder is an error, not a panic");
        // Open lists Microsoft Project XML too, to import; Save As only projects.
        std::fs::write(dir.join("Plan.XML"), b"x").unwrap();
        let open = list_dir(&dir, 100, listed_extensions(FileRequest::Open)).unwrap();
        assert!(open.entries.iter().any(|e| e.name == "Plan.XML"));
        let save = list_dir(&dir, 100, listed_extensions(FileRequest::SaveAs)).unwrap();
        assert!(!save.entries.iter().any(|e| e.name == "Plan.XML"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save As: a typed folder is opened, a bare name gets `.projectcraft`, and an existing file
    /// is replaced only on the second Save.
    #[test]
    fn save_adds_the_extension_and_asks_before_replacing() {
        let dir = scratch("save");
        std::fs::create_dir(dir.join("plans")).unwrap();
        std::fs::write(dir.join("plans").join("old.projectcraft"), b"x").unwrap();
        let mut b = FileBrowser::new(FileRequest::SaveAs, dir.clone(), "Untitled.projectcraft".into(), false);
        b.name = "plans".into();
        assert_eq!(b.accept(), Step::Stay);
        assert_eq!(b.dir, dir.join("plans"));
        b.name = "office move".into();
        assert_eq!(b.accept(), Step::Pick(dir.join("plans").join("office move.projectcraft")));
        b.name = "old".into();
        assert_eq!(b.accept(), Step::Stay, "asks before replacing");
        assert_eq!(b.accept(), Step::Pick(dir.join("plans").join("old.projectcraft")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Open wants an existing file; a highlighted folder is opened; Up goes to the parent.
    #[test]
    fn open_wants_a_file_and_folders_navigate() {
        let dir = scratch("open");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("p.projectcraft"), b"x").unwrap();
        let mut b = FileBrowser::new(FileRequest::Open, dir.clone(), String::new(), true);
        assert_eq!(b.accept(), Step::Stay, "nothing typed");
        b.name = "gone.projectcraft".into();
        assert_eq!(b.accept(), Step::Stay);
        assert_eq!(b.note, Note::NotFound("gone.projectcraft".into()));
        let sub = b.listing.as_ref().unwrap().entries[0].clone();
        b.select(&sub);
        assert_eq!(b.accept(), Step::Stay);
        assert_eq!(b.dir, dir.join("sub"));
        let file = b.listing.as_ref().unwrap().entries[0].clone();
        b.select(&file);
        assert_eq!(b.name, "p.projectcraft");
        assert_eq!(b.accept(), Step::Pick(dir.join("sub").join("p.projectcraft")));
        b.up();
        assert_eq!(b.dir, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Hostile names never panic: control characters, separators, `..`, a lone `~`, very long
    /// names, a folder that isn't there.
    #[test]
    fn hostile_names_never_panic() {
        let dir = scratch("hostile");
        for req in [FileRequest::Open, FileRequest::SaveAs] {
            let mut b = FileBrowser::new(req, dir.join("nowhere"), String::new(), false);
            assert!(b.listing.is_err());
            for typed in ["\0", "..", "../..", "~", "~/", "/", "a/b/c", "\u{202e}x", "   ", &"x".repeat(5000), "C:\\x", "*?<>|"] {
                b.name = typed.into();
                let _ = b.accept();
                b.location = typed.into();
                if let Some(d) = resolve(&b.dir, typed) {
                    b.navigate(d);
                }
                b.up();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shell couldn't show a system dialog: Save As carries on in the built-in browser, and
    /// its answer saves the project; Cancel closes it and changes nothing.
    #[test]
    fn no_system_dialog_falls_back_to_the_built_in_browser() {
        let dir = scratch("fallback");
        let mut a = App::new(Session::new(Project::new("Office move", projectcraft_engine::time::Date::EPOCH)), None);
        a.run("task.add", json!({"name": "Pack"}));
        assert!(!a.save());
        assert_eq!(a.take_file_request(), Some(FileRequest::SaveAs));
        a.file_unavailable(FileRequest::SaveAs);
        let b = a.browser.as_mut().unwrap();
        assert!(b.unavailable);
        assert_eq!(b.name, "Office move.projectcraft");
        b.navigate(dir.clone());
        b.name = "move".into();
        let Step::Pick(path) = b.accept() else { panic!("picked") };
        a.close_file_browser(Some(path));
        assert!(a.browser.is_none());
        assert!(!a.is_dirty());
        assert_eq!(a.path.as_deref(), Some(dir.join("move.projectcraft").as_path()));
        assert_eq!(a.browser_dir.as_deref(), Some(dir.as_path()), "opens there next time");

        a.file_unavailable(FileRequest::Open);
        assert_eq!(a.browser.as_ref().unwrap().dir, dir);
        a.close_file_browser(None);
        assert!(a.browser.is_none());
        assert_eq!(a.path.as_deref(), Some(dir.join("move.projectcraft").as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
