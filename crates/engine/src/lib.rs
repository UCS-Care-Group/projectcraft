//! ProjectCraft engine: the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`task.add`, `task.setPredecessors`), a label, a params description and a `run` function. The
//! toolbar, the task sheet, the CLI (and later the control channel and MCP server) all call
//! [`Session::run`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

pub use projectcraft_model as model;
pub use projectcraft_time as time;

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

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Human-readable params, for `commands` listings and agents.
    pub params: &'static str,
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
        match id {
            "edit.undo" => return self.step(true),
            "edit.redo" => return self.step(false),
            _ => {}
        }
        let spec = COMMANDS.iter().find(|c| c.id == id).ok_or_else(|| CmdError::Unknown(id.chars().take(60).collect()))?;
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

    pub fn save_json(&self) -> Result<String, CmdError> {
        serde_json::to_string_pretty(&self.project).map_err(|e| CmdError::Io(e.to_string()))
    }

    pub fn load_json(text: &str) -> Result<Session, CmdError> {
        let p: Project = serde_json::from_str(text).map_err(|e| CmdError::Io(format!("not a ProjectCraft project: {e}")))?;
        if p.tasks.len() > model::MAX_TASKS {
            return Err(CmdError::Io(format!("more than {} tasks", model::MAX_TASKS)));
        }
        Ok(Session::new(p))
    }
}

pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn describe() -> Value {
    COMMANDS
        .iter()
        .map(|c| json!({"id": c.id, "label": c.label, "params": c.params}))
        .chain([json!({"id": "edit.undo", "label": "Undo", "params": ""}), json!({"id": "edit.redo", "label": "Redo", "params": ""})])
        .collect()
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
        params: r#"{"name": text}"#,
        run: |p, v| {
            p.name = name(str_param(v, "name")?);
            Ok(())
        },
    },
    CommandSpec {
        id: "project.setStart",
        label: "Project Start Date",
        params: r#"{"date": "YYYY-MM-DD"}"#,
        run: |p, v| {
            p.start = str_param(v, "date")?.parse::<Date>().map_err(|e| CmdError::Params(e.to_string()))?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.add",
        label: "New Task",
        params: r#"{"name"?: text, "duration"?: working days}"#,
        run: |p, v| {
            let n = v.get("name").and_then(Value::as_str).map_or_else(|| "New task".to_owned(), name);
            p.add_task(n, v.get("duration").and_then(Value::as_i64).unwrap_or(1))?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.rename",
        label: "Task Name",
        params: r#"{"id": task, "name": text}"#,
        run: |p, v| {
            p.task_mut(id_param(v)?)?.name = name(str_param(v, "name")?);
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setDuration",
        label: "Duration",
        params: r#"{"id": task, "duration": working days}"#,
        run: |p, v| {
            p.task_mut(id_param(v)?)?.duration = model::check_days(int_param(v, "duration")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setPredecessors",
        label: "Predecessors",
        params: r#"{"id": task, "text": "3, 5SS+2"}"#,
        run: |p, v| {
            let links = model::parse_preds(str_param(v, "text")?)?;
            p.set_preds(id_param(v)?, links)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.delete",
        label: "Delete Task",
        params: r#"{"id": task}"#,
        run: |p, v| {
            p.delete_task(id_param(v)?)?;
            Ok(())
        },
    },
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
    fn hostile_params_never_panic() {
        let junk = [
            json!(null),
            json!({}),
            json!({"id": -1, "name": 3}),
            json!({"id": u64::MAX, "duration": i64::MIN, "text": "1,,x"}),
            json!({"date": "0000-00-00"}),
        ];
        for c in commands() {
            for j in &junk {
                let mut s = sample();
                let before = s.project.clone();
                if s.run(c.id, j).is_err() {
                    assert_eq!(s.project, before, "{} changed the project on error", c.id);
                }
            }
        }
        assert!(Session::load_json("{").is_err());
    }
}
