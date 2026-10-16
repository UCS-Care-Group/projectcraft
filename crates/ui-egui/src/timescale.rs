//! The Gantt Chart's timescale: how wide a day is, the two header tiers that width calls for,
//! the date under any x, and the shading of non-working days.
//!
//! Zoom is view state: it is not a command, not saved in the file and not undone.
//! The arithmetic is pure ([`Scale`], [`spans`], [`nonworking_runs`]) so the header, the shading
//! and the bars all place a date at the same x. Drawing only walks what is on screen, so a plan
//! spanning decades costs no more to draw than one spanning a month.

use egui::{FontId, Rect, Stroke, pos2};
use projectcraft_engine::model::Calendar;
use projectcraft_engine::time::Date;

use crate::theme::Tokens;

/// Narrowest and widest a day can be drawn, in points.
pub const MIN_DAY_W: f32 = 1.0;
pub const MAX_DAY_W: f32 = 48.0;
/// The steps Zoom In and Zoom Out move along (each preset is one of them).
const LADDER: [f32; 12] = [1.0, 1.5, 2.0, 3.0, 4.5, 6.0, 9.0, 13.0, 18.0, 26.0, 36.0, 48.0];
/// Height of one header tier.
pub const TIER_H: f32 = 18.0;
/// Furthest the chart reaches past its origin, in calendar days (about a century); a date
/// beyond it (a hostile file's year 9999) is not drawn.
pub const MAX_CHART_DAYS: i64 = 36_600;
/// Most units one tier draws in a frame; more than any screen can show at the narrowest zoom.
const MAX_SPANS: usize = 4096;
/// Most days one frame shades; a screen at the narrowest zoom shows far fewer.
const MAX_SHADED_DAYS: i64 = 20_000;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The View menu's timescale presets, named after the bottom tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Weeks over days (the default).
    Days,
    /// Months over weeks.
    Weeks,
    /// Years over months.
    Months,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Days, Preset::Weeks, Preset::Months];

    /// The day width this preset sets.
    pub fn day_w(self) -> f32 {
        match self {
            Preset::Days => 18.0,
            Preset::Weeks => 6.0,
            Preset::Months => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Days => "Days",
            Preset::Weeks => "Weeks",
            Preset::Months => "Months",
        }
    }

    /// The preset whose tiers a day width shows.
    pub fn of(day_w: f32) -> Preset {
        match tiers(day_w).1 {
            Unit::Day => Preset::Days,
            Unit::Week => Preset::Weeks,
            _ => Preset::Months,
        }
    }
}

/// A unit of one header tier. Weeks start on Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

/// The (top, bottom) tiers at a day width: weeks over days, months over weeks, years over months.
pub fn tiers(day_w: f32) -> (Unit, Unit) {
    if day_w >= 12.0 {
        (Unit::Week, Unit::Day)
    } else if day_w >= 4.0 {
        (Unit::Month, Unit::Week)
    } else {
        (Unit::Year, Unit::Month)
    }
}

/// A day width within the zoom limits; anything not a number becomes the default.
pub fn clamp_day_w(day_w: f32) -> f32 {
    if day_w.is_finite() { day_w.clamp(MIN_DAY_W, MAX_DAY_W) } else { Preset::Days.day_w() }
}

/// The next wider step, stopping at the widest.
pub fn zoom_in(day_w: f32) -> f32 {
    let w = clamp_day_w(day_w);
    LADDER.iter().copied().find(|s| *s > w + 1e-3).unwrap_or(MAX_DAY_W)
}

/// The next narrower step, stopping at the narrowest.
pub fn zoom_out(day_w: f32) -> f32 {
    let w = clamp_day_w(day_w);
    LADDER.iter().rev().copied().find(|s| *s < w - 1e-3).unwrap_or(MIN_DAY_W)
}

/// Where dates sit across the chart: x = 0 is the left edge of `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub origin: Date,
    pub day_w: f32,
}

impl Scale {
    /// The chart for a project starting on `start`: it opens on the Monday of the week before,
    /// so the first bar has room and the first week is whole.
    pub fn for_project(start: Date, day_w: f32) -> Scale {
        let monday = start.add_days(-i64::from(start.weekday())).unwrap_or(start);
        Scale { origin: monday.add_days(-7).unwrap_or(monday), day_w: clamp_day_w(day_w) }
    }

    /// The x of the left edge of `d`.
    pub fn x(self, d: Date) -> f32 {
        d.days_since(self.origin) as f32 * self.day_w
    }

    /// The date under `x`; `None` past the ends of the calendar.
    pub fn date_at(self, x: f32) -> Option<Date> {
        let n = (x / self.day_w).floor();
        if !n.is_finite() {
            return None;
        }
        self.origin.add_days(n as i64)
    }
}

/// The first day of the `unit` holding `d`.
pub fn unit_start(unit: Unit, d: Date) -> Date {
    let (y, m, _) = d.ymd();
    match unit {
        Unit::Day => Some(d),
        Unit::Week => d.add_days(-i64::from(d.weekday())),
        Unit::Month => Date::from_ymd(y, m, 1),
        Unit::Year => Date::from_ymd(y, 1, 1),
    }
    .unwrap_or(d)
}

/// The first day of the next `unit` after the one starting on `start`.
fn next(unit: Unit, start: Date) -> Option<Date> {
    let (y, m, _) = start.ymd();
    match unit {
        Unit::Day => start.add_days(1),
        Unit::Week => start.add_days(7),
        Unit::Month if m == 12 => Date::from_ymd(y.checked_add(1)?, 1, 1),
        Unit::Month => Date::from_ymd(y, m + 1, 1),
        Unit::Year => Date::from_ymd(y.checked_add(1)?, 1, 1),
    }
}

/// The units of one tier that show between `left` and `right` (chart x): each one's first
/// day and its left and right x.
pub fn spans(scale: Scale, unit: Unit, left: f32, right: f32) -> Vec<(Date, f32, f32)> {
    let mut out = Vec::new();
    let Some(first) = scale.date_at(left) else { return out };
    let mut d = unit_start(unit, first);
    while out.len() < MAX_SPANS {
        let x0 = scale.x(d);
        if x0 >= right {
            break;
        }
        let Some(n) = next(unit, d) else { break };
        out.push((d, x0, scale.x(n)));
        d = n;
    }
    out
}

/// The non-working stretches between `left` and `right` (chart x), as (left x, right x): days
/// off by weekday, and non-working exceptions. A working exception is not shaded.
pub fn nonworking_runs(calendar: &Calendar, scale: Scale, left: f32, right: f32) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::new();
    let Some(mut d) = scale.date_at(left) else { return out };
    for _ in 0..MAX_SHADED_DAYS {
        let x0 = scale.x(d);
        if x0 >= right {
            break;
        }
        let x1 = x0 + scale.day_w;
        if !calendar.is_workday(d) {
            match out.last_mut() {
                Some(run) if (run.1 - x0).abs() < 0.01 => run.1 = x1,
                _ => out.push((x0, x1)),
            }
        }
        let Some(n) = d.add_days(1) else { break };
        d = n;
    }
    out
}

/// A unit's label, longest first; the header shows the first that fits.
fn labels(unit: Unit, top: bool, d: Date) -> Vec<String> {
    let (y, m, day) = d.ymd();
    let month = MONTHS.get(m.saturating_sub(1) as usize).map_or("", |s| tl!(s));
    match (unit, top) {
        (Unit::Day, _) => vec![day.to_string()],
        (Unit::Week, true) => vec![format!("{day} {month} {y}"), format!("{day} {month}")],
        (Unit::Week, false) => vec![day.to_string()],
        (Unit::Month, true) => vec![format!("{month} {y}"), month.to_string()],
        (Unit::Month, false) => vec![month.to_string(), month.chars().take(1).collect()],
        (Unit::Year, _) => vec![y.to_string()],
    }
}

/// Draws the two tiers into `rect` (the chart's header cell, whose left edge is x = 0).
pub fn header(ui: &egui::Ui, rect: Rect, scale: Scale) {
    let t = Tokens::get(ui.ctx());
    let clip = ui.clip_rect().intersect(rect);
    if !clip.is_positive() {
        return;
    }
    let p = ui.painter_at(rect);
    p.rect_filled(clip, 0.0, t.window);
    let (left, right) = (clip.left() - rect.left(), clip.right() - rect.left());
    let (top, bottom) = tiers(scale.day_w);
    let font = FontId::proportional(11.0);
    let line = Stroke::new(1.0, t.border);
    for (row, unit) in [(0.0, top), (1.0, bottom)] {
        let (y0, y1) = (rect.top() + row * TIER_H, rect.top() + (row + 1.0) * TIER_H);
        for (d, x0, x1) in spans(scale, unit, left, right) {
            let (sx0, sx1) = (rect.left() + x0, rect.left() + x1);
            p.line_segment([pos2(sx0, y0), pos2(sx0, y1)], line);
            // A label too wide for its unit is left out rather than overlapping the next.
            // The top tier's label sits at the left of the unit's visible part, so a unit wider
            // than the view keeps its label on screen; the bottom tier's sits in the middle.
            let from = if row == 0.0 { sx0.max(clip.left()) } else { sx0 };
            let room = if row == 0.0 { sx1.min(clip.right()) } else { sx1 } - from - 4.0;
            for text in labels(unit, row == 0.0, d) {
                let galley = p.layout_no_wrap(text, font.clone(), if row == 0.0 { t.text } else { t.text_dim });
                if galley.size().x <= room {
                    let x = if row == 0.0 { from + 3.0 } else { (sx0 + sx1 - galley.size().x) / 2.0 };
                    p.galley(pos2(x, (y0 + y1 - galley.size().y) / 2.0), galley, t.text);
                    break;
                }
            }
        }
        p.line_segment([pos2(clip.left(), y1 - 0.5), pos2(clip.right(), y1 - 0.5)], line);
    }
}

/// Shades the non-working days in `rect` (one row's chart cell, whose left edge is x = 0).
pub fn shade(ui: &egui::Ui, rect: Rect, scale: Scale, calendar: &Calendar) {
    let clip = ui.clip_rect().intersect(rect);
    if !clip.is_positive() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let (left, right) = (clip.left() - rect.left(), clip.right() - rect.left());
    // Days a few points wide shade at half strength, so weekends read as a light texture.
    let fill = if scale.day_w < 4.0 { t.nonworking.linear_multiply(0.5) } else { t.nonworking };
    for (x0, x1) in nonworking_runs(calendar, scale, left, right) {
        let r = Rect::from_min_max(pos2(rect.left() + x0, rect.top()), pos2(rect.left() + x1, rect.bottom()));
        p.rect_filled(r.intersect(rect), 0.0, fill);
    }
}

/// Whether `d` falls on the chart: from its origin to [`MAX_CHART_DAYS`] after.
pub fn in_range(scale: Scale, d: Date) -> bool {
    (0..MAX_CHART_DAYS).contains(&d.days_since(scale.origin))
}

/// The x of the end of `d` (where a finish or a deadline is drawn), if it is on a chart
/// `chart_w` wide.
pub fn end_of_day_x(scale: Scale, d: Date, chart_w: f32) -> Option<f32> {
    let x = scale.x(d) + scale.day_w;
    (in_range(scale, d) && x <= chart_w).then_some(x)
}

/// The chart's width: from its origin to a fortnight past the project's finish, and at least
/// `min_w` so the shading fills the view. Bounded, however far a hostile file's dates run.
pub fn chart_width(scale: Scale, finish: Date, min_w: f32) -> f32 {
    let days = finish.days_since(scale.origin).saturating_add(15).clamp(1, MAX_CHART_DAYS);
    (days as f32 * scale.day_w).max(min_w).min(4_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[test]
    fn dates_and_x_map_both_ways_at_every_zoom() {
        // 2026-11-02 is a Monday; a project starting Wednesday the 4th opens on Monday 26 October.
        let s = Scale::for_project(date("2026-11-04"), 18.0);
        assert_eq!(s.origin, date("2026-10-26"));
        assert_eq!(s.x(date("2026-10-26")), 0.0);
        assert_eq!(s.x(date("2026-11-04")), 9.0 * 18.0);
        for w in LADDER {
            let s = Scale { day_w: w, ..s };
            for n in -400..400 {
                let d = s.origin.add_days(n).unwrap();
                assert_eq!(s.date_at(s.x(d)), Some(d), "{d} at {w}");
                assert_eq!(s.date_at(s.x(d) + w * 0.99), Some(d), "{d} at {w}, right edge");
            }
        }
        assert_eq!(s.date_at(f32::NAN), None);
        assert_eq!(s.date_at(f32::INFINITY), None);
        assert_eq!(s.date_at(1e12), None);
    }

    #[test]
    fn zoom_steps_and_clamps() {
        assert_eq!(zoom_in(18.0), 26.0);
        assert_eq!(zoom_out(18.0), 13.0);
        assert_eq!(zoom_in(MAX_DAY_W), MAX_DAY_W);
        assert_eq!(zoom_out(MIN_DAY_W), MIN_DAY_W);
        assert_eq!(zoom_in(1e9), MAX_DAY_W);
        assert_eq!(zoom_out(-5.0), MIN_DAY_W);
        assert_eq!(clamp_day_w(f32::NAN), 18.0);
        assert_eq!(Scale::for_project(date("2026-11-04"), 0.0).day_w, MIN_DAY_W);
        let mut w = MIN_DAY_W;
        for _ in 0..LADDER.len() * 2 {
            w = zoom_in(w);
        }
        assert_eq!(w, MAX_DAY_W);
        // Each preset is a step, and shows its own tiers.
        for p in Preset::ALL {
            assert!(LADDER.contains(&p.day_w()));
            assert_eq!(Preset::of(p.day_w()), p);
        }
        assert_eq!(tiers(Preset::Days.day_w()), (Unit::Week, Unit::Day));
        assert_eq!(tiers(Preset::Weeks.day_w()), (Unit::Month, Unit::Week));
        assert_eq!(tiers(Preset::Months.day_w()), (Unit::Year, Unit::Month));
    }

    #[test]
    fn tier_units_line_up_with_day_columns() {
        let s = Scale::for_project(date("2026-11-04"), 6.0);
        let weeks = spans(s, Unit::Week, 0.0, 6.0 * 21.0);
        assert_eq!(weeks.len(), 3);
        assert_eq!(weeks[0], (date("2026-10-26"), 0.0, 42.0));
        assert_eq!(weeks[1].0, date("2026-11-02"));
        // A month starting mid-week: its edge is the x of its first day.
        let months = spans(s, Unit::Month, 0.0, 6.0 * 70.0);
        assert_eq!(months[0].0, date("2026-10-01"));
        assert_eq!(months[1], (date("2026-11-01"), s.x(date("2026-11-01")), s.x(date("2026-12-01"))));
        assert_eq!(months[3].0, date("2027-01-01"));
        let years = spans(s, Unit::Year, 0.0, 1.0);
        assert_eq!(years, vec![(date("2026-01-01"), s.x(date("2026-01-01")), s.x(date("2027-01-01")))]);
        // Bounded however wide the range.
        let s = Scale { day_w: MIN_DAY_W, ..s };
        assert!(spans(s, Unit::Day, -1e9, 1e9).len() <= MAX_SPANS);
        assert!(spans(s, Unit::Day, f32::NAN, 10.0).is_empty());
    }

    #[test]
    fn weekends_and_non_working_exceptions_are_shaded_working_ones_are_not() {
        let mut cal = Calendar::default();
        cal.add_exception(date("2026-11-04"), false).unwrap(); // a Wednesday off
        cal.add_exception(date("2026-11-07"), true).unwrap(); // a Saturday worked
        let s = Scale::for_project(date("2026-11-04"), 10.0); // origin Monday 26 October
        let runs = nonworking_runs(&cal, s, 7.0 * 10.0, 14.0 * 10.0); // the week of 2 November
        let x = |d: &str| s.x(date(d));
        assert_eq!(runs, vec![(x("2026-11-04"), x("2026-11-05")), (x("2026-11-08"), x("2026-11-09"))]);
        // Saturday and Sunday join into one stretch.
        let runs = nonworking_runs(&Calendar::default(), s, 0.0, 70.0);
        assert_eq!(runs, vec![(x("2026-10-31"), x("2026-11-02"))]);
        assert!(nonworking_runs(&cal, s, 0.0, 1e12).len() as i64 <= MAX_SHADED_DAYS);
    }

    #[test]
    fn chart_width_is_bounded_and_fills_the_view() {
        let s = Scale::for_project(date("2026-11-04"), 18.0);
        assert_eq!(chart_width(s, date("2026-11-04"), 0.0), (9.0 + 15.0) * 18.0);
        assert_eq!(chart_width(s, date("2026-11-04"), 2000.0), 2000.0);
        assert!(chart_width(Scale { day_w: MAX_DAY_W, ..s }, date("9999-12-31"), 0.0) <= 4_000_000.0);
        assert!(chart_width(s, date("0001-01-01"), 0.0) > 0.0);
    }

    #[test]
    fn deadline_markers_sit_at_the_end_of_their_day_and_far_dates_are_left_out() {
        let s = Scale::for_project(date("2026-11-04"), 18.0);
        let w = chart_width(s, date("2026-11-20"), 0.0);
        assert_eq!(end_of_day_x(s, date("2026-11-20"), w), Some(s.x(date("2026-11-21"))));
        assert_eq!(end_of_day_x(s, date("2026-10-01"), w), None);
        assert_eq!(end_of_day_x(s, date("9999-12-31"), 1e9), None);
        assert!(!in_range(s, date("9999-12-31")));
        // A deadline the chart is widened for is drawn.
        let far = date("2027-06-30");
        assert!(end_of_day_x(s, far, chart_width(s, far, 0.0)).is_some());
    }

    #[test]
    fn labels_fall_back_to_shorter_forms() {
        assert_eq!(labels(Unit::Week, true, date("2026-11-02")), vec!["2 Nov 2026", "2 Nov"]);
        assert_eq!(labels(Unit::Month, false, date("2026-11-01")), vec!["Nov", "N"]);
        assert_eq!(labels(Unit::Year, true, date("2026-01-01")), vec!["2026"]);
    }
}
