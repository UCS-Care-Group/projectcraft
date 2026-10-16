//! Assignments in the Gantt Chart: the Work column's text, the Resource Names column's status
//! message, and the Assign Resources dialog (Alt+F10, or the toolbar), as Microsoft Project's.
//!
//! The dialog acts on every selected task, as Microsoft Project's does, and lists every resource:
//! a tick where all of them have it, a dash where only some do. Assign adds it to the tasks
//! without it (with the units typed, or the default), Remove takes it off those with it, and
//! typing new units into a ticked row and pressing Enter changes them on all. The units and work
//! shown are the focused task's. Each is one `assignment.*` command over the whole selection, so
//! one undo step; a refused one (units that don't read) says why and changes nothing.

use std::collections::BTreeMap;

use egui::{Key, RichText};
use projectcraft_engine::model::{
    Assignment, AssignmentWork, DurationUnit, Project, ResourceId, ResourceType, TaskId, UnitsFormat, assignment_work, default_units, duration_text,
    quantity_text,
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
    match assignment_work(a, duration, &project.options) {
        AssignmentWork::Minutes(m) => work_text(m, project),
        AssignmentWork::Quantity(q) if label.is_empty() => quantity_text(q),
        AssignmentWork::Quantity(q) => format!("{} {label}", quantity_text(q)),
        AssignmentWork::None => match a.units {
            projectcraft_engine::model::Units::Cost(m) => project.currency.format(m),
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

/// The Cost cell's tooltip: each assignment's cost, the fixed cost, and for a summary what its
/// subtasks add (`Ann: £1,000.00`).
pub(crate) fn cost_lines(project: &Project, task: TaskId) -> String {
    let money = |m: i64| project.currency.format(m);
    let mut lines: Vec<String> = project
        .assignments_of(task)
        .map(|a| {
            let name = project.resource(a.resource).map(|r| r.name.clone()).unwrap_or_default();
            format!("{name}: {}", money(project.assignment_cost(a)))
        })
        .collect();
    let fixed = project.task(task).map_or(0, |t| t.fixed_cost);
    if fixed != 0 {
        lines.push(format!("{}: {}", tl!("Fixed cost"), money(fixed)));
    }
    let own = lines.len();
    let total = project.cost_by_task().get(&task).copied().unwrap_or(0);
    let mine = project.assignments_of(task).fold(fixed, |sum, a| sum.saturating_add(project.assignment_cost(a)));
    if total != mine {
        lines.push(format!("{}: {}", tl!("Subtasks"), money(total.saturating_sub(mine))));
    }
    if lines.is_empty() {
        return tl!("No costs").to_owned();
    }
    if own < lines.len() || lines.len() > 1 {
        lines.push(format!("{}: {}", tl!("Total"), money(total)));
    }
    lines.join("\n")
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

/// The dialog's own state: the focused task, every selected task it assigns to (the focused one
/// among them, in row order), and the units typed into each row.
pub(crate) struct AssignForm {
    pub task: TaskId,
    pub tasks: Vec<TaskId>,
    pub units: BTreeMap<ResourceId, String>,
}

impl AssignForm {
    /// Each row the focused task has shows its units; the others start empty (the default).
    fn read(project: &Project, task: TaskId, tasks: Vec<TaskId>) -> AssignForm {
        let units = project.assignments_of(task).map(|a| (a.resource, project.units_text_full(a))).collect();
        AssignForm { task, tasks, units }
    }

    /// The params naming the tasks and the resource: `task` for one, `tasks` for a selection.
    fn target(&self, resource: ResourceId) -> Value {
        match self.tasks.as_slice() {
            [one] => json!({"task": one.0, "resource": resource.0}),
            many => json!({"tasks": many.iter().map(|t| t.0).collect::<Vec<_>>(), "resource": resource.0}),
        }
    }
}

/// A resource's default units, as the empty units field's hint: `100%`, `1 boxes`, `£0.00`.
fn default_hint(project: &Project, id: ResourceId) -> String {
    let Ok(r) = project.resource(id) else { return String::new() };
    let a = Assignment { task: TaskId(0), resource: id, units: default_units(r.kind, r.max_units), work: 0, duration: 0 };
    project.units_text_full(&a)
}

/// How many of `tasks` the resource is assigned to.
fn held_by(project: &Project, tasks: &[TaskId], resource: ResourceId) -> usize {
    tasks.iter().filter(|t| project.assignments_of(**t).any(|a| a.resource == resource)).count()
}

impl App {
    /// Opens Assign Resources for the selected tasks, showing the focused one's units; says so
    /// when no task has the focus.
    pub fn open_assign_resources(&mut self) {
        let p = &self.session.project;
        match self.sel.cursor.filter(|id| p.task(*id).is_ok()) {
            Some(task) => {
                let mut tasks: Vec<TaskId> = self.sel.ids(&self.order()).into_iter().filter(|t| p.task(*t).is_ok()).collect();
                if !tasks.contains(&task) {
                    tasks = vec![task];
                }
                self.assign = Some(AssignForm::read(p, task, tasks));
                self.status.clear();
            }
            None => self.status = tl!("Select a task to assign resources to.").to_owned(),
        }
    }

    pub(crate) fn assign_resources(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.assign.take() else { return };
        let p = self.session.project.clone();
        let Ok(task) = p.task(form.task) else { return };
        // Undo can take selected tasks away; the focused one is still there (or this closed).
        form.tasks.retain(|t| p.task(*t).is_ok());
        if !form.tasks.contains(&form.task) {
            form.tasks = vec![form.task];
        }
        let selected = form.tasks.len();
        let error = self.status.clone();
        let t = Tokens::get(ctx);
        let mut command: Option<(&'static str, Value)> = None;
        let mut close = false;
        let assigned: Vec<Assignment> = p.assignments_of(form.task).copied().collect();
        // The focused task's resources first, then those other selected tasks have, then the rest.
        let held: BTreeMap<ResourceId, usize> = p.resources.iter().map(|r| (r.id, held_by(&p, &form.tasks, r.id))).collect();
        let count_of = |id: ResourceId| held.get(&id).copied().unwrap_or(0);
        let others: Vec<ResourceId> = p.resources.iter().map(|r| r.id).filter(|id| !assigned.iter().any(|a| a.resource == *id)).collect();
        let rows: Vec<ResourceId> = assigned
            .iter()
            .map(|a| a.resource)
            .chain(others.iter().copied().filter(|id| count_of(*id) > 0))
            .chain(others.iter().copied().filter(|id| count_of(*id) == 0))
            .collect();
        let modal = egui::Modal::new(egui::Id::new("assign_resources")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.heading(tl!("Assign Resources"));
            if selected > 1 {
                let count = selected.to_string();
                ui.label(crate::i18n::fmt(tl!("{count} tasks selected: Assign and Remove act on all of them."), &[("count", &count)]));
                ui.label(RichText::new(crate::i18n::fmt(tl!("Units and work shown are for {name}."), &[("name", &task.name)])).color(t.text_dim));
            } else {
                ui.label(RichText::new(crate::i18n::fmt(tl!("Task: {name}"), &[("name", &task.name)])).color(t.text_dim));
            }
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
                        let count = count_of(*id);
                        let all = count == selected;
                        // Drawn: a tick where every selected task has the resource, a dash where
                        // only some do.
                        let (rect, mark) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        let pt = |x: f32, y: f32| rect.left_top() + egui::vec2(x, y);
                        let stroke = egui::Stroke::new(2.0, t.accent);
                        if all {
                            ui.painter().add(egui::Shape::line(vec![pt(2.0, 8.5), pt(6.5, 13.0), pt(14.0, 3.5)], stroke));
                        } else if count > 0 {
                            ui.painter().line_segment([pt(3.0, 8.0), pt(13.0, 8.0)], stroke);
                            let (n, total) = (count.to_string(), selected.to_string());
                            mark.on_hover_text(crate::i18n::fmt(
                                tl!("Assigned to {count} of the {total} selected tasks"),
                                &[("count", &n), ("total", &total)],
                            ));
                        }
                        ui.label(&r.name);
                        ui.label(RichText::new(tl!(r.kind.name())).color(t.text_dim));
                        let text = form.units.entry(*id).or_default();
                        let hint = default_hint(&p, *id);
                        let edit = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(110.0).min_size(egui::vec2(110.0, 0.0)));
                        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.label(a.map(|a| assignment_work_text(&p, a)).unwrap_or_default());
                        let typed = text.trim().to_owned();
                        ui.horizontal(|ui| match a.filter(|_| all) {
                            // On every selected task: new units go to all of them.
                            Some(a) => {
                                if (entered || edit.lost_focus()) && typed != p.units_text_full(a) && !typed.is_empty() {
                                    let mut params = form.target(*id);
                                    params["units"] = json!(typed);
                                    command = Some(("assignment.setUnits", params));
                                }
                                if ui.button(tl!("Remove")).clicked() {
                                    command = Some(("assignment.remove", form.target(*id)));
                                }
                            }
                            // On none or some: Assign gives it to the rest, Remove (when some
                            // have it) takes it off them.
                            None => {
                                if ui.button(tl!("Assign")).clicked() || entered {
                                    let mut params = form.target(*id);
                                    if !typed.is_empty() {
                                        params["units"] = json!(typed);
                                    }
                                    command = Some(("assignment.add", params));
                                }
                                if count > 0 && ui.button(tl!("Remove")).clicked() {
                                    command = Some(("assignment.remove", form.target(*id)));
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            let decimal = p.options.units_format == UnitsFormat::Decimal;
            let hint = match (p.resources.iter().any(|r| r.kind == ResourceType::Material), decimal) {
                (true, false) => tl!("Units: a percentage for people and equipment (50%); for materials a quantity (5 tons) or a rate (2 tons/d)."),
                (true, true) => {
                    tl!("Units: a decimal for people and equipment (0.5, or 50%); for materials a quantity (5 tons) or a rate (2 tons/d).")
                }
                (false, false) => tl!("Units: the share of each resource's working time (50% is half its day)."),
                (false, true) => tl!("Units: the share of each resource's working time (0.5 is half its day; 50% works too)."),
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
            form = AssignForm::read(&self.session.project, form.task, form.tasks);
        }
        if !(close || modal.should_close()) {
            self.assign = Some(form);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Key, Modifiers};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use projectcraft_engine::model::{ResourceId, TaskId};
    use serde_json::json;

    use crate::App;

    /// With several tasks selected, a summary among them, Assign, new units and Remove each act on
    /// all of them as one command, so one undo step each; the dialog says the selection is mixed.
    #[test]
    fn assign_resources_acts_on_every_selected_task() {
        let mut s = projectcraft_engine::sample();
        // Order furniture under Survey the new floor: the survey becomes a summary task.
        s.run("task.indent", &json!({"ids": [3]})).unwrap();
        let before = s.project.clone();
        let mut h = Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), App::new(s, None));
        h.run_steps(3);
        h.get_by_label("Survey the new floor").click();
        h.run_steps(3);
        h.get_by_label("Furniture delivered").click_modifiers(Modifiers::SHIFT);
        h.run_steps(3);
        h.key_press_modifiers(Modifiers::ALT, Key::F10);
        h.run_steps(3);
        let form = h.state().assign.as_ref().unwrap();
        assert_eq!((form.task, form.tasks.clone()), (TaskId(4), vec![TaskId(2), TaskId(3), TaskId(4)]));
        assert!(h.query_by_label_contains("3 tasks selected").is_some());
        assert!(h.query_by_label_contains("shown are for Furniture delivered").is_some());
        // Priya is on the summary and Order furniture, not on Furniture delivered: Assign gives
        // her to the one without, the others keep their units.
        h.get_all_by_label("Assign").next().unwrap().click();
        h.run_steps(3);
        let p = &h.state().session.project;
        assert_eq!(p.resource_names(TaskId(2)), "Priya Shah[50%], Tom Okafor[50%]");
        assert_eq!(p.resource_names(TaskId(3)), "Priya Shah");
        assert_eq!(p.resource_names(TaskId(4)), "Priya Shah");
        // Now on all three: new units, typed into her row and entered, go to all of them.
        h.state_mut().assign.as_mut().unwrap().units.insert(ResourceId(1), "25%".into());
        h.run_steps(2);
        h.get_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().focus();
        h.run_steps(1);
        h.key_press(Key::Enter);
        h.run_steps(3);
        for t in [2, 3, 4] {
            assert!(h.state().session.project.resource_names(TaskId(t)).starts_with("Priya Shah[25%]"), "task {t}");
        }
        h.get_all_by_label("Remove").next().unwrap().click();
        h.run_steps(3);
        let p = &h.state().session.project;
        let names: Vec<String> = [2, 3, 4].iter().map(|t| p.resource_names(TaskId(*t))).collect();
        assert_eq!(names, ["Tom Okafor[50%]", "", ""]);
        h.get_by_label("Close").click();
        h.run_steps(2);
        for _ in 0..3 {
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.run_steps(2);
        }
        assert_eq!(h.state().session.project, before, "three commands, three undo steps");
    }
}
