//! ProjectCraft front end: the Gantt Chart view, a task sheet with its bars on the right.
//!
//! The UI is thin: it reads [`Session`] and changes it only through [`Session::run`].
//! Colours come from [`theme::Tokens`] and text goes through [`tl!`] ([`i18n`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use egui::{Key, RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{TaskId, parse_duration, preds_text};
use projectcraft_engine::{Session, time::Date};
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

use theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Duration,
    Start,
    Finish,
    Preds,
}

/// The command a typed Start or Finish issues: Start No Earlier Than or Finish No Earlier Than
/// that date. The cell keeps showing the scheduler's date, as in Microsoft Project.
fn typed_date(field: Field, id: TaskId, text: &str) -> Option<(&'static str, Value)> {
    let ty = match field {
        Field::Start => "SNET",
        Field::Finish => "FNET",
        _ => return None,
    };
    Some(("task.setConstraint", json!({"id": id.0, "type": ty, "date": text.trim()})))
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
}

const ROW_H: f32 = 22.0;
const DAY_W: f32 = 18.0;

impl App {
    pub fn new(session: Session, path: Option<std::path::PathBuf>) -> Self {
        App { session, path, editing: None, status: String::new(), appearance: egui::ThemePreference::System, themed: None }
    }

    fn run(&mut self, id: &str, params: Value) {
        self.status = match self.session.run(id, &params) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        };
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.status = tl!("Start ProjectCraft with a file name to save (projectcraft plan.projectcraft)").to_owned();
            return;
        };
        self.status = match self.session.save_to(&path) {
            Ok(()) => i18n::fmt(tl!("Saved {path}"), &[("path", &path.display().to_string())]),
            Err(e) => i18n::fmt(tl!("Couldn't save: {error}"), &[("error", &e.to_string())]),
        };
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.themed != Some(self.appearance) {
            theme::apply(&ctx, self.appearance);
            self.themed = Some(self.appearance);
        }
        let (undo, redo, save) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, Key::Y),
                i.consume_key(egui::Modifiers::COMMAND, Key::S),
            )
        });
        if undo {
            self.run("edit.undo", json!({}));
        }
        if redo {
            self.run("edit.redo", json!({}));
        }
        if save {
            self.save();
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
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
                if ui.button(tl!("Save")).clicked() {
                    self.save();
                }
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
    }

    fn sheet(&mut self, ui: &mut egui::Ui) {
        let start = self.session.project.start;
        let mut commands: Vec<(&'static str, Value)> = Vec::new();
        egui::Grid::new("sheet").striped(true).min_row_height(ROW_H).show(ui, |ui| {
            for h in ["ID", "Task Name", "Duration", "Start", "Finish", "Predecessors", ""] {
                ui.label(RichText::new(tl!(h)).strong());
            }
            ui.end_row();
            let p = &self.session.project;
            let rows: Vec<_> = p.tasks.iter().map(|t| (t.id, t.name.clone(), t.duration, p.preds_text_rows(&t.preds))).collect();
            // The ID column is the row number; commands get Unique IDs.
            for (row, (id, name, duration, preds)) in (1..).zip(rows) {
                let dates = self.session.schedule.as_ref().ok().and_then(|s| s.tasks.get(&id).copied());
                ui.label(row.to_string());
                if let Some(c) = self.cell(ui, id, Field::Name, &name, 220.0) {
                    commands.push(("task.rename", json!({"id": id.0, "name": c})));
                }
                if let Some(c) = self.cell(ui, id, Field::Duration, &format!("{duration}d"), 60.0) {
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
                gantt_bar(ui, start, dates, duration == 0);
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

fn gantt_bar(ui: &mut egui::Ui, start: Date, dates: Option<projectcraft_engine::model::TaskDates>, milestone: bool) {
    let Some(d) = dates else {
        ui.label("");
        return;
    };
    // Calendar days, so weekends show as gaps.
    let x0 = d.start.days_since(start) as f32 * DAY_W;
    let x1 = (d.finish.days_since(start) + 1) as f32 * DAY_W;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(x1.max(x0 + DAY_W) + DAY_W, ROW_H), Sense::hover());
    let t = Tokens::get(ui.ctx());
    let colour = if d.critical { t.bar_critical } else { t.bar };
    let p = ui.painter();
    if milestone {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colours live in `theme.rs` only: every other source file takes them from `Tokens`.
    #[test]
    fn no_colours_outside_the_token_set() {
        for (file, src) in [("lib.rs", include_str!("lib.rs")), ("i18n.rs", include_str!("i18n.rs"))] {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for needle in ["Color32", "from_rgb", "from_gray", "from_black_alpha", "from_white_alpha"] {
                assert!(!code.contains(needle), "{file} names a colour ({needle}); add a token in theme.rs");
            }
        }
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
}
