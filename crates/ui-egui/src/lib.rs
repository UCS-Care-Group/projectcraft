//! ProjectCraft front end: the Gantt Chart view, a task sheet with its bars on the right.
//!
//! The UI is thin: it reads [`Session`] and changes it only through [`Session::run`].
//! Colours come from [`theme::Tokens`] and text goes through [`tl!`] ([`i18n`]).
//!
//! The sheet works like a spreadsheet: click, Shift-click and Ctrl-click select rows; the arrows
//! move the focused cell (Shift extends the selection); typing or F2 edits the focused cell and
//! Enter commits and moves down, Escape cancels; Insert inserts a task above, Delete deletes the
//! selected rows, Alt+Shift+→/← indent or outdent them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use egui::{Key, Modifiers, RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{
    DurationUnit, MAX_WORKING_TIMES, Outline, TaskId, WorkingTime, check_working_times, clock, duration_text, parse_clock,
};
use projectcraft_engine::{Session, WEEKDAYS, time::Date};
use serde_json::{Value, json};

/// An English UI string in the current interface language ([`i18n::t`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod i18n;
pub mod theme;
pub mod timescale;

use theme::Tokens;
mod assign;
mod browser;
mod files;
mod options;
pub use files::{EXTENSION, FileRequest, MAX_RECENT, parse_recent, push_recent, recent_text};
mod recovery;
mod resource_sheet;
mod selection;
pub use recovery::{AUTOSAVE_EVERY, RECOVERY_EVERY, UNTITLED_RECOVERY, recovery_path};
use selection::Selection;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Name,
    Duration,
    /// Read-only: the task's work, from its assignments.
    Work,
    Start,
    Finish,
    Preds,
    Resources,
    /// Read-only: the task's cost, its assignments' plus its fixed cost (and its subtasks').
    Cost,
    FixedCost,
    Deadline,
}

/// The columns, left to right: each field, its heading and its width.
const COLUMNS: [(Field, &str, f32); 10] = [
    (Field::Name, "Task Name", NAME_W),
    (Field::Duration, "Duration", 60.0),
    (Field::Work, "Work", 60.0),
    (Field::Start, "Start", 80.0),
    (Field::Finish, "Finish", 80.0),
    (Field::Preds, "Predecessors", 100.0),
    (Field::Resources, "Resource Names", 170.0),
    (Field::Cost, "Cost", 84.0),
    (Field::FixedCost, "Fixed Cost", 84.0),
    (Field::Deadline, "Deadline", 80.0),
];

/// The columns the cursor moves across, left to right.
const FIELDS: [Field; 10] = [
    Field::Name,
    Field::Duration,
    Field::Work,
    Field::Start,
    Field::Finish,
    Field::Preds,
    Field::Resources,
    Field::Cost,
    Field::FixedCost,
    Field::Deadline,
];

/// The command a typed Start or Finish issues: Start No Earlier Than or Finish No Earlier Than
/// that date. The cell keeps showing the scheduler's date, as in Microsoft Project. A typed
/// Deadline sets the deadline; an empty one clears it.
fn typed_date(field: Field, id: TaskId, text: &str) -> Option<(&'static str, Value)> {
    let ty = match field {
        Field::Start => "SNET",
        Field::Finish => "FNET",
        Field::Deadline if text.trim().is_empty() => return Some(("task.setDeadline", json!({"id": id.0}))),
        Field::Deadline => return Some(("task.setDeadline", json!({"id": id.0, "date": text.trim()}))),
        _ => return None,
    };
    Some(("task.setConstraint", json!({"id": id.0, "type": ty, "date": text.trim()})))
}

/// The command Alt+Shift+→ (indent) or Alt+Shift+← (outdent) issues for the selected rows.
fn outline_key(ids: &[TaskId], indent: bool, outdent: bool) -> Option<(&'static str, Value)> {
    let cmd = match (indent, outdent) {
        _ if ids.is_empty() => return None,
        (true, false) => "task.indent",
        (false, true) => "task.outdent",
        _ => return None,
    };
    Some((cmd, json!({"ids": ids.iter().map(|t| t.0).collect::<Vec<_>>()})))
}

/// The sheet keys pressed this frame, when no cell is being edited.
#[derive(Default)]
struct SheetKeys {
    /// Rows to move the cursor (negative is up), and whether Shift extends the selection.
    rows: isize,
    extend: bool,
    /// Columns to move the cursor.
    cols: isize,
    edit: bool,
    insert: bool,
    delete: bool,
    /// Text typed into the focused cell, which starts editing it.
    typed: String,
}

/// Which view fills the window, as Microsoft Project's View menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    /// The task sheet with its bars.
    #[default]
    GanttChart,
    ResourceSheet,
}

/// The Change Working Time dialog's own state: the weekday's working times and the exception
/// being typed. Everything else it shows is the project calendar, and every change it makes is a
/// `calendar.*` command.
#[derive(Default)]
struct WorkingTimeForm {
    date: String,
    working: bool,
    /// The exception's own hours; blank for the default.
    from: String,
    to: String,
    /// The weekday whose working times are shown (0 is Monday), and its from/to rows as typed.
    day: usize,
    rows: Vec<(String, String)>,
    /// The weekday `rows` were read from; `None` reads them again from the calendar.
    loaded: Option<usize>,
}

/// Weekday names for the working-times picker, Monday first.
const DAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// From/to rows as typed, blank rows skipped, as working times for a command (`8:00-12:00`); or
/// why they can't be.
fn typed_times(rows: &[(String, String)]) -> Result<Vec<String>, &'static str> {
    let mut times = Vec::new();
    for (from, to) in rows.iter().filter(|(f, t)| !(f.trim().is_empty() && t.trim().is_empty())) {
        let (Ok(f), Ok(t)) = (parse_clock(from), parse_clock(to)) else {
            return Err(tl!("Enter times as H:MM, such as 8:00 or 17:30."));
        };
        times.push(WorkingTime::new(f, t).map_err(|_| tl!("Each working time must end after it starts."))?);
    }
    check_working_times(&times).map_err(|_| tl!("Working times must be in order and must not overlap."))?;
    Ok(times.iter().map(ToString::to_string).collect())
}

/// Working times as the dialog lists them: `9:00–13:00, 14:00–16:00`.
fn times_text(times: &[WorkingTime]) -> String {
    times.iter().map(|w| format!("{}–{}", clock(w.from), clock(w.to))).collect::<Vec<_>>().join(", ")
}

pub struct App {
    pub session: Session,
    /// Where Save writes, if anywhere.
    pub path: Option<std::path::PathBuf>,
    /// The cell being typed in and its text; committed as one command when editing ends.
    editing: Option<(TaskId, Field, String)>,
    /// The editor opened this frame and wants the keyboard focus.
    focus_editor: bool,
    /// Enter ended the edit: the cursor moves down once it's committed.
    down_after_edit: bool,
    /// The selected rows and the focused cell's row; `field` is its column.
    sel: Selection,
    field: Field,
    /// A click on a cell this frame: its row, its column (none for the ID column), the modifiers, and whether it was a double click.
    clicked: Option<(TaskId, Option<Field>, Modifiers, bool)>,
    status: String,
    /// Light, dark or the system's; put into egui on the next frame.
    pub appearance: egui::ThemePreference,
    themed: Option<egui::ThemePreference>,
    /// The project as last opened or saved; differing from it means unsaved changes.
    saved: projectcraft_engine::model::Project,
    /// Recently opened or saved files, newest first (the shell loads and persists it).
    pub recent: Vec<std::path::PathBuf>,
    recent_changed: bool,
    /// A dialog for the shell to show.
    file_request: Option<FileRequest>,
    /// The unsaved-changes prompt, and what follows it.
    prompt: Option<files::Then>,
    /// What follows a Save As started from the prompt.
    after_save: Option<files::Then>,
    /// The owner chose to close; the window closes.
    closing: bool,
    title: String,
    /// Autosave and the recovery copy; `None` keeps nothing (the sample, screenshots).
    keeper: Option<recovery::Keeper>,
    /// The Change Working Time dialog, while it is open.
    working_time: Option<WorkingTimeForm>,
    /// The Project Options dialog, while it is open.
    options: Option<options::OptionsForm>,
    /// The built-in file browser, while it is open (when the system dialog can't be shown).
    browser: Option<Box<browser::FileBrowser>>,
    /// The folder the built-in browser showed last; it opens there next time.
    browser_dir: Option<std::path::PathBuf>,
    /// The Gantt Chart's day width in points: view state, not saved and not undone.
    day_w: f32,
    /// The view showing: view state, not saved and not undone.
    pub view: View,
    /// The Resource Sheet's selection and editing.
    resources: resource_sheet::ResourceSheet,
    /// The Assign Resources dialog, while it is open.
    assign: Option<assign::AssignForm>,
}

const ROW_H: f32 = 22.0;
/// The ID column: room for five digits.
const ID_W: f32 = 40.0;
/// How far each outline level indents a task name.
const INDENT_W: f32 = 16.0;
const NAME_W: f32 = 220.0;

impl App {
    pub fn new(session: Session, path: Option<std::path::PathBuf>) -> Self {
        let saved = session.project.clone();
        App {
            session,
            path,
            editing: None,
            focus_editor: false,
            down_after_edit: false,
            sel: Selection::default(),
            field: Field::Name,
            clicked: None,
            status: String::new(),
            appearance: egui::ThemePreference::System,
            themed: None,
            saved,
            recent: Vec::new(),
            recent_changed: false,
            file_request: None,
            prompt: None,
            after_save: None,
            closing: false,
            title: String::new(),
            keeper: None,
            working_time: None,
            options: None,
            browser: None,
            browser_dir: None,
            day_w: timescale::Preset::Days.day_w(),
            view: View::default(),
            resources: resource_sheet::ResourceSheet::default(),
            assign: None,
        }
    }

    /// Runs a command; an error goes to the status bar. True when it ran.
    fn run(&mut self, id: &str, params: Value) -> bool {
        self.status = match self.session.run(id, &params) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        };
        self.status.is_empty()
    }

    /// Every row's Unique ID, top to bottom.
    fn order(&self) -> Vec<TaskId> {
        self.session.project.tasks.iter().map(|t| t.id).collect()
    }

    /// The Gantt Chart's timescale preset (from the View menu).
    pub fn set_timescale(&mut self, preset: timescale::Preset) {
        self.day_w = preset.day_w();
    }

    /// One zoom step in (wider days) or out.
    pub fn zoom(&mut self, zoom_in: bool) {
        self.day_w = if zoom_in { timescale::zoom_in(self.day_w) } else { timescale::zoom_out(self.day_w) };
    }

    /// Opens the Change Working Time dialog on the project calendar.
    pub fn open_change_working_time(&mut self) {
        self.working_time = Some(WorkingTimeForm::default());
        self.status.clear();
    }

    /// Each frame before [`App::ui`], and also while the window is minimized or hidden, when `ui`
    /// does not run: the window's close request, held back while there are unsaved changes.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.intercept_close(ctx);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.themed != Some(self.appearance) {
            theme::apply(&ctx, self.appearance);
            self.themed = Some(self.appearance);
        }
        self.file_keys(&ctx);
        let (undo, redo) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, Key::Z), i.consume_key(egui::Modifiers::COMMAND, Key::Y)));
        let (zoom_in, zoom_out) = ctx.input_mut(|i| {
            let zoom_in = i.consume_key(egui::Modifiers::COMMAND, Key::Equals) | i.consume_key(egui::Modifiers::COMMAND, Key::Plus);
            (zoom_in, i.consume_key(egui::Modifiers::COMMAND, Key::Minus))
        });
        if zoom_in {
            self.zoom(true);
        }
        if zoom_out {
            self.zoom(false);
        }
        if undo {
            self.run("edit.undo", json!({}));
        }
        if redo {
            self.run("edit.redo", json!({}));
        }
        // Alt+F10: Assign Resources, as in Microsoft Project.
        if self.view == View::GanttChart && ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::F10)) {
            self.open_assign_resources();
        }
        if self.assign.as_ref().is_some_and(|f| self.session.project.task(f.task).is_err()) {
            self.assign = None;
        }
        // Undo, redo or a file opened can take rows away.
        let order = self.order();
        self.sel.prune(&order, None);
        let resource_order = self.resource_order();
        self.resources.sel.prune(&resource_order, None);
        if self.resources.editing.as_ref().is_some_and(|(id, _, _)| !resource_order.contains(id)) {
            self.resources.editing = None;
        }
        self.sheet_keys(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                self.file_menu(ui);
                ui.menu_button(tl!("Project"), |ui| {
                    if ui.button(tl!("Change Working Time…")).clicked() {
                        self.open_change_working_time();
                    }
                    if ui.button(tl!("Options…")).clicked() {
                        self.open_project_options();
                    }
                });
                self.view_menu(ui);
                ui.separator();
                ui.heading(&self.session.project.name);
                ui.separator();
                if self.view == View::ResourceSheet {
                    if ui.button(tl!("New Resource")).clicked() {
                        self.insert_resource();
                    }
                } else {
                    if ui.button(tl!("New Task")).clicked() {
                        self.run("task.add", json!({}));
                    }
                    let can = self.sel.cursor.is_some();
                    if ui
                        .add_enabled(can, egui::Button::new(tl!("Assign Resources…")))
                        .on_hover_text(tl!("Assign resources to the focused task (Alt+F10)"))
                        .clicked()
                    {
                        self.open_assign_resources();
                    }
                }
                if ui.add_enabled(self.session.can_undo(), egui::Button::new(tl!("Undo"))).clicked() {
                    self.run("edit.undo", json!({}));
                }
                if ui.add_enabled(self.session.can_redo(), egui::Button::new(tl!("Redo"))).clicked() {
                    self.run("edit.redo", json!({}));
                }
                if ui.button(tl!("Open")).clicked() {
                    self.request(files::Then::Open(None));
                }
                if ui.button(tl!("Save")).clicked() {
                    self.save();
                }
                // Zoom belongs to the Gantt Chart's timescale.
                if self.view == View::GanttChart {
                    ui.separator();
                    self.zoom_control(ui);
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            let text = match &self.session.schedule {
                Err(e) => i18n::fmt(tl!("Can't schedule: {error}"), &[("error", &e.to_string())]),
                Ok(s) if self.status.is_empty() => {
                    let p = &self.session.project;
                    i18n::fmt(
                        tl!("Start {start} · Finish {finish} · Cost {cost}"),
                        &[("start", &p.start.to_string()), ("finish", &s.finish.to_string()), ("cost", &p.currency.format(p.total_cost()))],
                    )
                }
                Ok(_) => self.status.clone(),
            };
            ui.label(text);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| match self.view {
                View::GanttChart => self.sheet(ui),
                View::ResourceSheet => self.resource_sheet(ui),
            });
        });
        self.change_working_time(&ctx);
        self.project_options(&ctx);
        self.assign_resources(&ctx);
        self.retype_prompt(&ctx);
        self.unsaved_prompt(&ctx);
        self.recovery_prompt(&ctx);
        self.file_browser(&ctx);
        self.update_title(&ctx);
    }

    /// The sheet's keyboard: Alt+Shift+→/← always; the rest only while no cell is being edited
    /// and no other text field has the keyboard.
    fn sheet_keys(&mut self, ctx: &egui::Context) {
        let order = self.order();
        let modal = self.prompt.is_some()
            || self.working_time.is_some()
            || self.options.is_some()
            || self.assign.is_some()
            || self.resources.retype.is_some()
            || self.browser.is_some()
            || self.keeper.as_ref().is_some_and(|k| k.offer.is_some());
        if self.view == View::ResourceSheet {
            if self.resources.editing.is_some() || ctx.text_edit_focused() || modal {
                return;
            }
            let k = read_sheet_keys(ctx, self.resources.sel.cursor.is_some());
            self.resource_keys(k);
            return;
        }
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let (indent, outdent) = ctx.input_mut(|i| (i.consume_key(alt_shift, Key::ArrowRight), i.consume_key(alt_shift, Key::ArrowLeft)));
        let ids = match &self.editing {
            Some((id, _, _)) => vec![*id],
            None => self.sel.ids(&order),
        };
        if let Some((cmd, params)) = outline_key(&ids, indent, outdent) {
            self.run(cmd, params);
        }
        if self.editing.is_some() || ctx.text_edit_focused() || modal {
            return;
        }
        let k = read_sheet_keys(ctx, self.sel.cursor.is_some());
        if k.rows != 0 {
            self.sel.step(&order, k.rows, k.extend);
        }
        if k.cols != 0 {
            let at = FIELDS.iter().position(|f| *f == self.field).unwrap_or(0);
            self.field = FIELDS.get(at.saturating_add_signed(k.cols)).copied().unwrap_or(self.field);
            if self.sel.cursor.is_none() {
                self.sel.step(&order, 0, false);
            }
        }
        if k.edit || !k.typed.trim().is_empty() {
            self.begin_edit((!k.typed.is_empty()).then_some(k.typed));
        }
        if k.insert {
            self.insert_row();
        }
        if k.delete {
            self.delete_rows();
        }
    }

    /// Switches the view. An edit in progress is committed first, as a click elsewhere would.
    pub fn set_view(&mut self, view: View) {
        if let Some((id, field, text)) = self.editing.take() {
            self.commit(id, field, text);
        }
        if let Some((id, field, text)) = self.resources.editing.take() {
            self.commit_resource(id, field, text);
        }
        self.view = view;
    }
}

/// The sheet keys pressed this frame (either sheet). `typing`: a cell has the focus, so typed
/// text starts editing it and is taken out of the input.
fn read_sheet_keys(ctx: &egui::Context, typing: bool) -> SheetKeys {
    ctx.input_mut(|i| {
        let mut k = SheetKeys::default();
        // Shift+arrow first: a plain-arrow shortcut would match it too.
        for (key, by) in [(Key::ArrowUp, -1), (Key::ArrowDown, 1)] {
            if i.consume_key(Modifiers::SHIFT, key) {
                (k.rows, k.extend) = (by, true);
            } else if i.consume_key(Modifiers::NONE, key) {
                k.rows = by;
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) {
            k.rows = 1;
        }
        for (key, by) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1), (Key::Tab, 1)] {
            if i.consume_key(Modifiers::NONE, key) {
                k.cols = by;
            }
        }
        k.edit = i.consume_key(Modifiers::NONE, Key::F2);
        k.insert = i.consume_key(Modifiers::NONE, Key::Insert);
        k.delete = i.consume_key(Modifiers::NONE, Key::Delete);
        if typing && !i.modifiers.command && !i.modifiers.alt {
            // Taken out of the input, so the editor this starts doesn't type it a second time.
            i.events.retain(|e| match e {
                egui::Event::Text(t) => {
                    k.typed.push_str(t);
                    false
                }
                _ => true,
            });
        }
        k
    })
}

impl App {
    /// Starts editing the focused cell, with `typed` replacing its text, or (F2) its text as it is.
    fn begin_edit(&mut self, typed: Option<String>) {
        let Some(id) = self.sel.cursor else { return };
        let Some(text) = self.cell_text(id, self.field) else { return };
        self.editing = Some((id, self.field, typed.unwrap_or(text)));
        self.focus_editor = true;
    }

    /// Insert: a new task above the focused row (at the end with none), which takes the focus.
    fn insert_row(&mut self) {
        let next = TaskId(self.session.project.next_id.max(1));
        let params = match self.sel.cursor {
            Some(c) => json!({"before": c.0}),
            None => json!({}),
        };
        if self.run("task.insert", params) && self.session.project.row_of(next).is_some() {
            self.sel.select_only(next);
            self.field = Field::Name;
        }
    }

    /// Delete: the selected rows (each with its subtasks). The focus stays on the same row number.
    fn delete_rows(&mut self) {
        let order = self.order();
        let ids = self.sel.ids(&order);
        if ids.is_empty() {
            return;
        }
        let at = self.sel.cursor.and_then(|c| order.iter().position(|t| *t == c));
        if self.run("task.delete", json!({"ids": ids.iter().map(|t| t.0).collect::<Vec<_>>()})) {
            self.sel.prune(&self.order(), at);
        }
    }

    /// A cell's text as the sheet shows it; `None` where it can't be edited (a summary's duration,
    /// the work, the cost).
    fn cell_text(&self, id: TaskId, field: Field) -> Option<String> {
        let p = &self.session.project;
        let t = p.task(id).ok()?;
        let dates = self.session.schedule.as_ref().ok().and_then(|s| s.tasks.get(&id).copied());
        let date = |d: Option<Date>| d.map_or_else(String::new, |d| d.to_string());
        Some(match field {
            Field::Name => t.name.clone(),
            Field::Duration if p.is_summary(id) => return None,
            Field::Duration => duration_text(t.duration, t.duration_unit, &p.options),
            Field::Work | Field::Cost => return None,
            Field::FixedCost => p.currency.format(t.fixed_cost),
            Field::Start => date(dates.map(|d| d.start)),
            Field::Finish => date(dates.map(|d| d.finish)),
            Field::Preds => p.preds_text_rows(&t.preds),
            Field::Resources => p.resource_names(id),
            Field::Deadline => date(t.deadline),
        })
    }

    /// Issues the command for a cell's new text.
    fn commit(&mut self, id: TaskId, field: Field, text: String) {
        if self.cell_text(id, field).as_deref() == Some(text.as_str()) {
            return;
        }
        match field {
            Field::Name => {
                self.run("task.rename", json!({"id": id.0, "name": text}));
            }
            Field::Duration => {
                // The text as typed: the command reads its unit, and keeps it for display.
                self.run("task.setDuration", json!({"id": id.0, "duration": text.trim()}));
            }
            Field::Start | Field::Finish | Field::Deadline => {
                if let Some((cmd, params)) = typed_date(field, id, &text) {
                    self.run(cmd, params);
                }
            }
            // Row IDs become Unique IDs; each lag goes as typed, so it keeps its unit and value.
            Field::Preds => match self.session.project.preds_rows_to_ids(&text) {
                Ok(by_id) => {
                    self.run("task.setPredecessors", json!({"id": id.0, "text": by_id}));
                }
                Err(e) => self.status = e.to_string(),
            },
            // Names as typed; a new name makes a work resource, and the status line says so.
            Field::Resources => {
                let before = self.session.project.clone();
                if self.run("task.setResourceNames", json!({"id": id.0, "text": text})) {
                    self.status = assign::created_message(&before, &self.session.project).unwrap_or_default();
                }
            }
            // Read in the project currency; the command takes whole minor units.
            Field::FixedCost => match self.session.project.currency.parse(&text) {
                Ok(amount) => {
                    self.run("task.setFixedCost", json!({"id": id.0, "amount": amount}));
                }
                Err(e) => self.status = e.to_string(),
            },
            Field::Work | Field::Cost => {}
        }
    }

    /// View: the timescale presets and zoom.
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tl!("View"), |ui| {
            for (view, label) in [(View::GanttChart, "Gantt Chart"), (View::ResourceSheet, "Resource Sheet")] {
                if ui.radio(self.view == view, tl!(label)).clicked() {
                    self.set_view(view);
                    ui.close();
                }
            }
            ui.separator();
            ui.label(RichText::new(tl!("Timescale")).color(Tokens::get(ui.ctx()).text_dim));
            let current = timescale::Preset::of(self.day_w);
            for preset in timescale::Preset::ALL {
                if ui.radio(current == preset, tl!(preset.label())).clicked() {
                    self.set_timescale(preset);
                    ui.close();
                }
            }
            ui.separator();
            // "Ctrl+=" rather than egui's "Ctrl+Equals".
            let shortcut = |key: Key, sign: &str| {
                ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key)).replace(key.name(), sign)
            };
            let (in_keys, out_keys) = (shortcut(Key::Equals, "="), shortcut(Key::Minus, "-"));
            if ui.add_enabled(self.day_w < timescale::MAX_DAY_W, egui::Button::new(tl!("Zoom In")).shortcut_text(in_keys)).clicked() {
                self.zoom(true);
            }
            if ui.add_enabled(self.day_w > timescale::MIN_DAY_W, egui::Button::new(tl!("Zoom Out")).shortcut_text(out_keys)).clicked() {
                self.zoom(false);
            }
        });
    }

    /// The toolbar's zoom control: out, the preset showing, in.
    fn zoom_control(&mut self, ui: &mut egui::Ui) {
        if ui.add_enabled(self.day_w > timescale::MIN_DAY_W, egui::Button::new("−")).on_hover_text(tl!("Zoom Out")).clicked() {
            self.zoom(false);
        }
        let current = timescale::Preset::of(self.day_w);
        egui::ComboBox::from_id_salt("timescale").selected_text(tl!(current.label())).width(72.0).show_ui(ui, |ui| {
            for preset in timescale::Preset::ALL {
                if ui.selectable_label(current == preset, tl!(preset.label())).clicked() {
                    self.set_timescale(preset);
                }
            }
        });
        if ui.add_enabled(self.day_w < timescale::MAX_DAY_W, egui::Button::new("+")).on_hover_text(tl!("Zoom In")).clicked() {
            self.zoom(true);
        }
    }

    /// Change Working Time: tick the working weekdays, set each weekday's working times, add and
    /// remove exceptions (a working one with its own times). Every change is a command, so each is
    /// one undo step and a refused one (no working weekday, overlapping times) changes nothing.
    fn change_working_time(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.working_time.take() else { return };
        let calendar = self.session.project.calendar.clone();
        if form.loaded != Some(form.day) {
            form.rows = calendar.weekday_times(form.day).iter().map(|w| (clock(w.from), clock(w.to))).collect();
            form.loaded = Some(form.day);
        }
        let error = self.status.clone();
        let t = Tokens::get(ctx);
        let mut commands: Vec<(&'static str, Value)> = Vec::new();
        let mut invalid: Option<&'static str> = None;
        let (mut add, mut close) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("change_working_time")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(tl!("Change Working Time"));
            ui.label(RichText::new(tl!("Project calendar")).color(t.text_dim));
            ui.add_space(10.0);

            ui.label(RichText::new(tl!("Working days")).strong());
            let mut working = calendar.working;
            ui.horizontal(|ui| {
                for (w, name) in working.iter_mut().zip(WEEKDAYS) {
                    ui.checkbox(w, tl!(name));
                }
            });
            if working != calendar.working {
                let weekdays: Vec<&str> = WEEKDAYS.iter().zip(working).filter(|(_, w)| *w).map(|(n, _)| *n).collect();
                commands.push(("calendar.setWorking", json!({ "weekdays": weekdays })));
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Working times for")).strong());
                let shown = DAY_NAMES.get(form.day).copied().unwrap_or("");
                egui::ComboBox::from_id_salt("working_times_day").selected_text(tl!(shown)).show_ui(ui, |ui| {
                    for (i, name) in DAY_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut form.day, i, tl!(name));
                    }
                });
            });
            if form.rows.is_empty() {
                ui.label(RichText::new(tl!("Non-working: add working times to work this day.")).color(t.text_dim));
            }
            let mut remove = None;
            for (i, (from, to)) in form.rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(tl!("From"));
                    ui.add(egui::TextEdit::singleline(from).hint_text("8:00").desired_width(56.0));
                    ui.label(tl!("To"));
                    ui.add(egui::TextEdit::singleline(to).hint_text("12:00").desired_width(56.0));
                    if ui.small_button(tl!("Delete")).on_hover_text(tl!("Delete these working times")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                form.rows.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(form.rows.len() < MAX_WORKING_TIMES, egui::Button::new(tl!("Add Times"))).clicked() {
                    form.rows.push((String::new(), String::new()));
                }
                if ui.button(tl!("Apply")).clicked() {
                    match typed_times(&form.rows) {
                        Ok(times) => {
                            let weekday = WEEKDAYS.get(form.day).copied().unwrap_or("Mon");
                            commands.push(("calendar.setWorkingTimes", json!({"weekday": weekday, "times": times})));
                        }
                        Err(why) => invalid = Some(why),
                    }
                }
            });
            ui.add_space(10.0);

            ui.label(RichText::new(tl!("Exceptions")).strong());
            if calendar.exceptions.is_empty() {
                ui.label(RichText::new(tl!("None: every day follows its weekday.")).color(t.text_dim));
            } else {
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    egui::Grid::new("exceptions").striped(true).num_columns(5).spacing([16.0, 4.0]).show(ui, |ui| {
                        for (date, worked) in &calendar.exceptions {
                            ui.label(date.to_string());
                            let weekday = WEEKDAYS.get(usize::from(date.weekday())).copied().unwrap_or("");
                            ui.label(RichText::new(tl!(weekday)).color(t.text_dim));
                            ui.label(if *worked { tl!("Working") } else { tl!("Non-working") });
                            ui.label(RichText::new(times_text(calendar.times_on(*date))).color(t.text_dim));
                            if ui.small_button(tl!("Remove")).clicked() {
                                commands.push(("calendar.removeException", json!({"date": date.to_string()})));
                            }
                            ui.end_row();
                        }
                    });
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = ui.label(tl!("Date"));
                let r = ui.add(egui::TextEdit::singleline(&mut form.date).hint_text(tl!("YYYY-MM-DD")).desired_width(96.0)).labelled_by(label.id);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.radio_value(&mut form.working, false, tl!("Non-working"));
                ui.radio_value(&mut form.working, true, tl!("Working"));
                let typed = !form.date.trim().is_empty();
                if (ui.add_enabled(typed, egui::Button::new(tl!("Add Exception"))).clicked() || enter) && typed {
                    let mut params = json!({"date": form.date.trim(), "working": form.working});
                    let row = [(form.from.clone(), form.to.clone())];
                    match typed_times(if form.working { &row[..] } else { &[] }) {
                        Ok(times) if !times.is_empty() => params["times"] = json!(times),
                        Ok(_) => {}
                        Err(why) => invalid = Some(why),
                    }
                    if invalid.is_none() {
                        commands.push(("calendar.addException", params));
                        add = true;
                    }
                }
            });
            if form.working {
                ui.horizontal(|ui| {
                    ui.label(tl!("Exception hours"));
                    ui.label(tl!("From"));
                    ui.add(egui::TextEdit::singleline(&mut form.from).hint_text("9:00").desired_width(56.0));
                    ui.label(tl!("To"));
                    ui.add(egui::TextEdit::singleline(&mut form.to).hint_text("13:00").desired_width(56.0));
                    ui.label(RichText::new(tl!("Blank: the default hours")).color(t.text_dim));
                });
            }

            if !error.is_empty() {
                ui.add_space(6.0);
                ui.label(error.as_str());
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        let mut ran = true;
        let changed = !commands.is_empty();
        for (id, params) in commands {
            ran &= self.run(id, params);
        }
        if let Some(why) = invalid {
            self.status = why.to_owned();
        } else if changed && ran {
            // Show the calendar as it now is.
            form.loaded = None;
            if add {
                form.date.clear();
                form.from.clear();
                form.to.clear();
            }
        }
        if !(close || modal.should_close()) {
            self.working_time = Some(form);
        }
    }

    fn sheet(&mut self, ui: &mut egui::Ui) {
        let scale = timescale::Scale::for_project(self.session.project.start, self.day_w);
        let finish = self.session.schedule.as_ref().map_or(self.session.project.start, |s| s.finish);
        // The chart reaches past the last deadline it can draw, too.
        let finish = self.session.project.tasks.iter().filter_map(|t| t.deadline).filter(|d| timescale::in_range(scale, *d)).fold(finish, Date::max);
        // At least as wide as the view, so the shading reaches its right edge.
        let chart_w = timescale::chart_width(scale, finish, ui.clip_rect().width());
        let calendar = self.session.project.calendar.clone();
        let order = self.order();
        let mut edits: Vec<(TaskId, Field, String)> = Vec::new();
        let t = Tokens::get(ui.ctx());
        // Grid row 0 is the header.
        let picked: Vec<bool> = order.iter().map(|id| self.sel.is_selected(*id)).collect();
        let row_colour = move |row: usize, _: &egui::Style| match row.checked_sub(1).and_then(|r| picked.get(r)) {
            Some(true) => Some(t.row_selected),
            _ if row % 2 == 1 => Some(t.stripe),
            _ => None,
        };
        egui::Grid::new("sheet").with_row_color(row_colour).min_row_height(ROW_H).min_col_width(0.0).show(ui, |ui| {
            // The unnamed column after ID holds indicators, as Project's Indicators column.
            // ID and the indicators are narrow, fixed-width columns.
            ui.add_sized([ID_W, ROW_H], egui::Label::new(RichText::new(tl!("ID")).strong()));
            ui.add_sized([ROW_H, ROW_H], egui::Label::new(""));
            for (field, h, width) in COLUMNS {
                if matches!(field, Field::Cost | Field::FixedCost) {
                    // Over the amounts, on the right as they are.
                    let text = tl!(h);
                    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let colour = ui.visuals().strong_text_color();
                    ui.painter().with_clip_rect(rect).text(rect.right_center() - Vec2::new(4.0, 0.0), egui::Align2::RIGHT_CENTER, text, font, colour);
                } else {
                    ui.label(RichText::new(tl!(h)).strong());
                }
            }
            let (header, _) = ui.allocate_exact_size(Vec2::new(chart_w, timescale::TIER_H * 2.0), Sense::hover());
            timescale::header(ui, header, scale);
            ui.end_row();
            let outline = Outline::of(&self.session.project);
            let work = self.session.project.work_by_task();
            let costs = self.session.project.cost_by_task();
            // The ID column is the row number; commands get Unique IDs.
            for (i, id) in order.iter().copied().enumerate() {
                let dates = self.session.schedule.as_ref().ok().and_then(|s| s.tasks.get(&id).copied());
                let summary = outline.is_summary(i);
                let (duration, deadline) = self.session.project.task(id).map_or((0, None), |t| (t.duration, t.deadline));
                let row_id = i.saturating_add(1).to_string();
                // Laid out as the other cells are, so the row number sits on their baseline.
                let r = ui
                    .horizontal(|ui| {
                        let (rect, r) = ui.allocate_exact_size(Vec2::new(ID_W, ROW_H), Sense::click());
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &row_id, egui::TextStyle::Body.resolve(ui.style()), t.text);
                        r
                    })
                    .inner;
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &row_id));
                self.take_click(ui, &r, id, None);
                indicator(ui, dates);
                let indent = (outline.level(i).saturating_sub(1).min(20) as f32) * INDENT_W;
                for (field, _, width) in COLUMNS {
                    let lead = if field == Field::Name { indent } else { 0.0 };
                    let value = self.cell_text(id, field);
                    let p = &self.session.project;
                    let shown = match &value {
                        Some(v) => v.clone(),
                        None if field == Field::Work => assign::work_text(work.get(&id).copied().unwrap_or(0), p),
                        None if field == Field::Cost => p.currency.format(costs.get(&id).copied().unwrap_or(0)),
                        // A summary's rolled-up duration shows in days.
                        None => duration_text(duration, DurationUnit::Days, &p.options),
                    };
                    let inner = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(lead);
                        self.cell(ui, id, field, &shown, width - lead, value.is_some())
                    });
                    if let Some(text) = inner.inner {
                        edits.push((id, field, text));
                    }
                }
                let shape = match (summary, duration) {
                    (true, _) => Bar::Summary,
                    (false, 0) => Bar::Milestone,
                    (false, _) => Bar::Task,
                };
                let names = self.session.project.resource_names(id);
                gantt_bar(ui, scale, chart_w, &calendar, dates, shape, deadline, &names);
                ui.end_row();
            }
        });
        for (id, field, text) in edits {
            self.commit(id, field, text);
        }
        if std::mem::take(&mut self.down_after_edit) {
            self.sel.step(&self.order(), 1, false);
        }
        if let Some((id, field, mods, double)) = self.clicked.take() {
            // A click elsewhere ends an edit: commit it first.
            if let Some((eid, efield, text)) = self.editing.take_if(|(e, f, _)| (*e, Some(*f)) != (id, field)) {
                self.commit(eid, efield, text);
            }
            self.sel.click(id, &self.order(), mods.shift, mods.command);
            if let Some(f) = field {
                self.field = f;
                if double {
                    self.begin_edit(None);
                }
            }
        }
    }

    /// Notes a click on a cell, for the sheet to act on once it's drawn. Cells never keep egui's
    /// keyboard focus, so the arrows, Enter and Space stay the sheet's.
    fn take_click(&mut self, ui: &egui::Ui, r: &egui::Response, id: TaskId, field: Option<Field>) {
        if r.has_focus() {
            r.surrender_focus();
        }
        if r.clicked() || r.double_clicked() {
            self.clicked = Some((id, field, ui.input(|i| i.modifiers), r.double_clicked()));
        }
    }

    /// A cell: its text, outlined when it has the focus; an editor while it's being edited, which
    /// returns the new text once when editing ends (Escape cancels).
    fn cell(&mut self, ui: &mut egui::Ui, id: TaskId, field: Field, value: &str, width: f32, editable: bool) -> Option<String> {
        if let Some((_, _, text)) = self.editing.as_mut().filter(|(i, f, _)| *i == id && *f == field) {
            // `min_size`: a grid cell offers only last frame's column width, which `desired_width` would shrink to.
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(width).min_size(Vec2::new(width, 0.0)));
            let text = text.clone();
            if std::mem::take(&mut self.focus_editor) {
                r.request_focus();
                return None;
            }
            let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if enter || escape || r.lost_focus() || !r.has_focus() {
                self.editing = None;
                r.surrender_focus();
                self.down_after_edit = enter;
                return (!escape).then_some(text);
            }
            return None;
        }
        let (rect, r) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, value));
        let t = Tokens::get(ui.ctx());
        let colour = if editable { t.text } else { t.text_dim };
        let font = egui::TextStyle::Body.resolve(ui.style());
        // Amounts sit on the right, so their digits line up.
        let painter = ui.painter().with_clip_rect(rect);
        if matches!(field, Field::Cost | Field::FixedCost) {
            painter.text(rect.right_center() - Vec2::new(4.0, 0.0), egui::Align2::RIGHT_CENTER, value, font, colour);
        } else {
            painter.text(rect.left_center() + Vec2::new(4.0, 0.0), egui::Align2::LEFT_CENTER, value, font, colour);
        }
        if self.sel.cursor == Some(id) && self.field == field {
            ui.painter().rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
        let r = match field {
            Field::Work => r.on_hover_text(assign::work_lines(&self.session.project, id)),
            Field::Cost => r.on_hover_text(assign::cost_lines(&self.session.project, id)),
            _ if !editable => r.on_hover_text(tl!("A summary task's duration is rolled up from its subtasks")),
            _ => r,
        };
        self.take_click(ui, &r, id, Some(field));
        None
    }
}

/// The indicator cell, drawn in code: a warning triangle when a constraint overrides one of the
/// task's links, else a round "!" badge when the task finishes after its deadline. The tooltip
/// names every indicator that applies.
fn indicator(ui: &mut egui::Ui, dates: Option<projectcraft_engine::model::TaskDates>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ROW_H, ROW_H), Sense::hover());
    let Some(d) = dates.filter(|d| d.constraint_conflict || d.deadline_missed) else { return };
    let t = Tokens::get(ui.ctx());
    let (c, r) = (rect.center(), ROW_H * 0.36);
    let p = ui.painter();
    let mut tips = Vec::new();
    if d.constraint_conflict {
        p.add(egui::Shape::convex_polygon(
            vec![c + Vec2::new(0.0, -r), c + Vec2::new(r, r * 0.8), c + Vec2::new(-r, r * 0.8)],
            t.warning,
            Stroke::NONE,
        ));
        p.text(c + Vec2::new(0.0, r * 0.2), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(ROW_H * 0.45), t.surface);
        tips.push(tl!("Constraint conflict: a link can't be honoured, so this task keeps its constraint date and has negative slack."));
    } else {
        p.circle_filled(c, r * 0.85, t.deadline_missed);
        p.text(c, egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(ROW_H * 0.45), t.surface);
    }
    if d.deadline_missed {
        tips.push(tl!("Missed deadline: this task finishes after its deadline, so it has negative slack."));
    }
    response.on_hover_text(tips.join("\n"));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bar {
    Task,
    Milestone,
    /// A thin bar over its subtasks' span with a point down at each end.
    Summary,
}

/// One row's chart cell: the non-working days shaded, then the task's bar, its resource names to
/// the right of it (as Microsoft Project's default bar style), and its deadline marker on the
/// timescale.
#[allow(clippy::too_many_arguments)]
fn gantt_bar(
    ui: &mut egui::Ui,
    scale: timescale::Scale,
    chart_w: f32,
    calendar: &projectcraft_engine::model::Calendar,
    dates: Option<projectcraft_engine::model::TaskDates>,
    shape: Bar,
    deadline: Option<Date>,
    names: &str,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(chart_w, ROW_H), Sense::hover());
    // Through the gaps between rows too, so the shading runs top to bottom unbroken.
    timescale::shade(ui, rect.expand2(Vec2::new(0.0, ui.spacing().item_spacing.y / 2.0)), scale, calendar);
    let Some(d) = dates else { return };
    // Calendar days, so weekends show as gaps; within a day, at the moment's share of its working
    // time (see `timescale::moment_x`). Kept within the chart, whatever a file's dates.
    let x0 = timescale::moment_x(scale, calendar, d.start_at).clamp(0.0, chart_w);
    let x1 = timescale::moment_x(scale, calendar, d.finish_at).clamp(0.0, chart_w);
    // The deadline marker sits at the end of its day, as a finish does; a date off the chart
    // (a hostile one in year 9999) isn't drawn.
    let marker = deadline.and_then(|dl| timescale::end_of_day_x(scale, dl, chart_w));
    let t = Tokens::get(ui.ctx());
    let colour = if d.critical { t.bar_critical } else { t.bar };
    let p = ui.painter();
    if shape == Bar::Summary {
        let (left, right) = (rect.left() + x0, rect.left() + x1.max(x0 + 1.0));
        let (top, bottom) = (rect.top() + 7.0, rect.top() + 12.0);
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)), 0.0, t.bar_summary);
        let point = 6.0;
        for (outer, inner) in [(left, left + point), (right, right - point)] {
            let tip = egui::pos2(outer, bottom + point);
            p.add(egui::Shape::convex_polygon(vec![egui::pos2(outer, bottom), egui::pos2(inner, bottom), tip], t.bar_summary, Stroke::NONE));
        }
    } else if shape == Bar::Milestone {
        let c = egui::pos2(rect.left() + x1, rect.center().y); // its moment
        let r = ROW_H * 0.3;
        p.add(egui::Shape::convex_polygon(
            vec![c + Vec2::new(0.0, -r), c + Vec2::new(r, 0.0), c + Vec2::new(0.0, r), c + Vec2::new(-r, 0.0)],
            colour,
            Stroke::NONE,
        ));
    } else {
        // At least 2 points wide, so a task of minutes still shows at the coarsest zoom.
        let bar =
            egui::Rect::from_min_max(egui::pos2(rect.left() + x0, rect.top() + 5.0), egui::pos2(rect.left() + x1.max(x0 + 2.0), rect.bottom() - 5.0));
        p.rect_filled(bar, 2.0, colour);
    }
    if !names.is_empty() {
        let gap = if shape == Bar::Milestone { ROW_H * 0.3 + 4.0 } else { 6.0 };
        let at = egui::pos2(rect.left() + x1.max(x0 + 2.0) + gap, rect.center().y);
        let font = egui::TextStyle::Small.resolve(ui.style());
        p.with_clip_rect(rect).text(at, egui::Align2::LEFT_CENTER, names, font, t.text);
    }
    if let Some(x) = marker {
        deadline_marker(p, egui::pos2(rect.left() + x, rect.top()), &t, d.deadline_missed);
    }
}

/// The deadline marker, drawn in code: a short stem with a downward-pointing head whose tip
/// rests on the top edge of the bar, at the deadline's x. Missed, it takes the missed colour.
fn deadline_marker(p: &egui::Painter, top: egui::Pos2, t: &Tokens, missed: bool) {
    let colour = if missed { t.deadline_missed } else { t.deadline };
    // Stem from the row top, head from 4 to 14 pt down: over the bar's top edge, outlined in
    // the surface colour so it reads on a bar of any colour.
    let (x, half) = (top.x, 6.0);
    p.line_segment([egui::pos2(x, top.y), egui::pos2(x, top.y + 5.0)], Stroke::new(2.5, colour));
    p.add(egui::Shape::convex_polygon(
        vec![egui::pos2(x - half, top.y + 4.0), egui::pos2(x + half, top.y + 4.0), egui::pos2(x, top.y + 14.0)],
        colour,
        Stroke::new(1.0, t.surface),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colours live in `theme.rs` only: every other source file takes them from `Tokens`.
    #[test]
    fn no_colours_outside_the_token_set() {
        for (file, src) in [
            ("lib.rs", include_str!("lib.rs")),
            ("i18n.rs", include_str!("i18n.rs")),
            ("files.rs", include_str!("files.rs")),
            ("recovery.rs", include_str!("recovery.rs")),
            ("timescale.rs", include_str!("timescale.rs")),
            ("selection.rs", include_str!("selection.rs")),
            ("resource_sheet.rs", include_str!("resource_sheet.rs")),
            ("options.rs", include_str!("options.rs")),
            ("assign.rs", include_str!("assign.rs")),
        ] {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for needle in ["Color32", "from_rgb", "from_gray", "from_black_alpha", "from_white_alpha"] {
                assert!(!code.contains(needle), "{file} names a colour ({needle}); add a token in theme.rs");
            }
        }
    }

    /// The Change Working Time dialog changes the calendar only through commands: ticking a
    /// weekday, adding and removing an exception are each one undo step; unticking the last
    /// working weekday is refused, says why, and leaves the dialog open.
    #[test]
    fn change_working_time_edits_the_calendar_through_commands() {
        use egui_kittest::kittest::Queryable;
        let mut app = App::new(projectcraft_engine::sample(), None);
        app.open_change_working_time();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app);
        h.run_steps(3);
        h.get_by_label("Sat").click();
        h.run_steps(3);
        assert_eq!(h.state().session.project.calendar.working, [true, true, true, true, true, true, false]);

        h.get_by_label("Date").focus();
        h.run_steps(1);
        h.get_by_label("Date").type_text("2026-11-04");
        h.run_steps(2);
        h.get_by_label("Add Exception").click();
        h.run_steps(3);
        let d = Date::from_ymd(2026, 11, 4).unwrap();
        assert_eq!(h.state().session.project.calendar.exceptions.get(&d), Some(&false));
        h.get_by_label("Remove").click();
        h.run_steps(3);
        assert!(h.state().session.project.calendar.exceptions.is_empty());

        h.state_mut().session.run("calendar.setWorking", &json!({"weekdays": ["Mon"]})).unwrap();
        h.run_steps(2);
        let before = h.state().session.project.clone();
        h.get_by_label("Mon").click();
        h.run_steps(3);
        assert_eq!(h.state().session.project, before);
        assert!(h.state().status.contains("at least one working weekday"), "{}", h.state().status);
        assert!(h.state().working_time.is_some());
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().working_time.is_none());
    }

    /// Working times are edited as from/to rows: Apply sets the weekday's times in one command;
    /// times that aren't H:MM, run backwards or overlap are refused with a message and change
    /// nothing. A working exception takes its own hours.
    #[test]
    fn change_working_time_edits_working_times_and_exception_hours() {
        use egui_kittest::kittest::Queryable;
        use projectcraft_engine::model::parse_working_time;
        let mut app = App::new(projectcraft_engine::sample(), None);
        app.open_change_working_time();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app);
        h.run_steps(3);
        // Monday shows the default rows.
        let form = h.state().working_time.as_ref().unwrap();
        assert_eq!(form.rows, vec![("8:00".into(), "12:00".into()), ("13:00".into(), "17:00".into())]);
        // A short Monday: delete the afternoon, apply.
        h.get_all_by_label("Delete").nth(1).unwrap().click();
        h.run_steps(2);
        h.get_by_label("Apply").click();
        h.run_steps(3);
        assert_eq!(h.state().session.project.calendar.weekday_times(0), &[parse_working_time("8:00-12:00").unwrap()]);
        assert!(h.state().session.can_undo());
        // Typed nonsense and overlaps are refused with a message; the calendar is unchanged.
        for rows in [
            vec![("8".into(), "noon".into())],
            vec![("13:00".into(), "9:00".into())],
            vec![("8:00".into(), "13:00".into()), ("12:00".into(), "17:00".into())],
        ] {
            let before = h.state().session.project.clone();
            h.state_mut().working_time.as_mut().unwrap().rows = rows;
            h.get_by_label("Apply").click();
            h.run_steps(3);
            assert_eq!(h.state().session.project, before);
            assert!(!h.state().status.is_empty());
        }
        // A worked Saturday, 9:00–13:00.
        h.get_by_label("Date").focus();
        h.run_steps(1);
        h.get_by_label("Date").type_text("2026-11-07");
        h.get_by_label("Working").click();
        h.run_steps(2);
        let form = h.state_mut().working_time.as_mut().unwrap();
        (form.from, form.to) = ("9:00".into(), "13:00".into());
        h.get_by_label("Add Exception").click();
        h.run_steps(3);
        let sat = Date::from_ymd(2026, 11, 7).unwrap();
        assert_eq!(h.state().session.project.calendar.times_on(sat), &[parse_working_time("9:00-13:00").unwrap()], "{}", h.state().status);
        h.get_by_label("9:00–13:00");
    }

    /// Zoom is view state: Ctrl+= and Ctrl+- and the View menu change the timescale, never the
    /// project or its undo history.
    #[test]
    fn zoom_and_timescale_presets_are_view_state() {
        use egui_kittest::kittest::Queryable;
        let app = App::new(projectcraft_engine::sample(), None);
        let before = app.session.project.clone();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app);
        h.run_steps(2);
        assert_eq!(h.state().day_w, timescale::Preset::Days.day_w());
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Equals);
        h.run_steps(2);
        assert_eq!(h.state().day_w, timescale::zoom_in(timescale::Preset::Days.day_w()));
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Minus);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Minus);
        h.run_steps(2);
        assert_eq!(h.state().day_w, timescale::zoom_out(timescale::Preset::Days.day_w()));
        h.get_by_label("View").click();
        h.run_steps(2);
        h.get_by_label("Months").click();
        h.run_steps(2);
        assert_eq!(timescale::Preset::of(h.state().day_w), timescale::Preset::Months);
        assert_eq!(h.state().session.project, before);
        assert!(!h.state().session.can_undo());
    }

    /// Worked example 15 (sheet side): a typed Start sets SNET, a typed Finish FNET. The
    /// scheduling side is `crates/engine/tests/constraints.rs`.
    #[test]
    fn typing_a_start_or_finish_issues_snet_or_fnet() {
        let mut s = projectcraft_engine::sample();
        let (cmd, params) = typed_date(Field::Start, TaskId(5), " 2026-12-07 ").unwrap();
        assert_eq!(params, json!({"id": 5, "type": "SNET", "date": "2026-12-07"}));
        s.run(cmd, &params).unwrap();
        assert_eq!(s.schedule.as_ref().unwrap().tasks[&TaskId(5)].start.to_string(), "2026-12-07");
        let (_, params) = typed_date(Field::Finish, TaskId(5), "2026-12-18").unwrap();
        assert_eq!(params["type"], "FNET");
        assert!(typed_date(Field::Name, TaskId(5), "x").is_none());
        assert!(s.run(cmd, &typed_date(Field::Start, TaskId(5), "next week").unwrap().1).is_err());
    }

    /// A typed Deadline sets it; clearing the cell clears it. Nothing moves either way.
    #[test]
    fn typing_a_deadline_sets_or_clears_it() {
        let mut s = projectcraft_engine::sample();
        let before = s.schedule.as_ref().unwrap().tasks[&TaskId(4)];
        let (cmd, params) = typed_date(Field::Deadline, TaskId(4), " 2026-11-13 ").unwrap();
        assert_eq!((cmd, &params), ("task.setDeadline", &json!({"id": 4, "date": "2026-11-13"})));
        s.run(cmd, &params).unwrap();
        let after = s.schedule.as_ref().unwrap().tasks[&TaskId(4)];
        assert!(after.deadline_missed);
        assert_eq!((after.start, after.finish), (before.start, before.finish));
        let (cmd, params) = typed_date(Field::Deadline, TaskId(4), "  ").unwrap();
        assert_eq!(params, json!({"id": 4}));
        s.run(cmd, &params).unwrap();
        assert_eq!(s.project.task(TaskId(4)).unwrap().deadline, None);
    }

    /// Alt+Shift+→ / ← indent or outdent the selected rows; with none selected the keys do nothing.
    #[test]
    fn alt_shift_arrows_indent_and_outdent_the_selected_rows() {
        let ids = [TaskId(3), TaskId(4)];
        assert_eq!(outline_key(&ids, true, false), Some(("task.indent", json!({"ids": [3, 4]}))));
        assert_eq!(outline_key(&ids[..1], false, true), Some(("task.outdent", json!({"ids": [3]}))));
        assert_eq!(outline_key(&[], true, false), None);
        assert_eq!(outline_key(&ids, false, false), None);
        assert_eq!(outline_key(&ids, true, true), None);
    }

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn sheet(session: Session) -> Harness<'static, App> {
        let mut h = Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), App::new(session, None));
        h.run_steps(3);
        h
    }

    fn click(h: &mut Harness<'static, App>, label: &str, mods: Modifiers) {
        h.get_by_label(label).click_modifiers(mods);
        h.run_steps(3);
    }

    fn key(h: &mut Harness<'static, App>, mods: Modifiers, k: Key) {
        h.key_press_modifiers(mods, k);
        h.run_steps(3);
    }

    fn typed(h: &mut Harness<'static, App>, text: &str) {
        h.event(egui::Event::Text(text.into()));
        h.run_steps(3);
    }

    fn order(h: &Harness<'static, App>) -> Vec<u32> {
        h.state().order().iter().map(|t| t.0).collect()
    }

    fn picked(h: &Harness<'static, App>) -> Vec<u32> {
        let app = h.state();
        app.sel.ids(&app.order()).iter().map(|t| t.0).collect()
    }

    #[test]
    fn click_shift_click_ctrl_click_and_delete_the_selection() {
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "Survey the new floor", Modifiers::NONE);
        assert_eq!(picked(&h), [2]);
        click(&mut h, "Network cabling", Modifiers::SHIFT);
        assert_eq!(picked(&h), [2, 3, 4, 5]);
        click(&mut h, "Order furniture", Modifiers::COMMAND);
        assert_eq!(picked(&h), [2, 4, 5]);
        key(&mut h, Modifiers::NONE, Key::Delete);
        assert_eq!(order(&h), [1, 3, 6, 7, 8]);
        // The focus stays on row 2, now Order furniture.
        assert_eq!(picked(&h), [3]);
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(order(&h), [1, 2, 3, 4, 5, 6, 7, 8], "one undo step brings all three back");
    }

    #[test]
    fn arrows_move_the_focus_and_shift_extends_the_selection() {
        let mut h = sheet(projectcraft_engine::sample());
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
        assert_eq!(picked(&h), [1], "no focus yet: the first row");
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
        key(&mut h, Modifiers::SHIFT, Key::ArrowDown);
        key(&mut h, Modifiers::SHIFT, Key::ArrowDown);
        assert_eq!(picked(&h), [2, 3, 4]);
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        assert_eq!(picked(&h), [3]);
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        assert_eq!(h.state().field, Field::Start);
        key(&mut h, Modifiers::NONE, Key::ArrowLeft);
        assert_eq!(h.state().field, Field::Work);
        key(&mut h, Modifiers::NONE, Key::F2);
        assert!(h.state().editing.is_none(), "the work is read-only");
        assert_eq!(h.state().session.project.tasks.len(), 8, "moving about changes nothing");
    }

    #[test]
    fn the_deadline_column_is_reached_by_the_arrows_and_typed_into() {
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "Order furniture", Modifiers::NONE);
        for _ in 0..10 {
            key(&mut h, Modifiers::NONE, Key::ArrowRight);
        }
        assert_eq!(h.state().field, Field::Deadline, "the last column, and no further");
        typed(&mut h, "2026-11-13");
        key(&mut h, Modifiers::NONE, Key::Enter);
        let d = h.state().session.project.task(TaskId(3)).unwrap().deadline;
        assert_eq!(d.map(|d| d.to_string()).as_deref(), Some("2026-11-13"));
        assert_eq!(picked(&h), [4]);
    }

    /// Fixed Cost is typed in the project currency; Cost is read-only and adds it to the
    /// assignments' costs; the status bar shows the project total.
    #[test]
    fn fixed_cost_is_typed_and_cost_rolls_it_up() {
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "Furniture delivered", Modifiers::NONE);
        for _ in 0..7 {
            key(&mut h, Modifiers::NONE, Key::ArrowRight);
        }
        assert_eq!(h.state().field, Field::Cost);
        assert_eq!(h.state().cell_text(TaskId(4), Field::Cost), None, "the cost is read-only");
        key(&mut h, Modifiers::NONE, Key::F2);
        assert!(h.state().editing.is_none());
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        typed(&mut h, "£1,099.50");
        key(&mut h, Modifiers::NONE, Key::Enter);
        let p = &h.state().session.project;
        assert_eq!(p.task(TaskId(4)).unwrap().fixed_cost, 109_950);
        assert_eq!(p.cost_by_task()[&TaskId(4)], 109_950);
        assert_eq!(h.state().cell_text(TaskId(4), Field::FixedCost).as_deref(), Some("£1,099.50"));
        assert_eq!(assign::cost_lines(p, TaskId(4)), "Fixed cost: £1,099.50");
        assert_eq!(assign::cost_lines(p, TaskId(7)), "Removals crew: £1,062.00\nPriya Shah: £360.00\nTravel: £150.00\nTotal: £1,572.00");
        assert_eq!(assign::cost_lines(p, TaskId(8)), "No costs");
        assert!(h.query_all_by_label_contains("Cost £13,467.50").next().is_some(), "the project total in the status bar");
        // Back up to task 4 and type something that isn't money: nothing changes, the status says why.
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        let before = h.state().session.project.clone();
        typed(&mut h, "-5");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project, before);
        assert!(h.state().status.contains("not an amount of money"), "{}", h.state().status);
    }

    #[test]
    fn f2_or_typing_edits_the_focused_cell_enter_commits_and_moves_down_escape_cancels() {
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "Agree the brief", Modifiers::NONE);
        key(&mut h, Modifiers::NONE, Key::F2);
        assert!(h.state().editing.is_some());
        typed(&mut h, " today");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.tasks[0].name, "Agree the brief today");
        assert_eq!(picked(&h), [2], "Enter moves down");
        // Typing replaces the cell's text.
        typed(&mut h, "Survey");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.tasks[1].name, "Survey");
        assert_eq!(picked(&h), [3]);
        typed(&mut h, "Nothing");
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert!(h.state().editing.is_none());
        assert_eq!(h.state().session.project.tasks[2].name, "Order furniture");
        assert_eq!(picked(&h), [3]);
        // The Duration column.
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        typed(&mut h, "2w");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.tasks[2].duration, 10 * 480, "ten 8-hour days, in working minutes");
        assert_eq!(h.state().cell_text(TaskId(3), Field::Duration).as_deref(), Some("2w"), "shown in the unit typed");
        // Part-day durations and lags keep their unit.
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        typed(&mut h, "1.5d");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.tasks[2].duration, 720);
        assert_eq!(h.state().cell_text(TaskId(3), Field::Duration).as_deref(), Some("1.5d"));
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        typed(&mut h, "4h");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().cell_text(TaskId(3), Field::Duration).as_deref(), Some("4h"));
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        typed(&mut h, "3ed");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert!(h.state().status.contains("elapsed durations aren't supported yet"), "{}", h.state().status);
        assert_eq!(h.state().cell_text(TaskId(3), Field::Duration).as_deref(), Some("4h"));
        // Enter alone moves down too.
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(picked(&h), [5]);
    }

    #[test]
    fn insert_puts_a_task_above_the_focused_row() {
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "Network cabling", Modifiers::NONE);
        key(&mut h, Modifiers::NONE, Key::Insert);
        assert_eq!(order(&h), [1, 2, 3, 4, 9, 5, 6, 7, 8]);
        assert_eq!(picked(&h), [9], "the new task takes the focus");
        assert_eq!(h.state().field, Field::Name);
        typed(&mut h, "Book the lift");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.tasks[4].name, "Book the lift");
    }

    #[test]
    fn alt_shift_arrows_indent_the_selection() {
        let mut s = Session::new(projectcraft_engine::model::Project::new("t", Date::EPOCH));
        for n in ["Phase", "A", "B"] {
            s.run("task.add", &json!({"name": n})).unwrap();
        }
        let mut h = sheet(s);
        click(&mut h, "A", Modifiers::NONE);
        click(&mut h, "B", Modifiers::SHIFT);
        key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowRight);
        let levels: Vec<u32> = h.state().session.project.tasks.iter().map(|t| t.outline_level).collect();
        assert_eq!(levels, [1, 2, 2]);
        key(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowLeft);
        let levels: Vec<u32> = h.state().session.project.tasks.iter().map(|t| t.outline_level).collect();
        assert_eq!(levels, [1, 1, 1]);
        assert_eq!(picked(&h), [2, 3], "the selection stays");
    }

    fn resource_ids(h: &Harness<'static, App>) -> Vec<u32> {
        h.state().session.project.resources.iter().map(|r| r.id.0).collect()
    }

    /// The Resource Sheet, from the View menu, edits like the task sheet: typing into the focused
    /// cell, Enter to commit and move down, Tab across, Insert and Delete; each change one undo
    /// step. Switching views changes nothing in the project.
    #[test]
    fn the_resource_sheet_edits_resources_through_commands() {
        use projectcraft_engine::model::ResourceId;
        let mut h = sheet(projectcraft_engine::sample());
        let before = h.state().session.project.clone();
        click(&mut h, "View", Modifiers::NONE);
        click(&mut h, "Resource Sheet", Modifiers::NONE);
        assert_eq!(h.state().view, View::ResourceSheet);
        assert_eq!(h.state().session.project, before);
        assert!(h.query_by_label("£45.00/hr").is_some(), "rates show with the currency symbol");
        click(&mut h, "Tom Okafor", Modifiers::NONE);
        for _ in 0..5 {
            key(&mut h, Modifiers::NONE, Key::Tab);
        }
        typed(&mut h, "75%");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.resource(ResourceId(2)).unwrap().max_units, 7_500);
        assert_eq!(h.state().resources.sel.cursor, Some(ResourceId(3)), "Enter moves down");
        typed(&mut h, "lots");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert!(h.state().status.contains("not a percentage"), "{}", h.state().status);
        key(&mut h, Modifiers::NONE, Key::Insert);
        assert_eq!(resource_ids(&h), [1, 2, 3, 6, 4, 5]);
        typed(&mut h, "Sam Reid");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().session.project.resource(ResourceId(6)).unwrap().name, "Sam Reid");
        click(&mut h, "Packing boxes", Modifiers::NONE);
        click(&mut h, "Travel", Modifiers::SHIFT);
        key(&mut h, Modifiers::NONE, Key::Delete);
        assert_eq!(resource_ids(&h), [1, 2, 3, 6]);
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(resource_ids(&h), [1, 2, 3, 6, 4, 5], "one undo step brings both back");
        assert_eq!(h.state().session.project.tasks, before.tasks, "the tasks are untouched");
        click(&mut h, "View", Modifiers::NONE);
        click(&mut h, "Gantt Chart", Modifiers::NONE);
        assert_eq!(h.state().view, View::GanttChart);
    }

    /// Typing in Resource Names assigns by name; an unknown name becomes a work resource, the
    /// status line says so, and one undo takes back both. The Work column shows duration × units.
    #[test]
    fn resource_names_are_typed_and_a_new_name_is_one_undo_step() {
        let mut h = sheet(projectcraft_engine::sample());
        let before = h.state().session.project.clone();
        assert_eq!(h.state().cell_text(TaskId(2), Field::Resources).as_deref(), Some("Priya Shah[50%], Tom Okafor[50%]"));
        // Survey: 3 days, Priya at 50% and Tom at 50%: 24 hours.
        assert!(h.query_by_label("24h").is_some());
        click(&mut h, "Furniture delivered", Modifiers::NONE);
        for _ in 0..6 {
            key(&mut h, Modifiers::NONE, Key::ArrowRight);
        }
        assert_eq!(h.state().field, Field::Resources);
        typed(&mut h, "Sam Reid[50%], Tom Okafor");
        key(&mut h, Modifiers::NONE, Key::Enter);
        let p = &h.state().session.project;
        assert_eq!(p.resource_names(TaskId(4)), "Sam Reid[50%], Tom Okafor[50%]");
        assert!(h.state().status.contains("Added Sam Reid to the Resource Sheet"), "{}", h.state().status);
        // 10 days × (50% + 50%) = 80 hours.
        assert!(h.query_by_label("80h").is_some());
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().session.project, before, "one undo step takes back the assignment and the new resource");
        // Text that doesn't read says why and changes nothing.
        key(&mut h, Modifiers::NONE, Key::ArrowUp);
        typed(&mut h, "Packing boxes[lots]");
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert!(h.state().status.contains("not a material quantity"), "{}", h.state().status);
        assert_eq!(h.state().session.project, before);
    }

    /// Assign Resources (Alt+F10) adds, changes the units of and removes assignments of the
    /// focused task, each one command.
    #[test]
    fn assign_resources_adds_removes_and_changes_units() {
        use projectcraft_engine::model::ResourceId;
        let mut h = sheet(projectcraft_engine::sample());
        let before = h.state().session.project.clone();
        click(&mut h, "Furniture delivered", Modifiers::NONE);
        key(&mut h, Modifiers::ALT, Key::F10);
        assert_eq!(h.state().assign.as_ref().map(|f| f.task), Some(TaskId(4)));
        // Nothing assigned yet: the first Assign is Priya, at the default 100%.
        h.get_all_by_label("Assign").next().unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().session.project.resource_names(TaskId(4)), "Priya Shah");
        // Packing boxes with typed units: the third unassigned row.
        h.state_mut().assign.as_mut().unwrap().units.insert(ResourceId(4), "12 boxes".into());
        h.run_steps(2);
        h.get_all_by_label("Assign").nth(2).unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().session.project.resource_names(TaskId(4)), "Priya Shah, Packing boxes[12 boxes]");
        // Priya's units changed: typed into her row, then Enter.
        h.state_mut().assign.as_mut().unwrap().units.insert(ResourceId(1), "25%".into());
        h.run_steps(2);
        h.get_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().focus();
        h.run_steps(1);
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().session.project.resource_names(TaskId(4)), "Priya Shah[25%], Packing boxes[12 boxes]");
        // Units that don't read: refused with the reason, nothing changes.
        let now = h.state().session.project.clone();
        h.state_mut().assign.as_mut().unwrap().units.insert(ResourceId(2), "lots".into());
        h.run_steps(2);
        h.get_all_by_label("Assign").next().unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().session.project, now);
        assert!(h.state().status.contains("not a percentage"), "{}", h.state().status);
        h.get_all_by_label("Remove").next().unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().session.project.resource_names(TaskId(4)), "Packing boxes[12 boxes]");
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().assign.is_none());
        for _ in 0..4 {
            key(&mut h, Modifiers::COMMAND, Key::Z);
        }
        assert_eq!(h.state().session.project, before, "four commands, four undo steps");
    }

    /// Changing an assigned resource's type asks first: Cancel keeps everything, Change Type
    /// drops its assignments.
    #[test]
    fn changing_an_assigned_resources_type_asks_first() {
        use projectcraft_engine::model::{ResourceId, ResourceType};
        let mut h = sheet(projectcraft_engine::sample());
        click(&mut h, "View", Modifiers::NONE);
        click(&mut h, "Resource Sheet", Modifiers::NONE);
        let before = h.state().session.project.clone();
        h.state_mut().commit_resource(ResourceId(2), resource_sheet::RField::Type, "Material".into());
        h.run_steps(3);
        assert!(h.state().resources.retype.is_some());
        assert!(h.query_by_label_contains("removes those assignments").is_some());
        click(&mut h, "Cancel", Modifiers::NONE);
        assert_eq!(h.state().session.project, before);
        h.state_mut().commit_resource(ResourceId(2), resource_sheet::RField::Type, "Material".into());
        h.run_steps(3);
        click(&mut h, "Change Type", Modifiers::NONE);
        assert_eq!(h.state().session.project.resource(ResourceId(2)).unwrap().kind, ResourceType::Material);
        assert!(!h.state().session.project.is_assigned(ResourceId(2)));
        // A resource with no assignments changes type without a question.
        h.state_mut().commit_resource(ResourceId(2), resource_sheet::RField::Type, "Cost".into());
        h.run_steps(3);
        assert!(h.state().resources.retype.is_none());
        assert_eq!(h.state().session.project.resource(ResourceId(2)).unwrap().kind, ResourceType::Cost);
    }
}
