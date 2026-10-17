//! ProjectCraft desktop app. `projectcraft [file.projectcraft | plan.xml | plan.csv] [--sample]`;
//! a `.xml` (Microsoft Project XML) or `.csv` is imported as a new, untitled project.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use projectcraft_engine::Session;
use projectcraft_ui_egui::App;

mod dialogs;
mod quit;
mod settings;

struct Shell {
    app: App,
    dialogs: dialogs::Dialogs,
    /// The only clock the app reads for autosave and the recovery copy ([`App::tick`]).
    started: std::time::Instant,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        match self.dialogs.answer() {
            Some((request, dialogs::Answer::Picked(path))) => self.app.file_picked(request, path),
            Some((request, dialogs::Answer::Unavailable)) => self.app.file_unavailable(request),
            None => {}
        }
        self.app.tick(self.started.elapsed());
        // Wake now and then so autosave and the recovery copy run while the window is idle.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(5));
        self.app.ui(ui);
        if !self.dialogs.busy()
            && let Some(request) = self.app.take_file_request()
        {
            self.dialogs.show(request, &self.app, frame, ui.ctx());
        }
        if let Some(recent) = self.app.take_recent_changed() {
            settings::save_recent(recent);
        }
        if let Some(on) = self.app.take_autosave_changed() {
            settings::save_autosave(on);
        }
    }

    /// Runs before `ui`, and also while the window is minimized or hidden, when `ui` does not:
    /// the unsaved-changes check first, then the platform quit.
    fn logic(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        quit::finish_frame(ctx);
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().find(|a| !a.starts_with("--")).map(std::path::PathBuf::from);
    let sample = args.iter().any(|a| a == "--sample");
    // Microsoft Project XML and CSV are imported as a new, untitled project; the window opens on its report.
    let (import, path) = match path {
        Some(p) if projectcraft_ui_egui::is_import(&p) && !sample => (Some(p), None),
        other => (None, other),
    };
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
    // `--sample` keeps nothing: no autosave, no recovery copy.
    if !sample {
        app.keep_recovery(settings::recovery_dir(), settings::load_autosave());
    }
    if let Some(p) = path.filter(|p| !sample && p.exists()) {
        projectcraft_ui_egui::push_recent(&mut app.recent, &p);
        settings::save_recent(&app.recent);
    }
    if let Some(p) = import {
        app.import_path(&p);
        if app.shown_import_report().is_none() {
            eprintln!("{}: {}", p.display(), app.status());
            std::process::exit(1);
        }
    }
    eframe::run_native(
        "ProjectCraft",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            // macOS: an app menu whose Quit (Cmd+Q) asks about unsaved changes first; nothing
            // elsewhere. Without it Cmd+Q still quits, only without asking, so the app runs on.
            if let Err(e) = craft_quit::install_app_menu(&cc.egui_ctx, "ProjectCraft") {
                eprintln!("app menu: {e}");
            }
            Ok(Box::new(Shell { app, dialogs: dialogs::Dialogs::default(), started: std::time::Instant::now() }))
        }),
    )
}

fn today() -> projectcraft_engine::time::Date {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    projectcraft_engine::time::Date::EPOCH.add_days(days as i64).unwrap_or(projectcraft_engine::time::Date::EPOCH)
}
