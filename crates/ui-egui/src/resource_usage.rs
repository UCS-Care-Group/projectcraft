//! The Resource Usage view, as Microsoft Project's: each resource with its assignments under it,
//! their total work, and their work on the timescale, day by day (or week by week, month by month
//! as the timescale is zoomed out). A work resource's cells are red where its work is over its
//! Max Units on its calendar, and its name is red when that happens on any day. Materials and
//! cost resources are listed with what their assignments use, with nothing on the timescale.
//!
//! It only shows: the usage comes from the scheduler ([`projectcraft_engine::Session::usage`]).

use std::collections::{BTreeMap, BTreeSet};

use egui::{RichText, Sense, Stroke, Vec2};
use projectcraft_engine::model::{Date, Project, ResourceType, Usage};

use crate::theme::Tokens;
use crate::{App, ID_W, INDENT_W, ROW_H, assign, timescale};

const NAME_W: f32 = 220.0;
const WORK_W: f32 = 80.0;

/// The timescale presets' day widths in this view: wide enough for "7.5h" in a day.
pub(crate) fn day_w(preset: timescale::Preset) -> f32 {
    match preset {
        timescale::Preset::Days => 36.0,
        timescale::Preset::Weeks => 9.0,
        timescale::Preset::Months => 3.0,
    }
}

/// One row: a resource (with its overallocated days) or one of its assignments.
struct Row<'a> {
    name: &'a str,
    work: String,
    resource: bool,
    over_anywhere: bool,
    days: Option<&'a BTreeMap<Date, i64>>,
    over: Option<&'a BTreeSet<Date>>,
}

/// The rows, top to bottom: each resource in Resource Sheet order, then its assignments in the
/// project's order.
fn rows<'a>(p: &'a Project, usage: Option<&'a Usage>) -> Vec<Row<'a>> {
    let mut out = Vec::new();
    for r in &p.resources {
        let u = usage.and_then(|u| u.resource(r.id));
        let total = match (r.kind, u) {
            (ResourceType::Work, Some(u)) => assign::work_text(u.work(), p),
            (ResourceType::Work, None) => assign::work_text(0, p),
            _ => String::new(),
        };
        out.push(Row {
            name: &r.name,
            work: total,
            resource: true,
            over_anywhere: u.is_some_and(|u| u.is_overallocated()),
            days: u.map(|u| &u.days),
            over: u.map(|u| &u.over),
        });
        for a in p.assignments.iter().filter(|a| a.resource == r.id) {
            let name = p.task(a.task).map_or("", |t| t.name.as_str());
            let days = u.and_then(|u| u.assignments.iter().find(|(t, _)| *t == a.task).map(|(_, d)| d));
            out.push(Row { name, work: assign::assignment_work_text(p, a), resource: false, over_anywhere: false, days, over: None });
        }
    }
    out
}

impl App {
    pub(crate) fn resource_usage(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let p = &self.session.project;
        if p.resources.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new(tl!("No resources yet. Add them in the Resource Sheet.")).color(t.text_dim));
            return;
        }
        let usage = self.session.usage();
        if let Err(e) = &usage {
            ui.label(RichText::new(e.to_string()).color(t.text_dim));
        }
        let rows = rows(p, usage.ok());
        let scale = timescale::Scale::for_project(p.start, self.usage_day_w);
        let finish = self.session.schedule.as_ref().map_or(p.start, |s| s.finish);
        let chart_w = timescale::chart_width(scale, finish, ui.clip_rect().width());
        egui::Grid::new("resource_usage").striped(true).min_row_height(ROW_H).min_col_width(0.0).show(ui, |ui| {
            ui.add_sized([ID_W, ROW_H], egui::Label::new(RichText::new(tl!("ID")).strong()));
            ui.add_sized([NAME_W, ROW_H], egui::Label::new(RichText::new(tl!("Resource Name")).strong()));
            let (rect, r) = ui.allocate_exact_size(Vec2::new(WORK_W, ROW_H), Sense::hover());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tl!("Work")));
            let font = egui::TextStyle::Body.resolve(ui.style());
            ui.painter().text(
                rect.right_center() - Vec2::new(6.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                tl!("Work"),
                font,
                ui.visuals().strong_text_color(),
            );
            let (header, _) = ui.allocate_exact_size(Vec2::new(chart_w, timescale::TIER_H * 2.0), Sense::hover());
            timescale::header(ui, header, scale);
            ui.end_row();
            let mut n = 0usize;
            for row in &rows {
                let colour = if row.over_anywhere { t.overallocated } else { t.text };
                let font = egui::TextStyle::Body.resolve(ui.style());
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ID_W, ROW_H), Sense::hover());
                if row.resource {
                    n = n.saturating_add(1);
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n.to_string(), font.clone(), colour);
                }
                let (rect, r) = ui.allocate_exact_size(Vec2::new(NAME_W, ROW_H), Sense::hover());
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, row.name));
                let lead = if row.resource { 4.0 } else { 4.0 + INDENT_W };
                let name = if row.resource { RichText::new(row.name).strong() } else { RichText::new(row.name) };
                let galley = egui::WidgetText::from(name.color(colour)).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    NAME_W - lead,
                    egui::TextStyle::Body,
                );
                ui.painter().galley(rect.left_center() + Vec2::new(lead, -galley.size().y / 2.0), galley, colour);
                if row.over_anywhere {
                    r.on_hover_text(tl!("Overallocated: red days have more work than Max Units allow on this resource's calendar"));
                }
                let (rect, _) = ui.allocate_exact_size(Vec2::new(WORK_W, ROW_H), Sense::hover());
                ui.painter().with_clip_rect(rect).text(
                    rect.right_center() - Vec2::new(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    &row.work,
                    font,
                    colour,
                );
                cells(ui, p, scale, chart_w, row, &t);
                ui.end_row();
            }
        });
    }
}

/// One row's timescale: the non-working days shaded, a line between units, and the work in each
/// unit on screen; a resource's units over Max Units on a red ground in red.
fn cells(ui: &mut egui::Ui, p: &Project, scale: timescale::Scale, chart_w: f32, row: &Row<'_>, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(chart_w, ROW_H), Sense::hover());
    let full = rect.expand2(Vec2::new(0.0, ui.spacing().item_spacing.y / 2.0));
    if scale.day_w >= 4.0 {
        timescale::shade(ui, full, scale, &p.calendar);
    }
    let clip = ui.clip_rect().intersect(rect);
    if !clip.is_positive() {
        return;
    }
    let unit = timescale::tiers(scale.day_w).1;
    let (left, right) = (clip.left() - rect.left(), clip.right() - rect.left());
    let painter = ui.painter_at(full);
    let font = egui::TextStyle::Body.resolve(ui.style());
    for (from, x0, x1) in timescale::spans(scale, unit, left, right) {
        let cell = egui::Rect::from_min_max(egui::pos2(rect.left() + x0, full.top()), egui::pos2(rect.left() + x1, full.bottom()));
        painter.line_segment([cell.left_top(), cell.left_bottom()], Stroke::new(1.0, t.border));
        let Some(days) = row.days else { continue };
        let span = ((x1 - x0) / scale.day_w).round().max(1.0) as i64;
        let Some(to) = from.add_days(span) else { continue };
        let work: i64 = days.range(from..to).fold(0i64, |a, (_, w)| a.saturating_add(*w));
        if work <= 0 {
            continue;
        }
        let over = row.over.is_some_and(|o| o.range(from..to).next().is_some());
        if over {
            painter.rect_filled(cell.shrink2(Vec2::new(1.0, 1.5)), 2.0, t.overallocated_fill);
        }
        let text = assign::work_text(work, p);
        let colour = if over { t.overallocated } else { t.text };
        let galley = painter.layout_no_wrap(text, font.clone(), colour);
        // A number too wide for its unit is left out rather than overlapping the next.
        if galley.size().x <= cell.width() - 4.0 {
            painter.galley(egui::pos2(cell.right() - 3.0 - galley.size().x, cell.center().y - galley.size().y / 2.0), galley, colour);
        }
    }
}
