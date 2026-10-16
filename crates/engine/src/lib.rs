//! ProjectCraft engine: the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`task.add`, `task.setPredecessors`), a label, a JSON Schema for its params, an [`Effect`] and a
//! `run` function. The toolbar, the task sheet, the CLI (and later the control channel and MCP
//! server) all call [`Session::run`], which checks params against the schema first (ADR 0004).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file;

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

pub use projectcraft_model as model;
pub use projectcraft_time as time;

pub use file::FORMAT_VERSION;

use model::{Date, ModelError, Project, Schedule, TaskId};

#[derive(Debug, thiserror::Error)]
pub enum CmdError {
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("bad params: {0}")]
    Params(String),
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("{0}")]
    Io(String),
}

/// What a command does to the world, so callers (MCP, `--read-only`, confirmations) can gate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Looks only.
    Read,
    /// Changes the open project; undoable.
    Edit,
    /// Removes the owner's work (deleting tasks); undoable, but needs confirmation from agents.
    Destructive,
    /// Touches files outside the open project.
    File,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Edit => "edit",
            Effect::Destructive => "destructive",
            Effect::File => "file",
        }
    }
}

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub effect: Effect,
    /// JSON Schema for the params: an object with `properties`, `required` and
    /// `additionalProperties: false`. Only what [`check_params`] understands.
    pub schema: fn() -> Value,
    pub run: fn(&mut Project, &Value) -> Result<(), CmdError>,
}

/// One project, its undo history, and the schedule computed from it.
pub struct Session {
    pub project: Project,
    pub schedule: Result<Schedule, ModelError>,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

/// ponytail: whole-project snapshots for undo; switch to edit records when projects get big.
const UNDO_DEPTH: usize = 200;

impl Session {
    pub fn new(project: Project) -> Self {
        let schedule = model::schedule(&project);
        Session { project, schedule, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Runs a command. On error, or if it panics, the project is left exactly as it was.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<(), CmdError> {
        let spec = COMMANDS.iter().find(|c| c.id == id).ok_or_else(|| CmdError::Unknown(id.chars().take(60).collect()))?;
        check_params(&(spec.schema)(), params)?;
        match id {
            "edit.undo" => return self.step(true),
            "edit.redo" => return self.step(false),
            _ => {}
        }
        let before = self.project.clone();
        let result = catch_unwind(AssertUnwindSafe(|| (spec.run)(&mut self.project, params)))
            .unwrap_or_else(|_| Err(CmdError::Params(format!("{id} failed unexpectedly; nothing was changed"))));
        match result {
            Ok(()) if self.project != before => {
                self.undo.push(before);
                if self.undo.len() > UNDO_DEPTH {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.schedule = model::schedule(&self.project);
                Ok(())
            }
            Ok(()) => Ok(()),
            Err(e) => {
                self.project = before;
                Err(e)
            }
        }
    }

    fn step(&mut self, undo: bool) -> Result<(), CmdError> {
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        if let Some(p) = from.pop() {
            to.push(std::mem::replace(&mut self.project, p));
            self.schedule = model::schedule(&self.project);
        }
        Ok(())
    }
}

pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn describe() -> Value {
    COMMANDS.iter().map(|c| json!({"id": c.id, "label": c.label, "effect": c.effect.as_str(), "params": (c.schema)()})).collect()
}

/// A params schema: an object with these properties, these required, and nothing else.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn no_params() -> Value {
    object(json!({}), &[])
}

fn task_id() -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": u32::MAX, "description": "the task's Unique ID (not its row ID)"})
}

fn days() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": model::MAX_DAYS, "description": "working days"})
}

/// Checks params against a command schema. Supports the subset our schemas use: an object with
/// `properties`, `required` and `additionalProperties: false`; property `type` integer, number,
/// string, boolean, array or object; `minimum`/`maximum`; `enum` of strings. `null` params count as `{}`.
pub fn check_params(schema: &Value, params: &Value) -> Result<(), CmdError> {
    let empty = serde_json::Map::new();
    let given = match params {
        Value::Null => &empty,
        Value::Object(m) => m,
        _ => return Err(CmdError::Params("params must be a JSON object".into())),
    };
    let props = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
    for (k, v) in given {
        let p = props.get(k).ok_or_else(|| {
            let known: Vec<&str> = props.keys().map(String::as_str).collect();
            let k: String = k.chars().take(60).collect();
            CmdError::Params(if known.is_empty() {
                format!("unknown param `{k}`; this command takes none")
            } else {
                format!("unknown param `{k}`; expected {}", known.join(", "))
            })
        })?;
        check_value(k, p, v)?;
    }
    for r in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if !given.contains_key(r) {
            return Err(CmdError::Params(format!("`{r}` is required")));
        }
    }
    Ok(())
}

fn check_value(k: &str, p: &Value, v: &Value) -> Result<(), CmdError> {
    let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
    let ok = match ty {
        "integer" => v.as_i64().is_some(),
        "number" => v.is_number(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    };
    if !ok {
        let what = match ty {
            "integer" => "a whole number",
            "string" => "text",
            _ => ty,
        };
        return Err(CmdError::Params(format!("`{k}` must be {what}")));
    }
    if let Some(allowed) = p.get("enum").and_then(Value::as_array)
        && !allowed.contains(v)
    {
        let names: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
        return Err(CmdError::Params(format!("`{k}` must be one of {}", names.join(", "))));
    }
    if let Some(n) = v.as_f64() {
        let min = p.get("minimum").and_then(Value::as_f64);
        let max = p.get("maximum").and_then(Value::as_f64);
        if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
            let range = |o: Option<f64>| o.map_or_else(String::new, |m| m.to_string());
            return Err(CmdError::Params(format!("`{k}` must be between {} and {}", range(min), range(max))));
        }
    }
    Ok(())
}

fn str_param<'a>(v: &'a Value, k: &str) -> Result<&'a str, CmdError> {
    v.get(k).and_then(Value::as_str).ok_or_else(|| CmdError::Params(format!("`{k}` (text) is required")))
}

fn int_param(v: &Value, k: &str) -> Result<i64, CmdError> {
    v.get(k).and_then(Value::as_i64).ok_or_else(|| CmdError::Params(format!("`{k}` (whole number) is required")))
}

fn id_param(v: &Value) -> Result<TaskId, CmdError> {
    let n = int_param(v, "id")?;
    u32::try_from(n).map(TaskId).map_err(|_| CmdError::Params(format!("no task {n}")))
}

/// Names are one line and of a sensible length.
fn name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(255).collect()
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "project.rename",
        label: "Rename Project",
        effect: Effect::Edit,
        schema: || object(json!({"name": {"type": "string"}}), &["name"]),
        run: |p, v| {
            p.name = name(str_param(v, "name")?);
            Ok(())
        },
    },
    CommandSpec {
        id: "project.setStart",
        label: "Project Start Date",
        effect: Effect::Edit,
        schema: || object(json!({"date": {"type": "string", "format": "date", "description": "YYYY-MM-DD"}}), &["date"]),
        run: |p, v| {
            p.start = str_param(v, "date")?.parse::<Date>().map_err(|e| CmdError::Params(e.to_string()))?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.add",
        label: "New Task",
        effect: Effect::Edit,
        schema: || object(json!({"name": {"type": "string"}, "duration": days()}), &[]),
        run: |p, v| {
            let n = v.get("name").and_then(Value::as_str).map_or_else(|| "New task".to_owned(), name);
            p.add_task(n, v.get("duration").and_then(Value::as_i64).unwrap_or(1))?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.rename",
        label: "Task Name",
        effect: Effect::Edit,
        schema: || object(json!({"id": task_id(), "name": {"type": "string"}}), &["id", "name"]),
        run: |p, v| {
            p.task_mut(id_param(v)?)?.name = name(str_param(v, "name")?);
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setDuration",
        label: "Duration",
        effect: Effect::Edit,
        schema: || object(json!({"id": task_id(), "duration": days()}), &["id", "duration"]),
        run: |p, v| {
            p.task_mut(id_param(v)?)?.duration = model::check_days(int_param(v, "duration")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setPredecessors",
        label: "Predecessors",
        effect: Effect::Edit,
        schema: || {
            object(json!({"id": task_id(), "text": {"type": "string", "description": "predecessors by Unique ID, e.g. 3, 5SS+2"}}), &["id", "text"])
        },
        run: |p, v| {
            let links = model::parse_preds(str_param(v, "text")?)?;
            p.set_preds(id_param(v)?, links)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setConstraint",
        label: "Constraint",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "id": task_id(),
                    "type": {"type": "string", "enum": model::Constraint::CODES, "description": "ASAP clears; SNET/FNET Start/Finish No Earlier Than; SNLT/FNLT Start/Finish No Later Than; MSO Must Start On; MFO Must Finish On. Constraints win over links."},
                    "date": {"type": "string", "format": "date", "description": "YYYY-MM-DD; required except for ASAP"},
                }),
                &["id", "type"],
            )
        },
        run: |p, v| {
            let date = match v.get("date").and_then(Value::as_str) {
                Some(d) => Some(d.parse::<Date>().map_err(|e| CmdError::Params(e.to_string()))?),
                None => None,
            };
            let c = model::Constraint::from_code(str_param(v, "type")?, date)?;
            p.task_mut(id_param(v)?)?.constraint = c;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.delete",
        label: "Delete Task",
        effect: Effect::Destructive,
        schema: || object(json!({"id": task_id()}), &["id"]),
        run: |p, v| {
            p.delete_task(id_param(v)?)?;
            Ok(())
        },
    },
    // Undo and redo are handled by `Session::run`; they're here for their metadata.
    CommandSpec { id: "edit.undo", label: "Undo", effect: Effect::Edit, schema: no_params, run: |_, _| Ok(()) },
    CommandSpec { id: "edit.redo", label: "Redo", effect: Effect::Edit, schema: no_params, run: |_, _| Ok(()) },
];

/// A small invented project to try things on.
pub fn sample() -> Session {
    let mut s = Session::new(Project::new("Office move", Date::from_ymd(2026, 11, 2).unwrap_or(Date::EPOCH)));
    let rows: &[(&str, i64, &str)] = &[
        ("Agree the brief", 2, ""),
        ("Survey the new floor", 3, "1"),
        ("Order furniture", 1, "1"),
        ("Furniture delivered", 10, "3"),
        ("Network cabling", 5, "2"),
        ("Pack the old office", 2, "4FS-1, 5"),
        ("Move day", 1, "6"),
        ("Settled in", 0, "7"),
    ];
    for (n, d, preds) in rows {
        let _ = s.run("task.add", &json!({"name": n, "duration": d}));
        let id = s.project.tasks.last().map_or(0, |t| t.id.0);
        let _ = s.run("task.setPredecessors", &json!({"id": id, "text": preds}));
    }
    // The sample opens with nothing to undo.
    s.undo.clear();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_undo_and_failed_commands_change_nothing() {
        let mut s = sample();
        assert!(s.schedule.is_ok());
        assert!(!s.can_undo());
        let before = s.project.clone();
        s.run("task.setDuration", &json!({"id": 1, "duration": 5})).unwrap();
        assert_ne!(s.project, before);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.project, before);
        s.run("edit.redo", &json!({})).unwrap();
        assert!(s.run("task.setPredecessors", &json!({"id": 1, "text": "8"})).is_err());
        assert!(s.run("nope", &json!({})).is_err());
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
    }

    #[test]
    fn every_command_has_a_closed_schema_and_unknown_params_are_refused() {
        for c in commands() {
            let schema = (c.schema)();
            assert_eq!(schema["type"], "object", "{}", c.id);
            assert_eq!(schema["additionalProperties"], false, "{}", c.id);
            assert!(schema["properties"].is_object(), "{}", c.id);
        }
        let listed = describe();
        let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"edit.undo") && ids.contains(&"task.delete"));
        assert_eq!(describe()[0]["effect"], "edit");
        assert_eq!(commands().iter().find(|c| c.id == "task.delete").unwrap().effect, Effect::Destructive);

        let mut s = sample();
        let before = s.project.clone();
        let e = s.run("task.add", &json!({"name": "x", "durration": 3})).unwrap_err().to_string();
        assert!(e.contains("durration"), "{e}");
        assert!(s.run("edit.undo", &json!({"steps": 2})).is_err());
        assert!(s.run("task.setDuration", &json!({"id": 1})).unwrap_err().to_string().contains("`duration` is required"));
        assert!(s.run("task.setDuration", &json!({"id": 1, "duration": "5"})).is_err());
        assert!(s.run("task.setDuration", &json!({"id": 1, "duration": model::MAX_DAYS + 1})).is_err());
        let e = s.run("task.setConstraint", &json!({"id": 1, "type": "SOON"})).unwrap_err().to_string();
        assert!(e.contains("must be one of ASAP, SNET"), "{e}");
        assert_eq!(s.project, before);
        s.run("task.add", &json!(null)).unwrap();
        s.run("task.add", &json!({"name": "Snag list", "duration": 3})).unwrap();
    }

    #[test]
    fn commands_take_unique_ids_that_survive_deletes_and_save() {
        for c in commands() {
            for (k, p) in (c.schema)()["properties"].as_object().unwrap() {
                if k == "id" || k == "text" {
                    assert!(p["description"].as_str().unwrap().contains("Unique ID"), "{}.{k}", c.id);
                }
            }
        }
        let mut s = sample();
        s.run("task.delete", &json!({"id": 1})).unwrap();
        // Task 3 is now on row 2; commands still name it 3, and links name Unique IDs.
        assert_eq!(s.project.row_of(TaskId(3)), Some(2));
        s.run("task.setPredecessors", &json!({"id": 4, "text": "3SS+1"})).unwrap();
        assert_eq!(s.project.preds_text_rows(&s.project.task(TaskId(4)).unwrap().preds), "2SS+1");
        s.run("task.add", &json!({})).unwrap();
        assert_eq!(s.project.tasks.last().unwrap().id, TaskId(9), "a deleted Unique ID is never reused");
        let mut round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
        round.run("task.add", &json!({})).unwrap();
        assert_eq!(round.project.tasks.last().unwrap().id, TaskId(10));
    }

    /// Valid params from a schema, then junk built from it: each property wrong-typed, out of
    /// range or huge, each required one missing, an unknown one added, and non-object params.
    fn junk_from(schema: &Value) -> Vec<Value> {
        let props = schema["properties"].as_object().cloned().unwrap_or_default();
        let good = |p: &Value| match p["type"].as_str() {
            _ if p["enum"].is_array() => p["enum"][1].clone(),
            Some("integer") | Some("number") => p["minimum"].clone(),
            Some("string") if p["format"] == "date" => json!("2026-11-02"),
            _ => json!("1"),
        };
        let valid: serde_json::Map<String, Value> = props.iter().map(|(k, p)| (k.clone(), good(p))).collect();
        let mut out = vec![json!(null), json!([]), json!("x"), json!(7), Value::Object(valid.clone())];
        let wrong = [
            json!(null),
            json!("x"),
            json!(1.5),
            json!(true),
            json!([1]),
            json!({}),
            json!(-1),
            json!(i64::MIN),
            json!(i64::MAX),
            json!(u64::MAX),
            json!(1e308),
            json!("0000-00-00"),
            json!("1,,x"),
            json!("9999SS+999999"),
        ];
        for (k, p) in &props {
            let mut edge = wrong.to_vec();
            for b in ["minimum", "maximum"] {
                if let Some(n) = p[b].as_i64() {
                    edge.extend([json!(n.saturating_sub(1)), json!(n), json!(n.saturating_add(1))]);
                }
            }
            for w in edge {
                let mut m = valid.clone();
                m.insert(k.clone(), w);
                out.push(Value::Object(m));
            }
            let mut m = valid.clone();
            m.remove(k);
            out.push(Value::Object(m));
        }
        let mut m = valid;
        m.insert("durration".into(), json!(3));
        out.push(Value::Object(m));
        out
    }

    #[test]
    fn hostile_params_never_panic() {
        for c in commands() {
            for j in junk_from(&(c.schema)()) {
                let mut s = sample();
                let before = s.project.clone();
                if s.run(c.id, &j).is_err() {
                    assert_eq!(s.project, before, "{} changed the project on error with {j}", c.id);
                }
            }
        }
        assert!(Session::load_json("{").is_err());
    }
}
