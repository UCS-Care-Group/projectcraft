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
    /// An overallocated resource: its name, its indicator on a task, its work over Max Units.
    pub overallocated: Color32,
    /// Behind a Resource Usage cell whose work is over Max Units.
    pub overallocated_fill: Color32,
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
            overallocated: Color32::from_rgb(0xC6, 0x28, 0x28),
            overallocated_fill: Color32::from_rgb(0xFB, 0xE3, 0xE3),
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
            overallocated: Color32::from_rgb(0xFF, 0x7A, 0x7A),
            overallocated_fill: Color32::from_rgb(0x4A, 0x24, 0x26),
        }
    }

    /// The palette egui is drawing with now.
    pub fn get(ctx: &egui::Context) -> Tokens {
        if ctx.global_style().visuals.dark_mode { Tokens::dark() } else { Tokens::light() }
    }
}

/// Put both palettes into egui (ours as its light and its dark style), install a Hebrew
/// fallback font if not yet registered on `ctx`, and pick the theme: `System` follows the OS (or
/// browser) appearance, `Light` and `Dark` pin it.
pub fn apply(ctx: &egui::Context, preference: egui::ThemePreference) {
    install_hebrew_fonts(ctx);
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

fn hebrew_available_key() -> egui::Id {
    egui::Id::new("projectcraft_hebrew_fonts_available")
}

fn hebrew_installed_key() -> egui::Id {
    egui::Id::new("projectcraft_hebrew_fonts_installed")
}

fn hebrew_installed_pass_key() -> egui::Id {
    egui::Id::new("projectcraft_hebrew_fonts_installed_pass")
}

/// Simulates an environment with no Hebrew-capable fallback font for integration tests without
/// duplicating internal cache keys.
#[doc(hidden)]
pub fn disable_hebrew_font_for_test(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.insert_temp(hebrew_installed_key(), true);
        d.insert_temp(hebrew_available_key(), false);
    });
}

fn has_hebrew_glyphs(ctx: &egui::Context) -> bool {
    ctx.fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::proportional(14.0), 'א') && fonts.has_glyph(&egui::FontId::monospace(14.0), 'א'))
}

/// Returns whether the fonts loaded on `ctx` can render Hebrew glyphs.
///
/// Must be called after `ctx` has completed at least one pass (or inside a `Ui` pass once any
/// queued `set_fonts` update has been applied).
pub fn hebrew_font_available(ctx: &egui::Context) -> bool {
    if let Some(available) = ctx.data(|d| d.get_temp::<bool>(hebrew_available_key())) {
        return available;
    }
    let installed_pass = ctx.data(|d| d.get_temp::<u64>(hebrew_installed_pass_key()));
    if installed_pass.is_some_and(|pass| ctx.cumulative_pass_nr() <= pass) {
        // `set_fonts` takes effect on the next pass; keep the candidate active until then.
        return true;
    }
    let available = has_hebrew_glyphs(ctx);
    ctx.data_mut(|d| d.insert_temp(hebrew_available_key(), available));
    available
}

#[cfg(not(target_arch = "wasm32"))]
fn read_be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(not(target_arch = "wasm32"))]
fn has_valid_sfnt_at(bytes: &[u8], offset: usize) -> bool {
    let Some(magic) = bytes.get(offset..offset + 4) else {
        return false;
    };
    if !matches!(magic, [0x00, 0x01, 0x00, 0x00] | b"OTTO" | b"true") {
        return false;
    }
    let Some(num_tables) = read_be_u16(bytes, offset + 4).map(usize::from) else {
        return false;
    };
    if num_tables == 0 {
        return false;
    }
    let Some(dir_end) = offset.checked_add(12).and_then(|s| s.checked_add(num_tables * 16)) else {
        return false;
    };
    if dir_end > bytes.len() {
        return false;
    }
    let mut has_valid_head = false;
    for i in 0..num_tables {
        let rec = offset + 12 + i * 16;
        let Some(tag) = bytes.get(rec..rec + 4) else {
            return false;
        };
        let Some(tbl_off) = read_be_u32(bytes, rec + 8).and_then(|v| usize::try_from(v).ok()) else {
            return false;
        };
        let Some(tbl_len) = read_be_u32(bytes, rec + 12).and_then(|v| usize::try_from(v).ok()) else {
            return false;
        };
        let Some(tbl_end) = tbl_off.checked_add(tbl_len) else {
            return false;
        };
        if tbl_end > bytes.len() {
            return false;
        }
        if tag == b"head" {
            if tbl_len < 54 || read_be_u32(bytes, tbl_off + 12) != Some(0x5F0F_3CF5) {
                return false;
            }
            has_valid_head = true;
        }
    }
    has_valid_head
}

#[cfg(not(target_arch = "wasm32"))]
fn has_valid_sfnt_structure(bytes: &[u8]) -> bool {
    if bytes.get(..4) == Some(b"ttcf") {
        let Some(num_fonts) = read_be_u32(bytes, 8).and_then(|v| usize::try_from(v).ok()) else {
            return false;
        };
        if num_fonts == 0 {
            return false;
        }
        let Some(first_offset) = read_be_u32(bytes, 12).and_then(|v| usize::try_from(v).ok()) else {
            return false;
        };
        return has_valid_sfnt_at(bytes, first_offset);
    }
    has_valid_sfnt_at(bytes, 0)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_valid_font_bytes(bytes: &[u8]) -> bool {
    if !has_valid_sfnt_structure(bytes) {
        return false;
    }
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data.insert("hebrew-probe".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes.to_vec())));
    // Parse through epaint's loader so truncated/corrupt faces are skipped rather than panicking
    // during FontDefinitions initialization.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = egui::epaint::text::FontsImpl::new(egui::epaint::text::TextOptions::default(), defs);
    }))
    .is_ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn read_valid_font(path: &std::path::Path) -> Option<egui::FontData> {
    let bytes = std::fs::read(path).ok()?;
    if !is_valid_font_bytes(&bytes) {
        eprintln!("note: skipping invalid Hebrew fallback font file: {}", path.display());
        return None;
    }
    Some(egui::FontData::from_owned(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn hebrew_font_data_from(env_dirs: &[Option<String>], candidates: &[&str]) -> Option<egui::FontData> {
    for dir in env_dirs.iter().flatten() {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            for filename in ["NotoSansHebrew-Regular.ttf", "NotoSansHebrew[wdth,wght].ttf"] {
                let candidate = std::path::Path::new(trimmed).join(filename);
                if let Some(data) = read_valid_font(&candidate) {
                    return Some(data);
                }
            }
        }
    }
    candidates.iter().find_map(|path| read_valid_font(std::path::Path::new(path)))
}

#[cfg(not(target_arch = "wasm32"))]
fn hebrew_font_data() -> Option<egui::FontData> {
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/SFHebrew.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansHebrew-Regular.ttf",
        "/usr/local/share/fonts/noto/NotoSansHebrew-Regular.ttf",
        "/run/current-system/sw/share/fonts/truetype/NotoSansHebrew-Regular.ttf",
        "/run/current-system/sw/share/fonts/noto/NotoSansHebrew-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
        "/usr/share/fonts/TTF/FreeSans.ttf",
        "/usr/share/fonts/gnu-free/FreeSans.ttf",
        "/usr/local/share/fonts/freefont-ttf/FreeSans.ttf",
    ];
    #[cfg(windows)]
    const CANDIDATES: &[&str] = &[r"C:\Windows\Fonts\arial.ttf", r"C:\Windows\Fonts\segoeui.ttf", r"C:\Windows\Fonts\tahoma.ttf"];
    #[cfg(not(any(unix, windows)))]
    const CANDIDATES: &[&str] = &[];

    let env_dirs = [std::env::var("PROJECTCRAFT_FONT_DIR").ok(), std::env::var("CRAFT_FONTS_DIR").ok()];
    hebrew_font_data_from(&env_dirs, CANDIDATES)
}

#[cfg(target_arch = "wasm32")]
fn hebrew_font_data() -> Option<egui::FontData> {
    None
}

fn apply_hebrew_fallback(fonts: &mut egui::FontDefinitions, font_data: egui::FontData) {
    fonts.font_data.insert("hebrew_fallback".to_owned(), std::sync::Arc::new(font_data));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("hebrew_fallback".to_owned());
        }
    }
}

fn install_hebrew_fonts(ctx: &egui::Context) {
    let key = hebrew_installed_key();
    if ctx.data(|d| d.get_temp::<bool>(key)) == Some(true) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, true));
    let Some(font_data) = hebrew_font_data() else {
        #[cfg(not(target_arch = "wasm32"))]
        {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!(
                    "note: no Hebrew fallback font found (checked PROJECTCRAFT_FONT_DIR, CRAFT_FONTS_DIR, and system font paths); Hebrew UI disabled"
                );
            });
        }
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    apply_hebrew_fallback(&mut fonts, font_data);
    ctx.set_fonts(fonts);
    ctx.data_mut(|d| d.remove::<bool>(hebrew_available_key()));
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(hebrew_installed_pass_key(), pass));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebrew_fallback_registers_in_proportional_and_monospace_families() {
        let mut fonts = egui::FontDefinitions::default();
        apply_hebrew_fallback(&mut fonts, egui::FontData::from_static(&[]));
        assert!(fonts.font_data.contains_key("hebrew_fallback"));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = fonts.families.get(&family).expect("family present");
            assert_eq!(list.last().map(String::as_str), Some("hebrew_fallback"));
        }
    }

    #[test]
    fn default_fonts_report_hebrew_unavailable_until_fallback_is_installed() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        assert!(!hebrew_font_available(&ctx), "egui default fonts should not report Hebrew coverage");
    }

    #[test]
    fn hebrew_fallback_font_supplies_hebrew_glyphs_when_available() {
        let ctx = egui::Context::default();
        apply(&ctx, egui::ThemePreference::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        if !hebrew_font_available(&ctx) {
            eprintln!("skipping glyph check: no system Hebrew font found on this host");
            return;
        }
        ctx.fonts_mut(|fonts| {
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let font = egui::FontId::new(14.0, family);
                for ch in "אבגדהוזחטיכלמנסעפצקרשתךםןףץ".chars() {
                    assert!(fonts.has_glyph(&font, ch), "missing Hebrew glyph {ch} in {font:?}");
                }
            }
        });
    }

    #[test]
    fn early_hebrew_availability_cache_clears_when_fonts_are_applied() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        assert!(!hebrew_font_available(&ctx));
        apply(&ctx, egui::ThemePreference::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        if hebrew_font_data().is_some() {
            assert!(hebrew_font_available(&ctx), "cached false must clear once fallback font is applied");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn corrupt_font_candidate_is_skipped_and_falls_through() {
        let valid_data = egui::FontDefinitions::default().font_data.into_values().next().expect("default egui font data present");
        let valid_bytes = valid_data.font.as_ref();
        assert!(is_valid_font_bytes(valid_bytes));
        assert!(!is_valid_font_bytes(b""));
        assert!(!is_valid_font_bytes(b"not a font"));
        // Truncated header or truncated real font bytes must fail validation.
        assert!(!is_valid_font_bytes(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x01]));
        assert!(!is_valid_font_bytes(&valid_bytes[..valid_bytes.len() / 2]));
        // Structurally valid SFNT directory pointing at a corrupt `head` table must fail too.
        let mut corrupt_sfnt = vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00];
        corrupt_sfnt.extend_from_slice(b"head");
        corrupt_sfnt.extend_from_slice(&[0u8; 4]);
        corrupt_sfnt.extend_from_slice(&28u32.to_be_bytes());
        corrupt_sfnt.extend_from_slice(&4u32.to_be_bytes());
        corrupt_sfnt.extend_from_slice(&[0xFF; 4]);
        assert!(!is_valid_font_bytes(&corrupt_sfnt));

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("projectcraft-bogus-font-dir-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp font dir");
        std::fs::write(dir.join("NotoSansHebrew-Regular.ttf"), b"truncated").expect("write bogus env font");
        let bad_candidate = dir.join("bad.ttf");
        std::fs::write(&bad_candidate, &valid_bytes[..valid_bytes.len() / 2]).expect("write truncated candidate");
        let good_candidate = dir.join("good.ttf");
        std::fs::write(&good_candidate, valid_bytes).expect("write valid font");

        let env_dir = dir.to_string_lossy().into_owned();
        let bad_str = bad_candidate.to_string_lossy().into_owned();
        let good_str = good_candidate.to_string_lossy().into_owned();
        assert!(hebrew_font_data_from(&[Some(env_dir.clone())], &[&bad_str]).is_none());
        let fell_through = hebrew_font_data_from(&[Some(env_dir)], &[&bad_str, &good_str]);
        assert!(fell_through.is_some(), "valid candidate after invalid env/candidate files should be selected");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
