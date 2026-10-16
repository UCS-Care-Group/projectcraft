//! ProjectCraft desktop app. `projectcraft [file.projectcraft] [--sample]`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use projectcraft_engine::Session;
use projectcraft_ui_egui::App;

mod dialogs;
mod settings;

struct Shell {
    app: App,
    dialogs: dialogs::Dialogs,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        if let Some((request, path)) = self.dialogs.answer() {
            self.app.file_picked(request, path);
        }
        self.app.ui(ui);
        if !self.dialogs.busy()
            && let Some(request) = self.app.take_file_request()
        {
            self.dialogs.show(request, &self.app, frame, ui.ctx());
        }
        if let Some(recent) = self.app.take_recent_changed() {
            settings::save_recent(recent);
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().find(|a| !a.starts_with("--")).map(std::path::PathBuf::from);
    let sample = args.iter().any(|a| a == "--sample");
    let session = match &path {
        _ if sample => projectcraft_engine::sample(),
        Some(p) if p.exists() => match Session::open(p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", p.display());
                std::process::exit(1);
            }
        },
        _ => Session::new(projectcraft_engine::model::Project::new("New project", today())),
    };
    let mut app = App::new(session, if sample { None } else { path.clone() });
    app.recent = settings::load_recent();
    if let Some(p) = path.filter(|p| !sample && p.exists()) {
        projectcraft_ui_egui::push_recent(&mut app.recent, &p);
        settings::save_recent(&app.recent);
    }
    eframe::run_native(
        "ProjectCraft",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(Shell { app, dialogs: dialogs::Dialogs::default() }))),
    )
}

fn today() -> projectcraft_engine::time::Date {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    projectcraft_engine::time::Date::EPOCH.add_days(days as i64).unwrap_or(projectcraft_engine::time::Date::EPOCH)
}
