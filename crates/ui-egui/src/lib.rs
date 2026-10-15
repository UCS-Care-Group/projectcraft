//! ProjectCraft front end: the Gantt Chart view, a task sheet with its bars on the right.
//!
//! The UI is thin: it reads [`Session`] and changes it only through [`Session::run`].
//! ponytail: one view, no ribbon, theme tokens or translation yet (see ROADMAP.md, M1).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use egui::{Color32, Key, RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{TaskId, preds_text};
use projectcraft_engine::{Session, time::Date};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Duration,
    Preds,
}

pub struct App {
    pub session: Session,
    /// Where Save writes, if anywhere.
    pub path: Option<std::path::PathBuf>,
    /// The cell being typed in and its text; committed as one command when focus leaves it.
    editing: Option<(TaskId, Field, String)>,
    status: String,
}

const ROW_H: f32 = 22.0;
const DAY_W: f32 = 18.0;

impl App {
    pub fn new(session: Session, path: Option<std::path::PathBuf>) -> Self {
        App { session, path, editing: None, status: String::new() }
    }

    fn run(&mut self, id: &str, params: Value) {
        self.status = match self.session.run(id, &params) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        };
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.status = "Start ProjectCraft with a file name to save (projectcraft plan.projectcraft)".into();
            return;
        };
        self.status = match self.session.save_json().map(|t| std::fs::write(&path, t)) {
            Ok(Ok(())) => format!("Saved {}", path.display()),
            Ok(Err(e)) => format!("Couldn't save: {e}"),
            Err(e) => format!("Couldn't save: {e}"),
        };
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
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
                if ui.button("New Task").clicked() {
                    self.run("task.add", json!({}));
                }
                if ui.add_enabled(self.session.can_undo(), egui::Button::new("Undo")).clicked() {
                    self.run("edit.undo", json!({}));
                }
                if ui.add_enabled(self.session.can_redo(), egui::Button::new("Redo")).clicked() {
                    self.run("edit.redo", json!({}));
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            let text = match &self.session.schedule {
                Err(e) => format!("Can't schedule: {e}"),
                Ok(s) if self.status.is_empty() => format!("Start {} · Finish {}", self.session.project.start, s.finish),
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
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            let rows: Vec<_> = self.session.project.tasks.iter().map(|t| (t.id, t.name.clone(), t.duration, preds_text(&t.preds))).collect();
            for (id, name, duration, preds) in rows {
                let dates = self.session.schedule.as_ref().ok().and_then(|s| s.tasks.get(&id).copied());
                ui.label(id.to_string());
                if let Some(c) = self.cell(ui, id, Field::Name, &name, 220.0) {
                    commands.push(("task.rename", json!({"id": id.0, "name": c})));
                }
                if let Some(c) = self.cell(ui, id, Field::Duration, &format!("{duration}d"), 60.0) {
                    match c.trim().trim_end_matches(['d', 'D']).trim().parse::<i64>() {
                        Ok(d) => commands.push(("task.setDuration", json!({"id": id.0, "duration": d}))),
                        Err(_) => self.status = format!("Not a duration: {c}"),
                    }
                }
                ui.label(dates.map_or_else(String::new, |d| d.start.to_string()));
                ui.label(dates.map_or_else(String::new, |d| d.finish.to_string()));
                if let Some(c) = self.cell(ui, id, Field::Preds, &preds, 100.0) {
                    commands.push(("task.setPredecessors", json!({"id": id.0, "text": c})));
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
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width));
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
    let colour = if d.critical { Color32::from_rgb(0xc8, 0x3c, 0x3c) } else { Color32::from_rgb(0x3c, 0x78, 0xc8) };
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
