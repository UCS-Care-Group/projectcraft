//! The Resource Sheet view: one row per resource, as Microsoft Project's. It works as the task
//! sheet does: click, Shift-click and Ctrl-click select rows; the arrows move the focused cell;
//! typing or F2 edits it, Enter commits and moves down, Escape cancels; Insert adds a resource
//! above, Delete deletes the selected ones. Every change is a `resource.*` command.

use egui::{Key, Modifiers, RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{
    self, AccrueAt, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS, Resource, ResourceId, ResourceType, format_money, parse_money, parse_units,
    units_text,
};
use serde_json::{Value, json};

use crate::selection::Selection;
use crate::theme::Tokens;
use crate::{App, ID_W, ROW_H, SheetKeys};

/// A Resource Sheet column.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum RField {
    #[default]
    Name,
    Type,
    Material,
    Initials,
    Group,
    MaxUnits,
    StandardRate,
    OvertimeRate,
    CostPerUse,
    AccrueAt,
}

/// The columns, left to right: heading, width, and whether the text sits on the right (amounts).
const COLUMNS: [(RField, &str, f32, bool); 10] = [
    (RField::Name, "Resource Name", 180.0, false),
    (RField::Type, "Type", 72.0, false),
    (RField::Material, "Material", 72.0, false),
    (RField::Initials, "Initials", 56.0, false),
    (RField::Group, "Group", 100.0, false),
    (RField::MaxUnits, "Max. Units", 76.0, true),
    (RField::StandardRate, "Std. Rate", 96.0, true),
    (RField::OvertimeRate, "Ovt. Rate", 96.0, true),
    (RField::CostPerUse, "Cost/Use", 88.0, true),
    (RField::AccrueAt, "Accrue At", 76.0, false),
];

/// The Resource Sheet's own state: its selection, focused column and the cell being typed in.
#[derive(Default)]
pub(crate) struct ResourceSheet {
    pub sel: Selection<ResourceId>,
    pub field: RField,
    pub editing: Option<(ResourceId, RField, String)>,
    focus_editor: bool,
    down_after_edit: bool,
    clicked: Option<(ResourceId, Option<RField>, Modifiers, bool)>,
    /// A type change waiting for the owner to agree: it drops the resource's assignments.
    pub retype: Option<(ResourceId, Value)>,
}

/// A money amount in the project currency. ponytail: the default symbol until projects carry a currency.
fn money(minor: i64) -> String {
    format_money(minor, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS)
}

/// A cell's text; `None` where the field doesn't apply to the resource's type (Max Units of a
/// material), which shows empty and can't be edited.
pub(crate) fn cell_text(r: &Resource, field: RField) -> Option<String> {
    let work = r.kind == ResourceType::Work;
    let per_hour = |n: i64| format!("{}{}", money(n), tl!("/hr"));
    Some(match field {
        RField::Name => r.name.clone(),
        RField::Type => tl!(r.kind.name()).to_owned(),
        RField::Material if r.kind == ResourceType::Material => r.material_label.clone(),
        RField::Initials => r.initials.clone(),
        RField::Group => r.group.clone(),
        RField::MaxUnits if work => units_text(r.max_units),
        RField::StandardRate if work => per_hour(r.standard_rate),
        RField::StandardRate if r.kind == ResourceType::Material => money(r.standard_rate),
        RField::OvertimeRate if work => per_hour(r.overtime_rate),
        RField::CostPerUse if r.kind != ResourceType::Cost => money(r.cost_per_use),
        RField::AccrueAt => tl!(r.accrue_at.name()).to_owned(),
        _ => return None,
    })
}

/// The command a cell's new text issues, or why the text can't be read.
pub(crate) fn typed(id: ResourceId, field: RField, text: &str) -> Result<(&'static str, Value), String> {
    let set = |k: &str, v: Value| Ok(("resource.set", json!({"id": id.0, k: v})));
    let amount = |t: &str| parse_money(t, DEFAULT_CURRENCY_SYMBOL, DEFAULT_MINOR_DIGITS).map_err(|e| e.to_string());
    match field {
        RField::Name => Ok(("resource.rename", json!({"id": id.0, "name": text}))),
        RField::Type => set("type", json!(ResourceType::from_name(text).map_err(|e| e.to_string())?.name())),
        RField::Material => set("materialLabel", json!(text)),
        RField::Initials => set("initials", json!(text)),
        RField::Group => set("group", json!(text)),
        RField::MaxUnits => {
            let u = parse_units(text).map_err(|e| e.to_string())?;
            // Exact: the percent as a decimal number (`37.50`), read back without a float.
            let percent = format!("{}.{:02}", u / model::UNITS_PER_PERCENT, u % model::UNITS_PER_PERCENT);
            set("maxUnits", serde_json::from_str::<Value>(&percent).map_err(|e| e.to_string())?)
        }
        RField::StandardRate => set("standardRate", json!(amount(text)?)),
        RField::OvertimeRate => set("overtimeRate", json!(amount(text)?)),
        RField::CostPerUse => set("costPerUse", json!(amount(text)?)),
        RField::AccrueAt => set("accrueAt", json!(AccrueAt::from_name(text).map_err(|e| e.to_string())?.name())),
    }
}

impl App {
    /// Every resource's Unique ID, top to bottom.
    pub(crate) fn resource_order(&self) -> Vec<ResourceId> {
        self.session.project.resources.iter().map(|r| r.id).collect()
    }

    /// The sheet keys, read by the caller, acted on in the Resource Sheet.
    pub(crate) fn resource_keys(&mut self, k: SheetKeys) {
        let order = self.resource_order();
        let rs = &mut self.resources;
        if k.rows != 0 {
            rs.sel.step(&order, k.rows, k.extend);
        }
        if k.cols != 0 {
            let at = COLUMNS.iter().position(|c| c.0 == rs.field).unwrap_or(0);
            rs.field = COLUMNS.get(at.saturating_add_signed(k.cols)).map_or(rs.field, |c| c.0);
            if rs.sel.cursor.is_none() {
                rs.sel.step(&order, 0, false);
            }
        }
        if k.edit || !k.typed.trim().is_empty() {
            self.begin_resource_edit((!k.typed.is_empty()).then_some(k.typed));
        }
        if k.insert {
            self.insert_resource();
        }
        if k.delete {
            self.delete_resources();
        }
    }

    fn begin_resource_edit(&mut self, typed: Option<String>) {
        let rs = &mut self.resources;
        let Some(id) = rs.sel.cursor else { return };
        let Some(text) = self.session.project.resource(id).ok().and_then(|r| cell_text(r, rs.field)) else { return };
        rs.editing = Some((id, rs.field, typed.unwrap_or(text)));
        rs.focus_editor = true;
    }

    /// Insert (or New Resource): a resource above the focused row, or at the end with none,
    /// which takes the focus on its name.
    pub(crate) fn insert_resource(&mut self) {
        let next = ResourceId(self.session.project.next_resource_id.max(1));
        let params = match self.resources.sel.cursor {
            Some(c) => json!({"before": c.0}),
            None => json!({}),
        };
        if self.run("resource.add", params) && self.session.project.resource_row_of(next).is_some() {
            self.resources.sel.select_only(next);
            self.resources.field = RField::Name;
        }
    }

    /// Delete: the selected resources. The focus stays on the same row number.
    fn delete_resources(&mut self) {
        let order = self.resource_order();
        let ids = self.resources.sel.ids(&order);
        if ids.is_empty() {
            return;
        }
        let at = self.resources.sel.cursor.and_then(|c| order.iter().position(|r| *r == c));
        if self.run("resource.delete", json!({"ids": ids.iter().map(|r| r.0).collect::<Vec<_>>()})) {
            self.resources.sel.prune(&self.resource_order(), at);
        }
    }

    pub(crate) fn commit_resource(&mut self, id: ResourceId, field: RField, text: String) {
        let now = self.session.project.resource(id).ok().and_then(|r| cell_text(r, field));
        if now.as_deref() == Some(text.as_str()) {
            return;
        }
        match typed(id, field, &text) {
            // As Microsoft Project, a new type loses the resource's assignments: ask first.
            Ok((_, params)) if field == RField::Type && self.session.project.is_assigned(id) => {
                let new = params.get("type").and_then(Value::as_str).unwrap_or("");
                if self.session.project.resource(id).is_ok_and(|r| r.kind.name() != new) {
                    self.resources.retype = Some((id, params));
                }
            }
            Ok((cmd, params)) => {
                self.run(cmd, params);
            }
            Err(e) => self.status = e,
        }
    }

    /// The question before a type change that drops assignments: Change Type runs it, Cancel
    /// (or Escape) changes nothing.
    pub(crate) fn retype_prompt(&mut self, ctx: &egui::Context) {
        let Some((id, params)) = self.resources.retype.take() else { return };
        let Ok(r) = self.session.project.resource(id) else { return };
        let count = self.session.project.assignments.iter().filter(|a| a.resource == id).count();
        let new = params.get("type").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let t = Tokens::get(ctx);
        let (mut change, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("retype_resource")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(tl!("Change Resource Type"));
            ui.add_space(6.0);
            let text = crate::i18n::fmt(
                tl!("{name} is assigned to {count} task(s). Making it a {type} resource removes those assignments."),
                &[("name", &r.name), ("count", &count.to_string()), ("type", tl!(&new))],
            );
            ui.label(text);
            ui.label(RichText::new(tl!("You can undo this.")).color(t.text_dim));
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button(tl!("Cancel")).clicked();
                change = ui.button(tl!("Change Type")).clicked();
            });
        });
        if change {
            self.run("resource.set", params);
        } else if !(cancel || modal.should_close()) {
            self.resources.retype = Some((id, params));
        }
    }

    pub(crate) fn resource_sheet(&mut self, ui: &mut egui::Ui) {
        let order = self.resource_order();
        let t = Tokens::get(ui.ctx());
        if order.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new(tl!("No resources yet. Press Insert or New Resource to add one.")).color(t.text_dim));
            return;
        }
        let mut edits: Vec<(ResourceId, RField, String)> = Vec::new();
        let picked: Vec<bool> = order.iter().map(|id| self.resources.sel.is_selected(*id)).collect();
        let row_colour = move |row: usize, _: &egui::Style| match row.checked_sub(1).and_then(|r| picked.get(r)) {
            Some(true) => Some(t.row_selected),
            _ if row % 2 == 1 => Some(t.stripe),
            _ => None,
        };
        egui::Grid::new("resource_sheet").with_row_color(row_colour).min_row_height(ROW_H).min_col_width(0.0).show(ui, |ui| {
            ui.add_sized([ID_W, ROW_H], egui::Label::new(RichText::new(tl!("ID")).strong()));
            // Each heading sits over its column's text: amounts on the right.
            for (_, heading, width, right) in COLUMNS {
                let text = tl!(heading);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
                let font = egui::TextStyle::Body.resolve(ui.style());
                let colour = ui.visuals().strong_text_color();
                let painter = ui.painter().with_clip_rect(rect);
                if right {
                    painter.text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, text, font, colour);
                } else {
                    painter.text(rect.left_center() + Vec2::new(4.0, 0.0), egui::Align2::LEFT_CENTER, text, font, colour);
                }
            }
            ui.end_row();
            for (i, id) in order.iter().copied().enumerate() {
                let Ok(r) = self.session.project.resource(id).cloned() else { continue };
                let row_id = i.saturating_add(1).to_string();
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ID_W, ROW_H), Sense::click());
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &row_id, egui::TextStyle::Body.resolve(ui.style()), t.text);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &row_id));
                self.take_resource_click(ui, &resp, id, None);
                for (field, _, width, right) in COLUMNS {
                    if let Some(text) = self.resource_cell(ui, &r, field, width, right) {
                        edits.push((id, field, text));
                    }
                }
                ui.end_row();
            }
        });
        for (id, field, text) in edits {
            self.commit_resource(id, field, text);
        }
        if std::mem::take(&mut self.resources.down_after_edit) {
            self.resources.sel.step(&self.resource_order(), 1, false);
        }
        if let Some((id, field, mods, double)) = self.resources.clicked.take() {
            if let Some((eid, efield, text)) = self.resources.editing.take_if(|(e, f, _)| (*e, Some(*f)) != (id, field)) {
                self.commit_resource(eid, efield, text);
            }
            let order = self.resource_order();
            self.resources.sel.click(id, &order, mods.shift, mods.command);
            if let Some(f) = field {
                self.resources.field = f;
                if double {
                    self.begin_resource_edit(None);
                }
            }
        }
    }

    fn take_resource_click(&mut self, ui: &egui::Ui, r: &egui::Response, id: ResourceId, field: Option<RField>) {
        if r.has_focus() {
            r.surrender_focus();
        }
        if r.clicked() || r.double_clicked() {
            self.resources.clicked = Some((id, field, ui.input(|i| i.modifiers), r.double_clicked()));
        }
    }

    /// One cell: its text, outlined when focused; an editor while being edited, returning the new
    /// text once when editing ends (Escape cancels).
    fn resource_cell(&mut self, ui: &mut egui::Ui, r: &Resource, field: RField, width: f32, right: bool) -> Option<String> {
        if let Some((_, _, text)) = self.resources.editing.as_mut().filter(|(i, f, _)| *i == r.id && *f == field) {
            let edit = egui::TextEdit::singleline(text).desired_width(width).min_size(Vec2::new(width, 0.0));
            let edit = if right { edit.horizontal_align(egui::Align::RIGHT) } else { edit };
            let resp = ui.add(edit);
            let text = text.clone();
            if std::mem::take(&mut self.resources.focus_editor) {
                resp.request_focus();
                return None;
            }
            let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if enter || escape || resp.lost_focus() || !resp.has_focus() {
                self.resources.editing = None;
                resp.surrender_focus();
                self.resources.down_after_edit = enter;
                return (!escape).then_some(text);
            }
            return None;
        }
        let value = cell_text(r, field);
        let shown = value.clone().unwrap_or_default();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &shown));
        let t = Tokens::get(ui.ctx());
        let font = egui::TextStyle::Body.resolve(ui.style());
        let painter = ui.painter().with_clip_rect(rect);
        if right {
            painter.text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, &shown, font, t.text);
        } else {
            painter.text(rect.left_center() + Vec2::new(4.0, 0.0), egui::Align2::LEFT_CENTER, &shown, font, t.text);
        }
        if self.resources.sel.cursor == Some(r.id) && self.resources.field == field {
            ui.painter().rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
        let resp = if value.is_none() {
            let kind = tl!(r.kind.name()).to_lowercase();
            resp.on_hover_text(crate::i18n::fmt(tl!("Doesn't apply to a {type} resource"), &[("type", &kind)]))
        } else {
            resp
        };
        self.take_resource_click(ui, &resp, r.id, Some(field));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_cells_become_resource_commands() {
        let mut s = projectcraft_engine::sample();
        let id = ResourceId(1);
        for (field, text) in [
            (RField::MaxUnits, "37.5%"),
            (RField::StandardRate, "£1,250.50/hr"),
            (RField::OvertimeRate, "60"),
            (RField::CostPerUse, "£0.99"),
            (RField::Type, "w"),
            (RField::AccrueAt, "start"),
            (RField::Group, "Site"),
            (RField::Initials, "PS"),
            (RField::Name, "Priya S."),
        ] {
            let (cmd, params) = typed(id, field, text).unwrap();
            s.run(cmd, &params).unwrap();
        }
        let r = s.project.resource(id).unwrap();
        assert_eq!((r.max_units, r.standard_rate, r.overtime_rate, r.cost_per_use), (3_750, 125_050, 6_000, 99));
        assert_eq!((r.accrue_at, r.group.as_str(), r.initials.as_str(), r.name.as_str()), (AccrueAt::Start, "Site", "PS", "Priya S."));
        assert_eq!(cell_text(r, RField::MaxUnits).as_deref(), Some("37.5%"));
        assert_eq!(cell_text(r, RField::StandardRate).as_deref(), Some("£1,250.50/hr"));
        assert_eq!(cell_text(r, RField::Material), None, "a work resource has no material label");
        for (field, text) in [(RField::MaxUnits, "lots"), (RField::StandardRate, "-5"), (RField::Type, "Person"), (RField::AccrueAt, "")] {
            assert!(typed(id, field, text).is_err(), "{text}");
        }
        let boxes = s.project.resource(ResourceId(4)).unwrap();
        assert_eq!(cell_text(boxes, RField::StandardRate).as_deref(), Some("£2.50"));
        assert_eq!(cell_text(boxes, RField::MaxUnits), None);
    }
}
