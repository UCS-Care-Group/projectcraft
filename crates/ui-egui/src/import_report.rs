//! The import report: shown after Open imports a Microsoft Project XML or CSV file, it lists
//! what was converted, dropped or not imported yet, each with where in the file and why.

use std::path::Path;

use egui::{Label, RichText};
use projectcraft_engine::interchange::{ImportReport, Outcome};

use crate::theme::Tokens;
use crate::{App, i18n};

/// The name a path shows as: its file name, else the whole path.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

const WHERE_W: f32 = 190.0;
const WHY_W: f32 = 300.0;

impl App {
    /// The import report that shows now, if any: the imported file's name and its report.
    pub fn shown_import_report(&self) -> Option<&ImportReport> {
        self.import_report.as_ref().map(|(_, r)| r)
    }

    /// The Import Report dialog, while it is open.
    pub(crate) fn import_report_dialog(&mut self, ctx: &egui::Context) {
        let Some((file, report)) = self.import_report.as_ref() else { return };
        let t = Tokens::get(ctx);
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("import-report")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.heading(tl!("Import Report"));
            ui.label(i18n::fmt(
                tl!("{file}: {tasks} tasks imported as a new, untitled project. ProjectCraft worked out every date; none were taken from the file."),
                &[("file", file), ("tasks", &report.tasks.to_string())],
            ));
            ui.add_space(6.0);
            if report.lines.is_empty() {
                ui.label(tl!("Everything in the file was imported."));
            } else {
                ui.label(RichText::new(tl!("These parts of the file were converted, dropped, or aren't imported yet:")).color(t.text_dim));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Grid::new("import-report-lines").striped(true).num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                        for h in [tl!("Result"), tl!("Feature"), tl!("Where"), tl!("Why")] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for l in &report.lines {
                            let colour = match l.outcome {
                                Outcome::Converted => t.text,
                                Outcome::Dropped => t.warning,
                                Outcome::NotYetImported => t.text_dim,
                            };
                            ui.label(RichText::new(tl!(l.outcome.label())).color(colour));
                            ui.label(l.feature.as_str());
                            ui.scope(|ui| {
                                ui.set_max_width(WHERE_W);
                                ui.add(Label::new(l.place.as_str()).wrap());
                            });
                            ui.scope(|ui| {
                                ui.set_max_width(WHY_W);
                                ui.add(Label::new(RichText::new(l.why.as_str()).color(t.text_dim)).wrap());
                            });
                            ui.end_row();
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Copy Report")).on_hover_text(tl!("Copy the report as text")).clicked() {
                    ui.ctx().copy_text(report.text());
                }
                if ui.button(tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.import_report = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use projectcraft_engine::Session;
    use projectcraft_engine::model::Project;

    use crate::{App, FileRequest};

    const GARDEN: &str = include_str!("../../interchange/tests/fixtures/garden-studio.xml");
    const KITCHEN: &[u8] = include_bytes!("../../interchange/tests/fixtures/kitchen-refit.csv");

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("projectcraft-ui-import-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn harness() -> Harness<'static, App> {
        let app = App::new(Session::new(Project::new("Mine", projectcraft_engine::time::Date::EPOCH)), None);
        Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(|ui, app: &mut App| app.ui(ui), app)
    }

    /// Open of a `.xml` imports it as a new, untitled project and shows the report; Ctrl+S then
    /// asks for a `.projectcraft` name and never writes the `.xml`.
    #[test]
    fn opening_xml_imports_shows_the_report_and_save_asks_for_a_projectcraft_name() {
        let d = dir("open");
        let xml = d.join("Garden studio.xml");
        std::fs::write(&xml, GARDEN).unwrap();
        let mut h = harness();
        h.state_mut().open_path(&xml);
        h.run_steps(3);
        assert_eq!(h.state().session.project.name, "Garden studio");
        assert_eq!(h.state().path, None, "an import is untitled");
        assert!(h.state().is_dirty(), "an import is unsaved until saved as a project");
        assert_eq!(h.state().recent.first(), Some(&xml));
        h.get_by_label("Import Report");
        h.get_by_label("Links (predecessors)");
        // The dialog settles (wrapped text sized) over a couple of frames.
        h.run_steps(3);
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert!(h.state().shown_import_report().is_none());
        assert!(h.query_by_label("Import Report").is_none());

        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.run_steps(2);
        assert_eq!(h.state_mut().take_file_request(), Some(FileRequest::SaveAs));
        assert_eq!(h.state().save_as_name(), "Garden studio.projectcraft");
        // Picking the .xml (or any .xml) is refused: the file is left as it was.
        h.state_mut().file_picked(FileRequest::SaveAs, Some(xml.clone()));
        assert!(h.state().status().contains("Microsoft Project XML"), "{}", h.state().status());
        assert_eq!(std::fs::read_to_string(&xml).unwrap(), GARDEN);
        assert_eq!(h.state().path, None);
        h.state_mut().file_picked(FileRequest::SaveAs, Some(d.join("Garden studio")));
        assert_eq!(h.state().path.as_deref(), Some(d.join("Garden studio.projectcraft").as_path()));
        assert!(!h.state().is_dirty());
        assert_eq!(Session::open(&d.join("Garden studio.projectcraft")).unwrap().project.tasks.len(), 8);
        assert_eq!(std::fs::read_to_string(&xml).unwrap(), GARDEN);
    }

    /// Open of a `.csv` imports it the same way: a new, untitled project named after the file,
    /// the report, and Save asking for a `.projectcraft` name (a `.csv` one is refused).
    #[test]
    fn opening_csv_imports_shows_the_report_and_save_never_writes_the_csv() {
        let d = dir("csv");
        let csv = d.join("Kitchen refit.csv");
        std::fs::write(&csv, KITCHEN).unwrap();
        let mut h = harness();
        h.state_mut().open_path(&csv);
        h.run_steps(3);
        assert_eq!(h.state().session.project.name, "Kitchen refit");
        assert_eq!(h.state().session.project.tasks.len(), 6);
        assert_eq!(h.state().path, None);
        assert!(h.state().is_dirty());
        h.get_by_label("Import Report");
        h.get_by_label("Notes column");
        h.run_steps(3);
        h.get_by_label("Close").click();
        h.run_steps(3);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.run_steps(2);
        assert_eq!(h.state_mut().take_file_request(), Some(FileRequest::SaveAs));
        assert_eq!(h.state().save_as_name(), "Kitchen refit.projectcraft");
        h.state_mut().file_picked(FileRequest::SaveAs, Some(csv.clone()));
        assert!(h.state().status().contains("CSV"), "{}", h.state().status());
        assert_eq!(std::fs::read(&csv).unwrap(), KITCHEN);
        // Without a Task Name column the import is refused and the project stays.
        let bad = d.join("bad.csv");
        std::fs::write(&bad, "Name of task,Duration\nA,1\n").unwrap();
        h.state_mut().open_path(&bad);
        assert!(h.state().status().contains("Task Name"), "{}", h.state().status());
        assert_eq!(h.state().session.project.name, "Kitchen refit");
    }

    #[test]
    fn a_file_that_cannot_be_imported_leaves_the_project_and_says_why() {
        let d = dir("bad");
        let xml = d.join("laughs.xml");
        std::fs::write(&xml, "<!DOCTYPE Project [<!ENTITY a \"a\">]><Project>&a;</Project>").unwrap();
        let mut h = harness();
        h.state_mut().open_path(&xml);
        h.run_steps(2);
        assert_eq!(h.state().session.project.name, "Mine");
        assert!(h.state().status().contains("Couldn't import"), "{}", h.state().status());
        assert!(h.state().shown_import_report().is_none());
        assert!(!h.state().is_dirty());
    }
}
