use std::sync::Mutex;

use egui_kittest::kittest::Queryable;
use projectcraft_ui_egui::App;
use projectcraft_ui_egui::i18n::{self, Lang};
use projectcraft_ui_egui::theme;

static LANG_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn view_language_menu_selects_hebrew_and_switches_back() {
    let _guard = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_current(Lang::English);

    let app = App::new(projectcraft_engine::sample(), None);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run_steps(2);

    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Language ⏵").click();
    h.run_steps(2);
    assert!(h.query_by_label("Pseudo").is_none(), "test-only Pseudo language must not appear in View -> Language");
    if !theme::hebrew_font_available(&h.ctx) {
        assert!(h.query_by_label("עברית").is_none(), "Hebrew must stay hidden when no Hebrew font is installed");
        return;
    }
    h.get_by_label("עברית").click();
    h.run_steps(2);

    assert_eq!(i18n::current(), Lang::Hebrew);
    assert_eq!(h.state_mut().take_language_changed(), Some(Lang::Hebrew));
    // Toolbar labels now resolve through the active Hebrew catalog.
    let _ = h.get_by_label(i18n::t("Save"));
    let _ = h.get_by_label(i18n::t("New Task"));

    // Switch back through the translated View -> Language menus.
    h.get_by_label(i18n::t("View")).click();
    h.run_steps(2);
    let lang_submenu = format!("{} ⏵", i18n::t("Language"));
    h.get_by_label(&lang_submenu).click();
    h.run_steps(2);
    h.get_by_label("English").click();
    h.run_steps(2);

    assert_eq!(i18n::current(), Lang::English);
    assert_eq!(h.state_mut().take_language_changed(), Some(Lang::English));
    let _ = h.get_by_label("Save");
}

#[test]
fn hebrew_falls_back_to_english_without_persisting_when_font_missing() {
    let _guard = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_current(Lang::Hebrew);

    let app_no_font = App::new(projectcraft_engine::sample(), None);
    let mut h_no_font = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(
        |ui, app: &mut App| {
            theme::disable_hebrew_font_for_test(ui.ctx());
            app.ui(ui);
        },
        app_no_font,
    );
    h_no_font.run_steps(2);
    assert_eq!(i18n::current(), Lang::English);
    assert_eq!(h_no_font.state_mut().take_language_changed(), None, "automatic fallback must not overwrite saved Hebrew preference");
    h_no_font.get_by_label("View").click();
    h_no_font.run_steps(2);
    h_no_font.get_by_label("Language ⏵").click();
    h_no_font.run_steps(2);
    assert!(h_no_font.query_by_label("עברית").is_none(), "Hebrew must not be selectable without Hebrew glyphs");

    i18n::set_current(Lang::English);
}
