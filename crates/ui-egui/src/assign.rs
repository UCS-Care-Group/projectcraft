//! Assignments in the Gantt Chart: the Work column's text, the Resource Names column's status
//! message, and the Assign Resources dialog (Alt+F10, or the toolbar), as Microsoft Project's.
//!
//! The dialog lists every resource for the focused task, the assigned ones first with a tick.
//! Assign adds one (with the units typed, or the default), Remove takes it off, and typing new
//! units into an assigned row and pressing Enter changes them. Each is one `assignment.*` command,
//! so one undo step; a refused one (units that don't read) says why and changes nothing.

use std::collections::BTreeMap;

use egui::{Key, RichText};
use projectcraft_engine::model::{
    Assignment, AssignmentWork, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS, DurationUnit, Project, ResourceId, ResourceType, TaskId,
    assignment_work, default_units, duration_text, format_money, quantity_text,
};
use serde_json::{Value, json};

use crate::App;
use crate::theme::Tokens;

/// Working minutes as the Work column shows them: hours, to two decimals (`20h`, `7.5h`).
pub(crate) fn work_text(minutes: i64, project: &Project) -> String {
    duration_text(minutes, DurationUnit::Hours, &project.options)
}

/// What one assignment uses over its task: hours of work, a material's quantity with its label,
/// or a cost resource's amount.
pub(crate) fn assignment_work_text(project: &Project, a: &Assignment) -> String {
    let duration = project.task(a.task).map_or(0, |t| t.duration);
    let label = project.resource(a.resource).map(|r| r.material_label.clone()).unwrap_or_default();
    match assignment_work(a.units, duration, &project.options) {
        AssignmentWork::Minutes(m) => work_text(m, project),
        AssignmentWork::Quantity(q) if label.is_empty() => quantity_text(q),
        AssignmentWork::Quantity(q) => format!("{} {label}", quantity_text(q)),
        AssignmentWork::None => match a.units {
            projectcraft_engine::model::Units::Cost(m) => format_money(m, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS),
            _ => String::new(),
        },
    }
}

/// The Work cell's tooltip: each assignment's work, one per line (`Ann: 20h`).
pub(crate) fn work_lines(project: &Project, task: TaskId) -> String {
    let lines: Vec<String> = project
        .assignments_of(task)
        .map(|a| {
            let name = project.resource(a.resource).map(|r| r.name.clone()).unwrap_or_default();
            format!("{name}: {}", assignment_work_text(project, a))
        })
        .collect();
    if lines.is_empty() { tl!("No resources assigned").to_owned() } else { lines.join("\n") }
}

/// The status line after Resource Names made new resources: their names, or nothing.
pub(crate) fn created_message(before: &Project, after: &Project) -> Option<String> {
    let made: Vec<&str> = after.resources.iter().filter(|r| before.resource(r.id).is_err()).map(|r| r.name.as_str()).collect();
    match made.as_slice() {
        [] => None,
        [one] => Some(crate::i18n::fmt(tl!("Added {name} to the Resource Sheet as a work resource (100% Max Units)."), &[("name", one)])),
        many => {
            Some(crate::i18n::fmt(tl!("Added {names} to the Resource Sheet as work resources (100% Max Units)."), &[("names", &many.join(", "))]))
        }
    }
}

/// The dialog's own state: the task it assigns to, and the units typed into each row.
pub(crate) struct AssignForm {
    pub task: TaskId,
    pub units: BTreeMap<ResourceId, String>,
}

impl AssignForm {
    /// Each assigned row shows its units; the others start empty (the default).
    fn read(project: &Project, task: TaskId) -> AssignForm {
        let units = project.assignments_of(task).map(|a| (a.resource, project.units_text_full(a))).collect();
        AssignForm { task, units }
    }
}

/// A resource's default units, as the empty units field's hint: `100%`, `1 boxes`, `£0.00`.
fn default_hint(project: &Project, id: ResourceId) -> String {
    let Ok(r) = project.resource(id) else { return String::new() };
    let a = Assignment { task: TaskId(0), resource: id, units: default_units(r.kind, r.max_units) };
    project.units_text_full(&a)
}

impl App {
    /// Opens Assign Resources for the focused task; says so when no task has the focus.
    pub fn open_assign_resources(&mut self) {
        match self.sel.cursor.filter(|id| self.session.project.task(*id).is_ok()) {
            Some(task) => {
                self.assign = Some(AssignForm::read(&self.session.project, task));
                self.status.clear();
            }
            None => self.status = tl!("Select a task to assign resources to.").to_owned(),
        }
    }

    pub(crate) fn assign_resources(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.assign.take() else { return };
        let p = self.session.project.clone();
        let Ok(task) = p.task(form.task) else { return };
        let error = self.status.clone();
        let t = Tokens::get(ctx);
        let mut command: Option<(&'static str, Value)> = None;
        let mut close = false;
        let assigned: Vec<Assignment> = p.assignments_of(form.task).copied().collect();
        let others = p.resources.iter().filter(|r| !assigned.iter().any(|a| a.resource == r.id)).map(|r| r.id);
        let rows: Vec<ResourceId> = assigned.iter().map(|a| a.resource).chain(others).collect();
        let modal = egui::Modal::new(egui::Id::new("assign_resources")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.heading(tl!("Assign Resources"));
            ui.label(RichText::new(crate::i18n::fmt(tl!("Task: {name}"), &[("name", &task.name)])).color(t.text_dim));
            ui.add_space(10.0);
            if rows.is_empty() {
                ui.label(
                    RichText::new(tl!("No resources yet. Add them in the Resource Sheet, or type a name in the Resource Names column."))
                        .color(t.text_dim),
                );
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("assign_rows").striped(true).num_columns(6).spacing([12.0, 4.0]).show(ui, |ui| {
                    for h in ["", "Resource Name", "Type", "Units", "Work", ""] {
                        ui.label(RichText::new(tl!(h)).strong());
                    }
                    ui.end_row();
                    for id in &rows {
                        let Ok(r) = p.resource(*id) else { continue };
                        let a = assigned.iter().find(|a| a.resource == *id);
                        // A tick, drawn as text, on the assigned rows.
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        if a.is_some() {
                            let pt = |x: f32, y: f32| rect.left_top() + egui::vec2(x, y);
                            ui.painter().add(egui::Shape::line(vec![pt(2.0, 8.5), pt(6.5, 13.0), pt(14.0, 3.5)], egui::Stroke::new(2.0, t.accent)));
                        }
                        ui.label(&r.name);
                        ui.label(RichText::new(tl!(r.kind.name())).color(t.text_dim));
                        let text = form.units.entry(*id).or_default();
                        let hint = default_hint(&p, *id);
                        let edit = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(110.0).min_size(egui::vec2(110.0, 0.0)));
                        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.label(a.map(|a| assignment_work_text(&p, a)).unwrap_or_default());
                        match a {
                            Some(a) => {
                                let changed = text.trim() != p.units_text_full(a);
                                if (entered || edit.lost_focus()) && changed && !text.trim().is_empty() {
                                    command = Some(("assignment.setUnits", json!({"task": form.task.0, "resource": id.0, "units": text.trim()})));
                                }
                                if ui.button(tl!("Remove")).clicked() {
                                    command = Some(("assignment.remove", json!({"task": form.task.0, "resource": id.0})));
                                }
                            }
                            None => {
                                if ui.button(tl!("Assign")).clicked() || entered {
                                    let mut params = json!({"task": form.task.0, "resource": id.0});
                                    if !text.trim().is_empty() {
                                        params["units"] = json!(text.trim());
                                    }
                                    command = Some(("assignment.add", params));
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            let hint = match p.resources.iter().any(|r| r.kind == ResourceType::Material) {
                true => tl!("Units: a percentage for people and equipment (50%); for materials a quantity (5 tons) or a rate (2 tons/d)."),
                false => tl!("Units: the share of each resource's working time (50% is half its day)."),
            };
            ui.label(RichText::new(hint).color(t.text_dim));
            if !error.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(error.as_str()).color(t.warning));
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        if let Some((cmd, params)) = command
            && self.run(cmd, params)
        {
            // Show the assignments as they now are.
            form = AssignForm::read(&self.session.project, form.task);
        }
        if !(close || modal.should_close()) {
            self.assign = Some(form);
        }
    }
}
