//! Headless UI screenshots: renders the whole ProjectCraft window offscreen (wgpu), so it works
//! with a locked screen or no display at all. As MailCraft's and WordCraft's.
//!
//! `cargo run -p projectcraft-ui-egui --example ui_shot -- out_dir` writes `gantt-light.png` and
//! `gantt-dark.png` of the sample project's Gantt Chart into `out_dir`.
//!
//! `cargo run -p projectcraft-ui-egui --example ui_shot -- script.jsonl` runs a script of JSON
//! lines instead: `{"cmd": "task.add", "params": {…}}` runs a command (as the CLI's `--cmd`),
//! `{"theme": "light" | "dark"}` switches the theme, `{"shot": "/abs/out.png"}` saves the window
//! as PNG, `{"steps": n}` runs extra frames. The window is 1280×720 pt at 2× (override with
//! `{"width": w, "height": h}` and `{"scale": s}` on any line) and opens the sample project
//! unless the first line is `{"empty": true}`.

use egui::ThemePreference;
use projectcraft_engine::Session;
use projectcraft_ui_egui::App;
use serde_json::{Value, json};

fn main() {
    if let Err(e) = run() {
        eprintln!("ui_shot: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arg = std::env::args().nth(1).ok_or("usage: ui_shot out_dir | script.jsonl")?;
    let lines: Vec<Value> = if arg.ends_with(".jsonl") {
        let script = std::fs::read_to_string(&arg).map_err(|e| format!("{arg}: {e}"))?;
        script.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).map_err(|e| format!("{l}: {e}"))).collect::<Result<_, _>>()?
    } else {
        let dir = std::path::Path::new(&arg);
        std::fs::create_dir_all(dir).map_err(|e| format!("{arg}: {e}"))?;
        let shot = |name: &str| dir.join(name).display().to_string();
        vec![json!({"theme": "light"}), json!({"shot": shot("gantt-light.png")}), json!({"theme": "dark"}), json!({"shot": shot("gantt-dark.png")})]
    };
    let number = |key: &str, default: f64| lines.iter().find_map(|l| l.get(key).and_then(Value::as_f64)).unwrap_or(default) as f32;
    let session = if lines.first().is_some_and(|l| l.get("empty").is_some()) {
        Session::new(projectcraft_engine::model::Project::new("New project", projectcraft_engine::time::Date::EPOCH))
    } else {
        projectcraft_engine::sample()
    };
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(number("width", 1280.0), number("height", 720.0)))
        .with_pixels_per_point(number("scale", 2.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut App| app.ui(ui), App::new(session, None));
    harness.run_steps(4);
    for l in lines {
        if let Some(theme) = l.get("theme").and_then(Value::as_str) {
            harness.state_mut().appearance = if theme == "dark" { ThemePreference::Dark } else { ThemePreference::Light };
            harness.run_steps(4);
        } else if let Some(id) = l.get("cmd").and_then(Value::as_str) {
            let params = l.get("params").cloned().unwrap_or_else(|| json!({}));
            if let Err(e) = harness.state_mut().session.run(id, &params) {
                println!("{id}: {e}");
            }
            harness.run_steps(2);
        } else if let Some(n) = l.get("steps").and_then(Value::as_u64) {
            harness.run_steps(n.min(10_000) as usize);
        } else if let Some(p) = l.get("shot").and_then(Value::as_str) {
            harness.run_steps(4);
            let img = harness.render().map_err(|e| format!("render failed: {e}"))?;
            img.save(p).map_err(|e| format!("{p}: {e}"))?;
            println!("{p}");
        }
    }
    Ok(())
}
