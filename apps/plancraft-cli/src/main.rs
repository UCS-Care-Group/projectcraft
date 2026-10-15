//! PlanCraft command line.

use std::process::ExitCode;

use plancraft_engine::{Session, model::preds_text};

const USAGE: &str = "\
usage:
  plancraft-cli commands                                  list every command
  plancraft-cli run (--sample | FILE) [--cmd ID[=JSON]]... [--save OUT]
  plancraft-cli schedule (--sample | FILE)                print the task sheet with its dates
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(source: Option<&String>) -> Result<Session, String> {
    match source.map(String::as_str) {
        Some("--sample") => Ok(plancraft_engine::sample()),
        Some(f) => Session::load_json(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?).map_err(|e| e.to_string()),
        None => Err(USAGE.into()),
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("commands") => {
            for c in plancraft_engine::describe().as_array().into_iter().flatten() {
                println!("{:<24} {:<20} {}", c["id"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""), c["params"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        Some("run") => {
            let mut s = open(args.get(1))?;
            let mut it = args.iter().skip(2);
            while let Some(a) = it.next() {
                let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                match a.as_str() {
                    "--cmd" => {
                        let (id, params) = v.split_once('=').unwrap_or((v, "{}"));
                        let params = serde_json::from_str(params).map_err(|e| format!("{id}: params aren't JSON: {e}"))?;
                        s.run(id, &params).map_err(|e| format!("{id}: {e}"))?;
                    }
                    "--save" => std::fs::write(v, s.save_json().map_err(|e| e.to_string())?).map_err(|e| format!("{v}: {e}"))?,
                    _ => return Err(format!("unknown option {a}\n\n{USAGE}")),
                }
            }
            Ok(())
        }
        Some("schedule") => {
            let s = open(args.get(1))?;
            let sched = s.schedule.as_ref().map_err(|e| format!("can't schedule: {e}"))?;
            println!("{}  (start {}, finish {})", s.project.name, s.project.start, sched.finish);
            println!("{:>4}  {:<28} {:>5}  {:<10}  {:<10}  {:>5}  preds", "ID", "Task Name", "Dur", "Start", "Finish", "Slack");
            for t in &s.project.tasks {
                let Some(d) = sched.tasks.get(&t.id) else { continue };
                let crit = if d.critical { "*" } else { " " };
                println!("{:>4}{crit} {:<28} {:>4}d  {}  {}  {:>5}  {}", t.id, t.name, t.duration, d.start, d.finish, d.slack, preds_text(&t.preds));
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
