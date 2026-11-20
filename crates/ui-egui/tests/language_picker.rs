use egui_kittest::kittest::Queryable;
use projectcraft_ui_egui::App;
use projectcraft_ui_egui::i18n::{self, Lang};

#[test]
fn view_language_menu_selects_hebrew_and_switches_back() {
    let app = App::new(projectcraft_engine::sample(), None);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run_steps(2);

    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Language ⏵").click();
    h.run_steps(2);
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
