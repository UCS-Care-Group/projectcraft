//! ProjectCraft command line.

use std::process::ExitCode;

use projectcraft_engine::{Session, model::preds_text};
use serde_json::Value;

const USAGE: &str = "\
usage:
  projectcraft-cli commands [--json]                         list every command, its effect and params
  projectcraft-cli run (--sample | FILE) [--cmd ID[=JSON]]... [--save OUT]
  projectcraft-cli schedule (--sample | FILE)                print the task sheet with its dates
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
        Some("--sample") => Ok(projectcraft_engine::sample()),
        Some(f) => Session::open(std::path::Path::new(f)).map_err(|e| e.to_string()),
        None => Err(USAGE.into()),
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("commands") if args.get(1).is_some_and(|a| a == "--json") => {
            println!("{}", serde_json::to_string_pretty(&projectcraft_engine::describe()).map_err(|e| e.to_string())?);
            Ok(())
        }
        Some("commands") => {
            for c in projectcraft_engine::describe().as_array().into_iter().flatten() {
                let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
                println!("{:<22} {:<20} {:<12} {}", s(&c["id"]), s(&c["label"]), s(&c["effect"]), params_summary(&c["params"]));
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
                    "--save" => s.save_to(std::path::Path::new(v)).map_err(|e| e.to_string())?,
                    _ => return Err(format!("unknown option {a}\n\n{USAGE}")),
                }
            }
            Ok(())
        }
        Some("schedule") => {
            let s = open(args.get(1))?;
            let sched = s.schedule.as_ref().map_err(|e| format!("can't schedule: {e}"))?;
            println!("{}  (start {}, finish {})", s.project.name, s.project.start, sched.finish);
            println!("{:>4}  {:<28} {:>5}  {:<10}  {:<10}  {:>5}  preds", "UID", "Task Name", "Dur", "Start", "Finish", "Slack");
            let levels = projectcraft_engine::model::Outline::of(&s.project).levels;
            for (t, level) in s.project.tasks.iter().zip(levels) {
                let Some(d) = sched.tasks.get(&t.id) else { continue };
                let crit = if d.critical { "*" } else { " " };
                // `!`: a constraint overrides a link (constraint conflict); `>`: it finishes after its deadline.
                let warn = if d.constraint_conflict {
                    "!"
                } else if d.deadline_missed {
                    ">"
                } else {
                    " "
                };
                // Subtasks indent two spaces a level.
                let name = format!("{}{}", "  ".repeat(level.saturating_sub(1).min(20) as usize), t.name);
                println!(
                    "{:>4}{crit} {:<28} {:>4}d  {}  {}  {:>5}{warn} {}",
                    t.id,
                    name,
                    t.duration,
                    d.start,
                    d.finish,
                    d.slack,
                    preds_text(&t.preds)
                );
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// `id: integer 1..4294967295, name?: string` from a params schema; `--json` gives the full schema.
fn params_summary(schema: &Value) -> String {
    let required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let props = schema["properties"].as_object().into_iter().flatten();
    let parts: Vec<String> = props
        .map(|(k, p)| {
            let opt = if required.contains(&k.as_str()) { "" } else { "?" };
            let ty = p["description"].as_str().or(p["type"].as_str()).unwrap_or("any");
            let range = match (p["minimum"].as_i64(), p["maximum"].as_i64()) {
                (Some(a), Some(b)) => format!(" {a}..{b}"),
                _ => String::new(),
            };
            format!("{k}{opt}: {ty}{range}")
        })
        .collect();
    format!("{{{}}}", parts.join(", "))
}
