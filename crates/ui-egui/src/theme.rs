//! Design tokens (light and dark) and the egui style built from them, as MailCraft and WordCraft.
//!
//! Every colour the interface draws comes from [`Tokens`]; widgets never hard-code one.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Every colour the UI uses.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    /// The window's own background: toolbar, status bar, around the sheet.
    pub window: Color32,
    /// Where content sits: the task sheet and the chart.
    pub surface: Color32,
    /// Every other row of the sheet.
    pub stripe: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub input: Color32,
    pub input_border: Color32,
    /// A task bar, a milestone diamond.
    pub bar: Color32,
    /// A critical task's bar.
    pub bar_critical: Color32,
    /// A warning indicator: a link that a constraint overrides.
    pub warning: Color32,
    /// A summary task's bar and its end points.
    pub bar_summary: Color32,
    /// A deadline marker on the Gantt Chart.
    pub deadline: Color32,
    /// A missed deadline: its marker and the indicator.
    pub deadline_missed: Color32,
    /// Behind the bars: a day that is not worked (a weekend day, a non-working exception).
    pub nonworking: Color32,
    /// A selected row of the sheet.
    pub row_selected: Color32,
}

impl Tokens {
    /// The light palette: white and light neutral greys, with a blue accent.
    pub fn light() -> Self {
        Tokens {
            dark: false,
            window: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            surface: Color32::WHITE,
            stripe: Color32::from_rgb(0xF7, 0xF7, 0xF7),
            border: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            border_strong: Color32::from_rgb(0xC4, 0xC4, 0xC4),
            text: Color32::from_rgb(0x24, 0x24, 0x24),
            text_dim: Color32::from_rgb(0x61, 0x61, 0x61),
            text_disabled: Color32::from_rgb(0xA6, 0xA6, 0xA6),
            hover: Color32::from_rgb(0xE9, 0xE9, 0xE9),
            pressed: Color32::from_rgb(0xD6, 0xD6, 0xD6),
            accent: Color32::from_rgb(0x1F, 0x6F, 0xB8),
            accent_text: Color32::from_rgb(0x15, 0x5C, 0x9C),
            input: Color32::WHITE,
            input_border: Color32::from_rgb(0xC4, 0xC4, 0xC4),
            bar: Color32::from_rgb(0x3C, 0x78, 0xC8),
            bar_critical: Color32::from_rgb(0xC8, 0x3C, 0x3C),
            warning: Color32::from_rgb(0xB0, 0x5A, 0x00),
            bar_summary: Color32::from_rgb(0x30, 0x30, 0x30),
            deadline: Color32::from_rgb(0x2E, 0x7D, 0x32),
            deadline_missed: Color32::from_rgb(0xA3, 0x15, 0x7A),
            nonworking: Color32::from_rgb(0xEB, 0xEB, 0xEE),
            row_selected: Color32::from_rgb(0xD4, 0xE6, 0xF8),
        }
    }

    /// The dark palette: near-black neutral greys, with a light blue accent.
    pub fn dark() -> Self {
        Tokens {
            dark: true,
            window: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            surface: Color32::from_rgb(0x24, 0x24, 0x24),
            stripe: Color32::from_rgb(0x2A, 0x2A, 0x2A),
            border: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            border_strong: Color32::from_rgb(0x5A, 0x5A, 0x5A),
            text: Color32::from_rgb(0xEE, 0xEE, 0xEE),
            text_dim: Color32::from_rgb(0xB4, 0xB4, 0xB4),
            text_disabled: Color32::from_rgb(0x70, 0x70, 0x70),
            hover: Color32::from_rgb(0x38, 0x38, 0x38),
            pressed: Color32::from_rgb(0x4A, 0x4A, 0x4A),
            accent: Color32::from_rgb(0x58, 0xA8, 0xF0),
            accent_text: Color32::from_rgb(0x82, 0xC0, 0xFF),
            input: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            input_border: Color32::from_rgb(0x55, 0x55, 0x55),
            bar: Color32::from_rgb(0x5A, 0x96, 0xE6),
            bar_critical: Color32::from_rgb(0xE8, 0x62, 0x62),
            warning: Color32::from_rgb(0xF2, 0xA6, 0x3B),
            bar_summary: Color32::from_rgb(0xD2, 0xD2, 0xD2),
            deadline: Color32::from_rgb(0x6C, 0xC0, 0x70),
            deadline_missed: Color32::from_rgb(0xF0, 0x6A, 0xC8),
            nonworking: Color32::from_rgb(0x31, 0x31, 0x36),
            row_selected: Color32::from_rgb(0x23, 0x3F, 0x5C),
        }
    }

    /// The palette egui is drawing with now.
    pub fn get(ctx: &egui::Context) -> Tokens {
        if ctx.global_style().visuals.dark_mode { Tokens::dark() } else { Tokens::light() }
    }
}

/// Put both palettes into egui (ours as its light and its dark style) and pick the theme:
/// `System` follows the OS (or browser) appearance, `Light` and `Dark` pin it.
pub fn apply(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(&Tokens::light()));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&Tokens::dark()));
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
    });
    ctx.options_mut(|o| {
        o.fallback_theme = egui::Theme::Light;
        o.theme_preference = preference;
        // Ctrl with = and - zoom the Gantt Chart's timescale; the window itself never zooms.
        o.zoom_with_keyboard = false;
    });
}

/// egui's visuals for one of our palettes.
fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.window;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.input;
    v.faint_bg_color = t.stripe;
    v.window_stroke = Stroke::new(1.0, t.border_strong);
    v.selection.bg_fill = t.accent.linear_multiply(if t.dark { 0.45 } else { 0.30 });
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent_text;
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = t.surface;
    v.widgets.noninteractive.weak_bg_fill = t.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.bg_fill = t.input;
    v.widgets.inactive.weak_bg_fill = t.input;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.corner_radius = CornerRadius::same(4);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.corner_radius = CornerRadius::same(4);
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.corner_radius = CornerRadius::same(4);
    v.widgets.open.bg_fill = t.pressed;
    v.widgets.open.weak_bg_fill = t.pressed;
    v.text_cursor.stroke = Stroke::new(2.0, t.accent);
    v.popup_shadow.color = shadow(t, 40, 120);
    v.window_shadow.color = shadow(t, 50, 140);
    v
}

fn shadow(t: &Tokens, light: u8, dark: u8) -> Color32 {
    Color32::from_black_alpha(if t.dark { dark } else { light })
}
