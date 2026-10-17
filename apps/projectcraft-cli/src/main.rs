//! ProjectCraft command line.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::process::ExitCode;

use projectcraft_engine::Session;
use projectcraft_engine::model::{days_text, duration_text, preds_text};
use serde_json::Value;

const USAGE: &str = "\
usage:
  projectcraft-cli commands [--json]                         list every command, its effect and params
  projectcraft-cli run SOURCE [--cmd ID[=JSON]]... [--save OUT]
  projectcraft-cli schedule SOURCE                           print the task sheet with its dates

SOURCE is --sample, a .projectcraft FILE, or --import FILE: Microsoft Project XML or a .csv
(Task Name column required), imported as a new project (its import report goes to stderr; --save
writes a .projectcraft, never the import).
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

/// The session `args` name, and how many of them that took.
fn open(args: &[String]) -> Result<(Session, usize), String> {
    match args.first().map(String::as_str) {
        Some("--import") => {
            let f = args.get(1).ok_or_else(|| format!("--import needs a file\n\n{USAGE}"))?;
            let s = Session::import_file(std::path::Path::new(f), None).map_err(|e| e.to_string())?;
            if let Some(report) = s.import_report() {
                eprint!("{}", report.text());
            }
            Ok((s, 2))
        }
        source => Ok((open_one(source)?, 1)),
    }
}

fn open_one(source: Option<&str>) -> Result<Session, String> {
    match source {
        Some("--sample") => Ok(projectcraft_engine::sample()),
        Some(f) => {
            let path = std::path::Path::new(f);
            let s = Session::open(path).map_err(|e| e.to_string())?;
            sweep(path);
            Ok(s)
        }
        None => Err(USAGE.into()),
    }
}

/// Removes the temporary files that unfinished saves left beside `path`, after an open or a save.
/// Never fatal: one that can't be removed is left where it is, with one line on stderr.
fn sweep(path: &std::path::Path) {
    if projectcraft_engine::sweep_stale_temps(path).failed {
        eprintln!("warning: couldn't remove a temporary file left by an unfinished save beside {}", path.display());
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
            let (mut s, used) = open(args.get(1..).unwrap_or_default())?;
            let mut it = args.iter().skip(1 + used);
            while let Some(a) = it.next() {
                let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                match a.as_str() {
                    "--cmd" => {
                        let (id, params) = v.split_once('=').unwrap_or((v, "{}"));
                        let params = serde_json::from_str(params).map_err(|e| format!("{id}: params aren't JSON: {e}"))?;
                        s.run(id, &params).map_err(|e| format!("{id}: {e}"))?;
                    }
                    "--save" => {
                        let path = std::path::Path::new(v);
                        // A project is never written over (or as) Microsoft Project XML or CSV.
                        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("xml") || e.eq_ignore_ascii_case("csv")) {
                            return Err(format!("--save {v}: ProjectCraft saves .projectcraft files, not Microsoft Project XML or CSV"));
                        }
                        s.save_to(path).map_err(|e| e.to_string())?;
                        sweep(path);
                    }
                    _ => return Err(format!("unknown option {a}\n\n{USAGE}")),
                }
            }
            Ok(())
        }
        Some("schedule") => {
            let (s, _) = open(args.get(1..).unwrap_or_default())?;
            let sched = s.schedule.as_ref().map_err(|e| format!("can't schedule: {e}"))?;
            println!("{}  (start {}, finish {})", s.project.name, s.project.start, sched.finish);
            println!(
                "{:>4}  {:<28} {:>5}  {:<10}  {:<10}  {:>5}  {:<12} {:>12}  resources",
                "UID", "Task Name", "Dur", "Start", "Finish", "Slack", "preds", "Cost"
            );
            let costs = s.project.cost_by_task();
            let levels = projectcraft_engine::model::Outline::of(&s.project).levels;
            // Durations and slack are working minutes; shown in days, as entered.
            let day = s.project.minutes_per_day();
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
                let line = format!(
                    "{:>4}{crit} {:<28} {:>6}  {}  {}  {:>5}{warn} {:<12} {:>12}  {}",
                    t.id,
                    name,
                    duration_text(t.duration, t.duration_unit, &s.project.options),
                    d.start,
                    d.finish,
                    days_text(d.slack, day),
                    preds_text(&t.preds, &s.project.options),
                    s.project.currency.format(costs.get(&t.id).copied().unwrap_or(0)),
                    s.project.resource_names(t.id)
                );
                println!("{}", line.trim_end());
            }
            println!("Total cost {}", s.project.currency.format(s.project.total_cost()));
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
