//! ProjectCraft front end: the Gantt Chart view, a task sheet with its bars on the right.
//!
//! The UI is thin: it reads [`Session`] and changes it only through [`Session::run`].
//! Colours come from [`theme::Tokens`] and text goes through [`tl!`] ([`i18n`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use egui::{Key, RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{Outline, TaskId, parse_duration, preds_text};
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
mod files;
pub use files::{EXTENSION, FileRequest, MAX_RECENT, parse_recent, push_recent, recent_text};
mod recovery;
pub use recovery::{AUTOSAVE_EVERY, RECOVERY_EVERY, UNTITLED_RECOVERY, recovery_path};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Duration,
    Start,
    Finish,
    Preds,
    Deadline,
}

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

/// The command Alt+Shift+→ (indent) or Alt+Shift+← (outdent) issues for the selected row.
/// ponytail: the selected row is the one whose cell has focus; multi-select comes with #10.
fn outline_key(selected: Option<TaskId>, indent: bool, outdent: bool) -> Option<(&'static str, Value)> {
    let id = selected?;
    let cmd = match (indent, outdent) {
        (true, false) => "task.indent",
        (false, true) => "task.outdent",
        _ => return None,
    };
    Some((cmd, json!({"ids": [id.0]})))
}

/// The Change Working Time dialog's own state: the exception being typed. Everything else it
/// shows is the project calendar, and every change it makes is a `calendar.*` command.
#[derive(Default)]
struct WorkingTime {
    date: String,
    working: bool,
}

pub struct App {
    pub session: Session,
    /// Where Save writes, if anywhere.
    pub path: Option<std::path::PathBuf>,
    /// The cell being typed in and its text; committed as one command when focus leaves it.
    editing: Option<(TaskId, Field, String)>,
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
    working_time: Option<WorkingTime>,
    /// The Gantt Chart's day width in points: view state, not saved and not undone.
    day_w: f32,
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
            day_w: timescale::Preset::Days.day_w(),
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
        self.working_time = Some(WorkingTime::default());
        self.status.clear();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.themed != Some(self.appearance) {
            theme::apply(&ctx, self.appearance);
            self.themed = Some(self.appearance);
        }
        self.intercept_close(&ctx);
        self.file_keys(&ctx);
        let (undo, redo) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, Key::Z), i.consume_key(egui::Modifiers::COMMAND, Key::Y)));
        let alt_shift = egui::Modifiers::ALT | egui::Modifiers::SHIFT;
        let (indent, outdent) = ctx.input_mut(|i| (i.consume_key(alt_shift, Key::ArrowRight), i.consume_key(alt_shift, Key::ArrowLeft)));
        if let Some((id, params)) = outline_key(self.editing.as_ref().map(|(id, _, _)| *id), indent, outdent) {
            self.run(id, params);
        }
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

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                self.file_menu(ui);
                ui.menu_button(tl!("Project"), |ui| {
                    if ui.button(tl!("Change Working Time…")).clicked() {
                        self.open_change_working_time();
                    }
                });
                self.view_menu(ui);
                ui.separator();
                ui.heading(&self.session.project.name);
                ui.separator();
                if ui.button(tl!("New Task")).clicked() {
                    self.run("task.add", json!({}));
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
                ui.separator();
                self.zoom_control(ui);
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            let text = match &self.session.schedule {
                Err(e) => i18n::fmt(tl!("Can't schedule: {error}"), &[("error", &e.to_string())]),
                Ok(s) if self.status.is_empty() => i18n::fmt(
                    tl!("Start {start} · Finish {finish}"),
                    &[("start", &self.session.project.start.to_string()), ("finish", &s.finish.to_string())],
                ),
                Ok(_) => self.status.clone(),
            };
            ui.label(text);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| self.sheet(ui));
        });
        self.change_working_time(&ctx);
        self.unsaved_prompt(&ctx);
        self.recovery_prompt(&ctx);
        self.update_title(&ctx);
    }

    /// View: the timescale presets and zoom.
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tl!("View"), |ui| {
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

    /// Change Working Time: tick the working weekdays, add and remove exceptions. Every change is
    /// a command, so each is one undo step and a refused one (no working weekday) changes nothing.
    fn change_working_time(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.working_time.take() else { return };
        let calendar = self.session.project.calendar.clone();
        let error = self.status.clone();
        let t = Tokens::get(ctx);
        let mut commands: Vec<(&'static str, Value)> = Vec::new();
        let (mut add, mut close) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("change_working_time")).show(ctx, |ui| {
            ui.set_width(460.0);
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

            ui.label(RichText::new(tl!("Exceptions")).strong());
            if calendar.exceptions.is_empty() {
                ui.label(RichText::new(tl!("None: every day follows its weekday.")).color(t.text_dim));
            } else {
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    egui::Grid::new("exceptions").striped(true).num_columns(4).spacing([16.0, 4.0]).show(ui, |ui| {
                        for (date, worked) in &calendar.exceptions {
                            ui.label(date.to_string());
                            let weekday = WEEKDAYS.get(usize::from(date.weekday())).copied().unwrap_or("");
                            ui.label(RichText::new(tl!(weekday)).color(t.text_dim));
                            ui.label(if *worked { tl!("Working") } else { tl!("Non-working") });
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
                    commands.push(("calendar.addException", json!({"date": form.date.trim(), "working": form.working})));
                    add = true;
                }
            });

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
        for (id, params) in commands {
            ran &= self.run(id, params);
        }
        if add && ran {
            form.date.clear();
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
        let mut commands: Vec<(&'static str, Value)> = Vec::new();
        egui::Grid::new("sheet").striped(true).min_row_height(ROW_H).min_col_width(0.0).show(ui, |ui| {
            // The unnamed column after ID holds indicators, as Project's Indicators column.
            // ID and the indicators are narrow, fixed-width columns.
            ui.add_sized([ID_W, ROW_H], egui::Label::new(RichText::new(tl!("ID")).strong()));
            ui.add_sized([ROW_H, ROW_H], egui::Label::new(""));
            for h in ["Task Name", "Duration", "Start", "Finish", "Predecessors", "Deadline"] {
                ui.label(RichText::new(tl!(h)).strong());
            }
            let (header, _) = ui.allocate_exact_size(Vec2::new(chart_w, timescale::TIER_H * 2.0), Sense::hover());
            timescale::header(ui, header, scale);
            ui.end_row();
            let p = &self.session.project;
            let outline = Outline::of(p);
            let rows: Vec<_> = p.tasks.iter().map(|t| (t.id, t.name.clone(), t.duration, p.preds_text_rows(&t.preds), t.deadline)).collect();
            // The ID column is the row number; commands get Unique IDs.
            for (i, (id, name, duration, preds, deadline)) in rows.into_iter().enumerate() {
                let dates = self.session.schedule.as_ref().ok().and_then(|s| s.tasks.get(&id).copied());
                let summary = outline.is_summary(i);
                ui.add_sized([ID_W, ROW_H], egui::Label::new(i.saturating_add(1).to_string()));
                indicator(ui, dates);
                let indent = (outline.level(i).saturating_sub(1).min(20) as f32) * INDENT_W;
                let renamed = ui.horizontal(|ui| {
                    ui.add_space(indent);
                    self.cell(ui, id, Field::Name, &name, NAME_W - indent)
                });
                if let Some(c) = renamed.inner {
                    commands.push(("task.rename", json!({"id": id.0, "name": c})));
                }
                let duration_text = format!("{duration}d");
                if summary {
                    // Rolled up from the subtasks; not editable while a summary.
                    ui.label(&duration_text).on_hover_text(tl!("A summary task's duration is rolled up from its subtasks"));
                } else if let Some(c) = self.cell(ui, id, Field::Duration, &duration_text, 60.0) {
                    match parse_duration(&c) {
                        Ok(d) => commands.push(("task.setDuration", json!({"id": id.0, "duration": d}))),
                        Err(e) => self.status = e.to_string(),
                    }
                }
                for (field, shown) in [(Field::Start, dates.map(|d| d.start)), (Field::Finish, dates.map(|d| d.finish))] {
                    if let Some(c) = self.cell(ui, id, field, &shown.map_or_else(String::new, |d| d.to_string()), 80.0) {
                        commands.extend(typed_date(field, id, &c));
                    }
                }
                if let Some(c) = self.cell(ui, id, Field::Preds, &preds, 100.0) {
                    match self.session.project.parse_preds_rows(&c) {
                        Ok(links) => commands.push(("task.setPredecessors", json!({"id": id.0, "text": preds_text(&links)}))),
                        Err(e) => self.status = e.to_string(),
                    }
                }
                if let Some(c) = self.cell(ui, id, Field::Deadline, &deadline.map_or_else(String::new, |d| d.to_string()), 80.0) {
                    commands.extend(typed_date(Field::Deadline, id, &c));
                }
                let shape = match (summary, duration) {
                    (true, _) => Bar::Summary,
                    (false, 0) => Bar::Milestone,
                    (false, _) => Bar::Task,
                };
                gantt_bar(ui, scale, chart_w, &calendar, dates, shape, deadline);
                ui.end_row();
            }
        });
        for (id, params) in commands {
            self.run(id, params);
        }
    }

    /// A text cell: shows `value`, edits a copy, returns the new text once when editing ends with a change.
    fn cell(&mut self, ui: &mut egui::Ui, id: TaskId, field: Field, value: &str, width: f32) -> Option<String> {
        let mine = matches!(&self.editing, Some((i, f, _)) if *i == id && *f == field);
        let mut text = match &self.editing {
            Some((_, _, t)) if mine => t.clone(),
            _ => value.to_owned(),
        };
        // `min_size`: a grid cell offers only last frame's column width, which `desired_width` would shrink to.
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width).min_size(Vec2::new(width, 0.0)));
        if r.has_focus() {
            self.editing = Some((id, field, text.clone()));
        }
        if mine && r.lost_focus() {
            self.editing = None;
            return (text != value).then_some(text);
        }
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

/// One row's chart cell: the non-working days shaded, then the task's bar and its deadline
/// marker on the timescale.
fn gantt_bar(
    ui: &mut egui::Ui,
    scale: timescale::Scale,
    chart_w: f32,
    calendar: &projectcraft_engine::model::Calendar,
    dates: Option<projectcraft_engine::model::TaskDates>,
    shape: Bar,
    deadline: Option<Date>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(chart_w, ROW_H), Sense::hover());
    // Through the gaps between rows too, so the shading runs top to bottom unbroken.
    timescale::shade(ui, rect.expand2(Vec2::new(0.0, ui.spacing().item_spacing.y / 2.0)), scale, calendar);
    let Some(d) = dates else { return };
    // Calendar days, so weekends show as gaps. Kept within the chart, whatever a file's dates.
    let x0 = scale.x(d.start).clamp(0.0, chart_w);
    let x1 = scale.x(d.finish.add_days(1).unwrap_or(d.finish)).clamp(0.0, chart_w);
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
        let c = egui::pos2(rect.left() + x1, rect.center().y); // the end of its day
        let r = ROW_H * 0.3;
        p.add(egui::Shape::convex_polygon(
            vec![c + Vec2::new(0.0, -r), c + Vec2::new(r, 0.0), c + Vec2::new(0.0, r), c + Vec2::new(-r, 0.0)],
            colour,
            Stroke::NONE,
        ));
    } else {
        let bar = egui::Rect::from_min_max(egui::pos2(rect.left() + x0, rect.top() + 5.0), egui::pos2(rect.left() + x1, rect.bottom() - 5.0));
        p.rect_filled(bar, 2.0, colour);
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

    /// Alt+Shift+→ / ← indent or outdent the selected row; with no row selected the keys do nothing.
    #[test]
    fn alt_shift_arrows_indent_and_outdent_the_selected_row() {
        assert_eq!(outline_key(Some(TaskId(3)), true, false), Some(("task.indent", json!({"ids": [3]}))));
        assert_eq!(outline_key(Some(TaskId(3)), false, true), Some(("task.outdent", json!({"ids": [3]}))));
        assert_eq!(outline_key(None, true, false), None);
        assert_eq!(outline_key(Some(TaskId(3)), false, false), None);
        assert_eq!(outline_key(Some(TaskId(3)), true, true), None);
        let mut s = projectcraft_engine::sample();
        s.run("task.setPredecessors", &json!({"id": 8, "text": ""})).unwrap();
        s.run("task.add", &json!({"name": "Unlinked"})).unwrap();
        let (cmd, params) = outline_key(Some(TaskId(9)), true, false).unwrap();
        s.run(cmd, &params).unwrap();
        assert!(s.project.is_summary(TaskId(8)));
    }
}
