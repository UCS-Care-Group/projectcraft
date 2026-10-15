//! PlanCraft desktop app. `plancraft [file.plancraft] [--sample]`.

use plancraft_engine::Session;
use plancraft_ui_egui::App;

struct Shell(App);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().find(|a| !a.starts_with("--")).map(std::path::PathBuf::from);
    let session = match &path {
        _ if args.iter().any(|a| a == "--sample") => plancraft_engine::sample(),
        Some(p) if p.exists() => {
            match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| Session::load_json(&t).map_err(|e| e.to_string())) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{}: {e}", p.display());
                    std::process::exit(1);
                }
            }
        }
        _ => Session::new(plancraft_engine::model::Project::new("New project", today())),
    };
    eframe::run_native("PlanCraft", eframe::NativeOptions::default(), Box::new(|_| Ok(Box::new(Shell(App::new(session, path))))))
}

fn today() -> plancraft_engine::time::Date {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    plancraft_engine::time::Date::EPOCH.add_days(days as i64).unwrap_or(plancraft_engine::time::Date::EPOCH)
}
