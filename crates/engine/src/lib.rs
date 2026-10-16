//! ProjectCraft engine: the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`task.add`, `task.setPredecessors`), a label, a JSON Schema for its params, an [`Effect`] and a
//! `run` function. The toolbar, the task sheet, the CLI (and later the control channel and MCP
//! server) all call [`Session::run`], which checks params against the schema first.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file;

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

pub use projectcraft_model as model;
pub use projectcraft_time as time;

pub use file::{FORMAT_VERSION, TempSweep, create_private_dir_all, sweep_stale_temps, write_private};

use model::{Date, ModelError, Project, ResourceField, ResourceId, Schedule, TaskId};

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
        let mut s = Session { project, schedule, undo: Vec::new(), redo: Vec::new() };
        s.reschedule();
        s
    }

    /// Schedules the project, after keeping its outline to the rule. Each summary task stores its
    /// rolled-up duration (working minutes), so one that stops being a summary keeps its last rolled-up duration
    /// (worked example 13); the scheduler never reads a summary's own duration. A summary's work
    /// assignments work that duration.
    fn reschedule(&mut self) {
        self.project.normalize_outline();
        self.schedule = model::schedule(&self.project);
        if let Ok(s) = &self.schedule {
            self.project.set_rolled_up(&s.rolled_up);
        }
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
        let result = catch_unwind(AssertUnwindSafe(|| {
            (spec.run)(&mut self.project, params)?;
            self.reschedule();
            Ok(())
        }))
        .unwrap_or_else(|_| Err(CmdError::Params(format!("{id} failed unexpectedly; nothing was changed"))));
        match result {
            Ok(()) if self.project != before => {
                self.undo.push(before);
                if self.undo.len() > UNDO_DEPTH {
                    self.undo.remove(0);
                }
                self.redo.clear();
                Ok(())
            }
            Ok(()) => Ok(()),
            Err(e) => {
                self.project = before;
                self.schedule = model::schedule(&self.project);
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

fn duration() -> Value {
    json!({
        "type": ["integer", "string"],
        "minimum": 0,
        "maximum": model::MAX_DAYS,
        "description": "working days as a whole number (3), or text with a unit: 30m, 4h, 1.5d, 2w, 1mo (d, w and mo at the project's hours per day, hours per week and days per month; rounded to the minute; elapsed units are refused). Shown in the unit given.",
    })
}

/// The `duration` param in working minutes, and the unit to show it in: a whole number of days,
/// or text such as `4h` or `1.5d`; `default_days` when absent.
fn duration_param(p: &Project, v: &Value, default_days: Option<i64>) -> Result<(i64, model::DurationUnit), CmdError> {
    match v.get("duration") {
        Some(Value::String(text)) => Ok(model::parse_duration(text, &p.options)?),
        Some(n) => {
            let days = n.as_i64().ok_or_else(|| CmdError::Params("`duration` must be a whole number of days or text such as 4h".into()))?;
            Ok((p.days_to_minutes(days)?, model::DurationUnit::Days))
        }
        None => match default_days {
            Some(d) => Ok((p.days_to_minutes(d)?, model::DurationUnit::Days)),
            None => Err(CmdError::Params("`duration` is required".into())),
        },
    }
}

/// Checks params against a command schema. Supports the subset our schemas use: an object with
/// `properties`, `required` and `additionalProperties: false`; property `type` integer, number,
/// string, boolean, array or object, or a list of them (any one will do); `minimum`/`maximum`; `enum` of strings. `null` params count as `{}`.
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
    let types: Vec<&str> = match p.get("type") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        Some(t) => t.as_str().into_iter().collect(),
        None => Vec::new(),
    };
    let fits = |ty: &str| match ty {
        "integer" => v.as_i64().is_some(),
        "number" => v.is_number(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    };
    if !types.is_empty() && !types.iter().any(|t| fits(t)) {
        let what: Vec<&str> = types
            .iter()
            .map(|ty| match *ty {
                "integer" => "a whole number",
                "string" => "text",
                t => t,
            })
            .collect();
        return Err(CmdError::Params(format!("`{k}` must be {}", what.join(" or "))));
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

/// The `ids` param: a list of Unique IDs.
fn ids_param(v: &Value) -> Result<Vec<TaskId>, CmdError> {
    let list = v.get("ids").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`ids` (a list of Unique IDs) is required".into()))?;
    if list.len() > model::MAX_TASKS {
        return Err(CmdError::Params(format!("`ids` lists more than {} tasks", model::MAX_TASKS)));
    }
    list.iter()
        .map(|n| {
            n.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .map(TaskId)
                .ok_or_else(|| CmdError::Params("`ids` must be Unique IDs (whole numbers)".into()))
        })
        .collect()
}

fn ids() -> Value {
    json!({"type": "array", "items": task_id(), "description": "the tasks' Unique IDs (not their row IDs); each moves with its subtasks"})
}

/// A list of Unique IDs, described for one command.
fn ids_for(what: &str) -> Value {
    json!({"type": "array", "items": task_id(), "description": format!("the tasks' Unique IDs (not their row IDs); {what}")})
}

/// An optional Unique ID param naming the row to go above; absent means the end.
fn above(what: &str) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": u32::MAX, "description": format!("the Unique ID (not the row ID) of the task to {what} above; the end if absent")})
}

fn opt_id_param(v: &Value, k: &str) -> Result<Option<TaskId>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| Some(TaskId(n)))
            .ok_or_else(|| CmdError::Params(format!("`{k}` must be a Unique ID"))),
    }
}

fn id_param(v: &Value) -> Result<TaskId, CmdError> {
    let n = int_param(v, "id")?;
    u32::try_from(n).map(TaskId).map_err(|_| CmdError::Params(format!("no task {n}")))
}

fn date_param(v: &Value, k: &str) -> Result<Date, CmdError> {
    str_param(v, k)?.parse::<Date>().map_err(|e| CmdError::Params(e.to_string()))
}

fn date() -> Value {
    json!({"type": "string", "format": "date", "description": "YYYY-MM-DD"})
}

/// Weekday names as `calendar.setWorking` takes them, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAY_NAMES: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// Working weekdays (Monday first) from a list of names: `Mon` or `Monday`, any case.
fn weekdays_param(v: &Value) -> Result<[bool; 7], CmdError> {
    let list = v.get("weekdays").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`weekdays` (a list) is required".into()))?;
    let mut working = [false; 7];
    for item in list {
        let text = item.as_str().ok_or_else(|| CmdError::Params("`weekdays` holds weekday names, e.g. \"Mon\"".into()))?;
        let lower = text.trim().to_lowercase();
        let i = (0..7)
            .find(|&i| WEEKDAYS.get(i).is_some_and(|w| w.to_lowercase() == lower) || WEEKDAY_NAMES.get(i).is_some_and(|w| *w == lower))
            .ok_or_else(|| {
                let text: String = text.chars().take(20).collect();
                CmdError::Params(format!("`{text}` is not a weekday; expected one of {}", WEEKDAYS.join(", ")))
            })?;
        if let Some(w) = working.get_mut(i) {
            *w = true;
        }
    }
    Ok(working)
}

/// Working times as commands take them: `["8:00-12:00", "13:00-17:00"]`; none is a day off.
fn times_param(v: &Value) -> Result<Vec<model::WorkingTime>, CmdError> {
    let list = v.get("times").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`times` (a list) is required".into()))?;
    if list.len() > model::MAX_WORKING_TIMES {
        return Err(CmdError::Params(format!("a day holds at most {} working times", model::MAX_WORKING_TIMES)));
    }
    let times = list
        .iter()
        .map(|t| {
            let text = t.as_str().ok_or_else(|| CmdError::Params("`times` holds working times, e.g. \"8:00-12:00\"".into()))?;
            Ok(model::parse_working_time(text)?)
        })
        .collect::<Result<Vec<_>, CmdError>>()?;
    model::check_working_times(&times)?;
    Ok(times)
}

fn times(what: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "string", "description": "from-to in 24-hour time, e.g. 8:00-12:00"},
        "maxItems": model::MAX_WORKING_TIMES,
        "description": format!("{what}: in order and not overlapping, whole minutes, within 0:00-24:00"),
    })
}

/// Hours as a param, in whole minutes: `7.5` is 450.
fn hours_param(v: &Value, k: &str) -> Result<Option<u32>, CmdError> {
    let Some(h) = v.get(k).and_then(Value::as_f64) else { return Ok(None) };
    let m = h * 60.0;
    if !m.is_finite() || !(0.0..=f64::from(u32::MAX)).contains(&m) || (m - m.round()).abs() > 1e-6 {
        return Err(CmdError::Params(format!("`{k}` must be whole minutes of an hour, e.g. 7.5")));
    }
    Ok(Some(m.round() as u32))
}

/// Names are one line and of a sensible length.
fn name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(255).collect()
}

fn resource_id(what: &str) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": u32::MAX, "description": format!("the resource's Unique ID (not its row ID){what}")})
}

fn resource_ids(what: &str) -> Value {
    json!({"type": "array", "items": resource_id(""), "description": format!("the resources' Unique IDs (not their row IDs); {what}")})
}

fn money(what: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": model::MAX_MONEY, "description": format!("{what}, in whole minor units of the project currency (pence: 1250 is £12.50)")})
}

/// An optional resource Unique ID param; absent (or null) is `None`.
fn opt_resource_param(v: &Value, k: &str) -> Result<Option<ResourceId>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| Some(ResourceId(n)))
            .ok_or_else(|| CmdError::Params(format!("`{k}` must be a resource's Unique ID"))),
    }
}

fn resource_param(v: &Value) -> Result<ResourceId, CmdError> {
    opt_resource_param(v, "id")?.ok_or_else(|| CmdError::Params("`id` (a resource's Unique ID) is required".into()))
}

/// The `ids` param as resource Unique IDs.
fn resource_ids_param(v: &Value) -> Result<Vec<ResourceId>, CmdError> {
    let list = v.get("ids").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`ids` (a list of Unique IDs) is required".into()))?;
    if list.len() > model::MAX_RESOURCES {
        return Err(CmdError::Params(format!("`ids` lists more than {} resources", model::MAX_RESOURCES)));
    }
    list.iter()
        .map(|n| {
            n.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .map(ResourceId)
                .ok_or_else(|| CmdError::Params("`ids` must be Unique IDs (whole numbers)".into()))
        })
        .collect()
}

/// The fields `resource.set` was given, in the order it applies them: the type first, so the
/// others are checked against the new type.
fn resource_fields(v: &Value) -> Result<Vec<ResourceField>, CmdError> {
    let text = |k: &str| v.get(k).and_then(Value::as_str);
    let amount = |k: &str| v.get(k).and_then(Value::as_i64);
    let mut out = Vec::new();
    if let Some(t) = text("type") {
        out.push(ResourceField::Type(model::ResourceType::from_name(t)?));
    }
    if let Some(s) = text("initials") {
        out.push(ResourceField::Initials(s.into()));
    }
    if let Some(s) = text("group") {
        out.push(ResourceField::Group(s.into()));
    }
    if let Some(n) = v.get("maxUnits") {
        // The number exactly as written (`37.5`), never through a float.
        let units = n.as_number().map(ToString::to_string).ok_or_else(|| CmdError::Params("`maxUnits` must be a number".into()))?;
        out.push(ResourceField::MaxUnits(model::parse_units(&units)?));
    }
    if let Some(s) = text("materialLabel") {
        out.push(ResourceField::MaterialLabel(s.into()));
    }
    if let Some(n) = amount("standardRate") {
        out.push(ResourceField::StandardRate(n));
    }
    if let Some(n) = amount("overtimeRate") {
        out.push(ResourceField::OvertimeRate(n));
    }
    if let Some(n) = amount("costPerUse") {
        out.push(ResourceField::CostPerUse(n));
    }
    if let Some(a) = text("accrueAt") {
        out.push(ResourceField::AccrueAt(model::AccrueAt::from_name(a)?));
    }
    Ok(out)
}

/// The tasks and resource of an `assignment.*` command: `task` (one Unique ID) or `tasks` (a
/// list, as the Assign Resources dialog sends a selection), never both; each task must exist.
/// A task listed twice counts once.
fn assignment_params(p: &Project, v: &Value) -> Result<(Vec<TaskId>, ResourceId), CmdError> {
    let resource = opt_resource_param(v, "resource")?.ok_or_else(|| CmdError::Params("`resource` (a resource's Unique ID) is required".into()))?;
    let tasks = match (opt_id_param(v, "task")?, v.get("tasks")) {
        (Some(_), Some(_)) => return Err(CmdError::Params("give `task` or `tasks`, not both".into())),
        (Some(t), None) => vec![t],
        (None, Some(list)) => {
            let list = list.as_array().ok_or_else(|| CmdError::Params("`tasks` must be a list of Unique IDs".into()))?;
            if list.len() > model::MAX_TASKS {
                return Err(CmdError::Params(format!("`tasks` lists more than {} tasks", model::MAX_TASKS)));
            }
            let mut out: Vec<TaskId> = Vec::with_capacity(list.len());
            for n in list {
                let t = n
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .map(TaskId)
                    .ok_or_else(|| CmdError::Params("`tasks` must be Unique IDs (whole numbers)".into()))?;
                if !out.contains(&t) {
                    out.push(t);
                }
            }
            out
        }
        (None, None) => return Err(CmdError::Params("`task` (a task's Unique ID) or `tasks` (a list of them) is required".into())),
    };
    if tasks.is_empty() {
        return Err(CmdError::Params("`tasks` lists no task".into()));
    }
    for t in &tasks {
        p.task(*t)?;
    }
    Ok((tasks, resource))
}

/// Of `tasks`, those the resource is (`true`) or isn't (`false`) assigned to.
fn holding(p: &Project, tasks: &[TaskId], resource: ResourceId, assigned: bool) -> Vec<TaskId> {
    tasks.iter().copied().filter(|t| p.assignments_of(*t).any(|a| a.resource == resource) == assigned).collect()
}

/// The `task`/`tasks` pair of an `assignment.*` command's schema.
fn assignment_tasks(what: &str) -> Value {
    json!({
        "task": task_id(),
        "tasks": {"type": "array", "items": task_id(), "description": format!("instead of `task`: the selected tasks' Unique IDs, changed together as one undo step; {what}")},
    })
}

/// An `assignment.*` schema: `task` or `tasks`, `resource`, and these other properties.
fn assignment_schema(what: &str, more: Value, required: &[&str]) -> Value {
    let mut props = assignment_tasks(what);
    if let (Some(props), Some(more)) = (props.as_object_mut(), more.as_object()) {
        props.insert("resource".into(), resource_id(""));
        props.extend(more.clone());
    }
    object(props, required)
}

/// The `units` param read for the resource's type; `None` when absent.
fn units_param(p: &Project, resource: ResourceId, v: &Value) -> Result<Option<model::Units>, CmdError> {
    let Some(text) = v.get("units").and_then(Value::as_str) else { return Ok(None) };
    Ok(Some(model::parse_assignment_units(p.resource(resource)?.kind, text, &p.currency)?))
}

fn units(what: &str) -> Value {
    json!({"type": "string", "description": format!("{what}; as the Resource Names column writes them: a work resource's percent of its working time (50%), a material's quantity, fixed (5 tons) or per time (2 tons/d, with m, h, d, w or mo), or a cost resource's amount in the project currency (£200)")})
}

/// Most headings `view.setColumns` reads: every column and Task Name and ID, with room for repeats.
const MAX_COLUMN_NAMES: usize = 64;

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
        schema: || object(json!({"date": date()}), &["date"]),
        run: |p, v| {
            p.start = date_param(v, "date")?;
            Ok(())
        },
    },
    CommandSpec {
        id: "project.setOptions",
        label: "Project Options",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "hoursPerDay": {"type": "number", "minimum": 1, "maximum": 24, "description": "working hours in a day (d); default 8; whole minutes, e.g. 7.5"},
                    "hoursPerWeek": {"type": "number", "minimum": 1, "maximum": 168, "description": "working hours in a week (w); default 40; whole minutes"},
                    "daysPerMonth": {"type": "integer", "minimum": 1, "maximum": 31, "description": "working days in a month (mo); default 20"},
                    "currency": {"type": "string", "description": "the project's one currency, as its ISO 4217 code (GBP, the default; EUR, USD, JPY, BHD…). No exchange rates: amounts keep their value, rounded half to even when the new currency has fewer decimal places. A known code brings its symbol and decimal places"},
                    "currencySymbol": {"type": "string", "description": "the symbol amounts show with (£, €, $, CHF); at most 8 characters, no digits, signs, commas or full stops. Absent: the currency's own"},
                    "currencyDigits": {"type": "integer", "minimum": 0, "maximum": model::MAX_CURRENCY_DIGITS, "description": "decimal places of the currency's minor unit (2 for pence, 0 for yen, 3 for fils). Absent: the currency's own"},
                    "defaultTaskType": {"type": "string", "enum": model::TaskType::NAMES, "description": "the type new tasks get; default Fixed Units"},
                    "newTasksEffortDriven": {"type": "boolean", "description": "whether new tasks are effort-driven; default false (new Fixed Work tasks always are)"},
                }),
                &[],
            )
        },
        run: |p, v| {
            let mut o = p.options;
            if let Some(m) = hours_param(v, "hoursPerDay")? {
                o.minutes_per_day = m;
            }
            if let Some(m) = hours_param(v, "hoursPerWeek")? {
                o.minutes_per_week = m;
            }
            if let Some(d) = v.get("daysPerMonth").and_then(Value::as_i64) {
                o.days_per_month = u32::try_from(d).map_err(|_| CmdError::Params("`daysPerMonth` must be between 1 and 31".into()))?;
            }
            if let Some(t) = v.get("defaultTaskType").and_then(Value::as_str) {
                o.task_type = model::TaskType::from_name(t)?;
            }
            if let Some(e) = v.get("newTasksEffortDriven").and_then(Value::as_bool) {
                o.effort_driven = e;
            }
            o.check()?;
            p.options = o;
            let symbol = v.get("currencySymbol").and_then(Value::as_str);
            let digits = match v.get("currencyDigits").and_then(Value::as_i64) {
                Some(d) => Some(
                    u32::try_from(d)
                        .map_err(|_| CmdError::Params(format!("`currencyDigits` must be between 0 and {}", model::MAX_CURRENCY_DIGITS)))?,
                ),
                None => None,
            };
            if v.get("currency").is_some() || symbol.is_some() || digits.is_some() {
                // A symbol or places alone keep the code; a new code alone brings its own.
                let code = v.get("currency").and_then(Value::as_str);
                let same = code.is_none_or(|c| c.trim().eq_ignore_ascii_case(&p.currency.code));
                let (symbol, digits) =
                    if same { (symbol.or(Some(p.currency.symbol.as_str())), digits.or(Some(p.currency.digits))) } else { (symbol, digits) };
                let c = model::Currency::new(code.unwrap_or(&p.currency.code), symbol, digits)?;
                p.set_currency(c)?;
            }
            Ok(())
        },
    },
    CommandSpec {
        id: "task.add",
        label: "New Task",
        effect: Effect::Edit,
        schema: || object(json!({"name": {"type": "string"}, "duration": duration()}), &[]),
        run: |p, v| {
            let n = v.get("name").and_then(Value::as_str).map_or_else(|| "New task".to_owned(), name);
            let (duration, unit) = duration_param(p, v, Some(1))?;
            let id = p.add_task(n, duration)?;
            p.task_mut(id)?.duration_unit = unit;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.insert",
        label: "Insert Task",
        effect: Effect::Edit,
        schema: || object(json!({"before": above("insert"), "name": {"type": "string"}, "duration": duration()}), &[]),
        run: |p, v| {
            let n = v.get("name").and_then(Value::as_str).map_or_else(|| "New task".to_owned(), name);
            let (duration, unit) = duration_param(p, v, Some(1))?;
            let id = p.insert_task(opt_id_param(v, "before")?, n, duration)?;
            p.task_mut(id)?.duration_unit = unit;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.move",
        label: "Move Task",
        effect: Effect::Edit,
        schema: || object(json!({"ids": ids_for("each moves with its subtasks"), "to": above("move")}), &["ids"]),
        run: |p, v| {
            p.move_tasks(&ids_param(v)?, opt_id_param(v, "to")?)?;
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
        schema: || object(json!({"id": task_id(), "duration": duration()}), &["id", "duration"]),
        run: |p, v| {
            let id = id_param(v)?;
            if p.is_summary(id) {
                return Err(CmdError::Params("a summary task's duration is rolled up from its subtasks and can't be set".into()));
            }
            let (duration, unit) = duration_param(p, v, None)?;
            p.set_duration(id, duration, unit)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setWork",
        label: "Work",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "id": task_id(),
                    "work": {"type": ["number", "string"], "minimum": 0, "description": "the task's work: hours as a number (20, 7.5), or text with a unit (30m, 20h, 2d, 1w). Shared among its work resources in proportion to their work; Fixed Units and Fixed Work tasks change duration, Fixed Duration tasks change units. Needs a work resource."},
                }),
                &["id", "work"],
            )
        },
        run: |p, v| {
            let text = match v.get("work") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => return Err(CmdError::Params("`work` (hours, or text such as 20h) is required".into())),
            };
            let work = model::parse_work(&text, &p.options)?;
            p.set_task_work(id_param(v)?, work)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setType",
        label: "Task Type",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "id": task_id(),
                    "type": {"type": "string", "enum": model::TaskType::NAMES, "description": "which of duration, units and work stays when another changes: Fixed Units (the default), Fixed Duration or Fixed Work. Nothing is recalculated when it changes."},
                    "effortDriven": {"type": "boolean", "description": "keep the task's work as work resources are added or removed; always on for Fixed Work. Not for a summary."},
                }),
                &["id"],
            )
        },
        run: |p, v| {
            let ty = v.get("type").and_then(Value::as_str).map(model::TaskType::from_name).transpose()?;
            let effort = v.get("effortDriven").and_then(Value::as_bool);
            if ty.is_none() && effort.is_none() {
                return Err(CmdError::Params("give `type`, `effortDriven` or both".into()));
            }
            p.set_task_type(id_param(v)?, ty, effort)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setPredecessors",
        label: "Predecessors",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({"id": task_id(), "text": {"type": "string", "description": "predecessors by Unique ID, e.g. 3, 5SS+2, 7FS+4h; a lag is a number with an optional sign and unit (m, h, d, w, mo; days when none; decimals allowed: 7FF-1.5d). Elapsed and percent lags are refused."}}),
                &["id", "text"],
            )
        },
        run: |p, v| {
            let links = model::parse_preds(str_param(v, "text")?, &p.options)?;
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
            let id = id_param(v)?;
            p.task_mut(id)?.constraint = c;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setDeadline",
        label: "Deadline",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "id": task_id(),
                    "date": {"type": "string", "format": "date", "description": "YYYY-MM-DD; leave out to clear. A deadline never moves the task; finishing after it gives negative slack."},
                }),
                &["id"],
            )
        },
        run: |p, v| {
            let date = match v.get("date").and_then(Value::as_str) {
                Some(d) => Some(d.parse::<Date>().map_err(|e| CmdError::Params(e.to_string()))?),
                None => None,
            };
            p.task_mut(id_param(v)?)?.deadline = date;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setFixedCost",
        label: "Fixed Cost",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({"id": task_id(), "amount": money("the task's own cost, not a resource's; 0 clears it. Summary tasks take one too")}),
                &["id", "amount"],
            )
        },
        run: |p, v| {
            p.set_fixed_cost(id_param(v)?, int_param(v, "amount")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.indent",
        label: "Indent Task",
        effect: Effect::Edit,
        schema: || object(json!({"ids": ids()}), &["ids"]),
        run: |p, v| {
            p.indent(&ids_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.outdent",
        label: "Outdent Task",
        effect: Effect::Edit,
        schema: || object(json!({"ids": ids()}), &["ids"]),
        run: |p, v| {
            p.outdent(&ids_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "task.delete",
        label: "Delete Task",
        effect: Effect::Destructive,
        schema: || object(json!({"ids": ids_for("each is deleted with its subtasks")}), &["ids"]),
        run: |p, v| {
            p.delete_tasks(&ids_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "calendar.setWorking",
        label: "Working Days",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({"weekdays": {
                    "type": "array",
                    "items": {"type": "string", "enum": WEEKDAYS},
                    "description": "the weekdays worked, e.g. [\"Mon\", \"Tue\", \"Wed\", \"Thu\", \"Fri\"]; at least one (a week with none is refused)",
                }}),
                &["weekdays"],
            )
        },
        run: |p, v| {
            p.calendar.set_working(weekdays_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "calendar.setWorkingTimes",
        label: "Working Times",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "weekday": {"type": "string", "enum": WEEKDAYS},
                    "times": times("the weekday's working times, e.g. [\"8:00-12:00\", \"13:00-17:00\"]; none makes it non-working (a week with no working day is refused)"),
                }),
                &["weekday", "times"],
            )
        },
        run: |p, v| {
            let day = str_param(v, "weekday")?;
            let i = WEEKDAYS
                .iter()
                .position(|w| *w == day)
                .ok_or_else(|| CmdError::Params(format!("`weekday` must be one of {}", WEEKDAYS.join(", "))))?;
            p.calendar.set_weekday_times(i, times_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "calendar.addException",
        label: "Add Exception",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "date": date(),
                    "working": {"type": "boolean", "description": "true makes the day worked (an overtime Saturday); false makes it non-working (a holiday); absent, the day is worked if it has times"},
                    "times": times("a working day's times, e.g. [\"9:00-13:00\"]; absent, the default 8:00-12:00 and 13:00-17:00"),
                }),
                &["date"],
            )
        },
        run: |p, v| {
            let date = date_param(v, "date")?;
            let times = if v.get("times").is_some() { Some(times_param(v)?) } else { None };
            let working = v.get("working").and_then(Value::as_bool).unwrap_or_else(|| times.as_ref().is_some_and(|t| !t.is_empty()));
            match (working, times) {
                (false, Some(t)) if !t.is_empty() => Err(CmdError::Params("a non-working exception has no working times".into())),
                (true, Some(t)) if t.is_empty() => Err(CmdError::Params("a working exception needs working times".into())),
                (true, Some(t)) => Ok(p.calendar.set_exception(date, t)?),
                (w, _) => Ok(p.calendar.add_exception(date, w)?),
            }
        },
    },
    CommandSpec {
        id: "calendar.removeException",
        label: "Remove Exception",
        effect: Effect::Edit,
        schema: || object(json!({"date": date()}), &["date"]),
        run: |p, v| {
            p.calendar.remove_exception(date_param(v, "date")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "resource.add",
        label: "New Resource",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "name": {"type": "string", "description": "unique (any case); no [ ] or commas. Absent: `Resource n`"},
                    "type": {"type": "string", "enum": model::ResourceType::NAMES, "description": "Work (the default): people and equipment; Material: consumables; Cost: costs that don't depend on work"},
                    "before": resource_id(" to insert above; the end if absent"),
                }),
                &[],
            )
        },
        run: |p, v| {
            let kind = match v.get("type").and_then(Value::as_str) {
                Some(t) => model::ResourceType::from_name(t)?,
                None => model::ResourceType::Work,
            };
            p.add_resource(v.get("name").and_then(Value::as_str), kind, opt_resource_param(v, "before")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "resource.rename",
        label: "Resource Name",
        effect: Effect::Edit,
        schema: || {
            object(json!({"id": resource_id(""), "name": {"type": "string", "description": "unique (any case); no [ ] or commas"}}), &["id", "name"])
        },
        run: |p, v| {
            p.rename_resource(resource_param(v)?, str_param(v, "name")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "resource.set",
        label: "Resource Information",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({
                    "id": resource_id(""),
                    "type": {"type": "string", "enum": model::ResourceType::NAMES, "description": "applied first; the other fields given must suit the new type"},
                    "initials": {"type": "string"},
                    "group": {"type": "string"},
                    "maxUnits": {"type": "number", "minimum": 0, "maximum": model::MAX_UNITS / model::UNITS_PER_PERCENT, "description": "work resources: percent of full time, up to two decimal places (100 is full time, 50 half time)"},
                    "materialLabel": {"type": "string", "description": "material resources: the unit it is counted in, e.g. tons"},
                    "standardRate": money("work resources: per hour; materials: per unit of the material label"),
                    "overtimeRate": money("work resources: per hour of overtime"),
                    "costPerUse": money("work and material resources: charged once per assignment"),
                    "accrueAt": {"type": "string", "enum": model::AccrueAt::NAMES, "description": "when the cost is incurred; Prorated (the default) spreads it over the work"},
                }),
                &["id"],
            )
        },
        run: |p, v| {
            p.set_resource(resource_param(v)?, &resource_fields(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "resource.move",
        label: "Move Resource",
        effect: Effect::Edit,
        schema: || object(json!({"ids": resource_ids("kept in their order"), "to": resource_id(" to move above; the end if absent")}), &["ids"]),
        run: |p, v| {
            p.move_resources(&resource_ids_param(v)?, opt_resource_param(v, "to")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "resource.delete",
        label: "Delete Resource",
        effect: Effect::Destructive,
        schema: || object(json!({"ids": resource_ids("each is deleted with its assignments")}), &["ids"]),
        run: |p, v| {
            p.delete_resources(&resource_ids_param(v)?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "assignment.add",
        label: "Assign Resource",
        effect: Effect::Edit,
        schema: || {
            assignment_schema(
                "those it is already assigned to keep their units",
                json!({"units": units("absent: 100% (or the resource's Max Units if lower), one of a material's label, no cost")}),
                &["resource"],
            )
        },
        run: |p, v| {
            let (tasks, resource) = assignment_params(p, v)?;
            let units = units_param(p, resource, v)?;
            // Refused (as already assigned) only when every task has it.
            let todo = holding(p, &tasks, resource, false);
            for task in if todo.is_empty() { tasks } else { todo } {
                p.assign(task, resource, units)?;
            }
            Ok(())
        },
    },
    CommandSpec {
        id: "assignment.setUnits",
        label: "Assignment Units",
        effect: Effect::Edit,
        schema: || assignment_schema("those it isn't assigned to are left alone", json!({"units": units("the new units")}), &["resource", "units"]),
        run: |p, v| {
            let (tasks, resource) = assignment_params(p, v)?;
            let units = units_param(p, resource, v)?.ok_or_else(|| CmdError::Params("`units` (text) is required".into()))?;
            // Refused (as not assigned) only when no task has it.
            let todo = holding(p, &tasks, resource, true);
            for task in if todo.is_empty() { tasks } else { todo } {
                p.set_assignment_units(task, resource, units)?;
            }
            Ok(())
        },
    },
    CommandSpec {
        id: "assignment.remove",
        label: "Remove Assignment",
        effect: Effect::Edit,
        schema: || assignment_schema("those it isn't assigned to are left alone", json!({}), &["resource"]),
        run: |p, v| {
            let (tasks, resource) = assignment_params(p, v)?;
            let todo = holding(p, &tasks, resource, true);
            for task in if todo.is_empty() { tasks } else { todo } {
                p.unassign(task, resource)?;
            }
            Ok(())
        },
    },
    CommandSpec {
        id: "task.setResourceNames",
        label: "Resource Names",
        effect: Effect::Edit,
        schema: || {
            object(
                json!({"id": task_id(), "text": {"type": "string", "description": "the task's resources by name (not by Unique ID), as Microsoft Project's Resource Names column: Ann[50%], Bob, Concrete[2 tons/d], Travel[£200]; units as `assignment.add` takes them, absent the default. Replaces the task's assignments; empty removes them. A name that isn't a resource yet becomes a new work resource with 100% Max Units."}}),
                &["id", "text"],
            )
        },
        run: |p, v| {
            p.set_resource_names(id_param(v)?, str_param(v, "text")?)?;
            Ok(())
        },
    },
    CommandSpec {
        id: "view.setColumns",
        label: "Columns",
        effect: Effect::Edit,
        schema: || {
            let names: Vec<&str> = model::Column::ALL.iter().map(|c| c.name()).collect();
            object(
                json!({"columns": {
                    "type": "array",
                    "items": {"type": "string", "enum": names},
                    "maxItems": MAX_COLUMN_NAMES,
                    "description": "the task sheet's columns to show after ID and Task Name, left to right, by heading (any case); the rest are hidden. ID and Task Name always show (listing them changes nothing); a repeat counts once. Saved with the project",
                }}),
                &["columns"],
            )
        },
        run: |p, v| {
            let list =
                v.get("columns").and_then(Value::as_array).ok_or_else(|| CmdError::Params("`columns` (a list of headings) is required".into()))?;
            if list.len() > MAX_COLUMN_NAMES {
                return Err(CmdError::Params(format!("`columns` lists more than {MAX_COLUMN_NAMES} headings")));
            }
            let names = list
                .iter()
                .map(|n| n.as_str().ok_or_else(|| CmdError::Params("`columns` holds headings, e.g. \"Duration\"".into())))
                .collect::<Result<Vec<_>, _>>()?;
            p.columns = model::Columns::from_names(names)?;
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
    // Invented people and things.
    let resources = [
        json!({"name": "Priya Shah", "group": "Facilities", "standardRate": 4500, "overtimeRate": 6000}),
        json!({"name": "Tom Okafor", "group": "IT", "maxUnits": 50, "standardRate": 5250}),
        json!({"name": "Removals crew", "group": "Contractors", "maxUnits": 300, "standardRate": 3800, "costPerUse": 15000, "accrueAt": "End"}),
        json!({"name": "Packing boxes", "type": "Material", "materialLabel": "boxes", "standardRate": 250, "accrueAt": "Start"}),
        json!({"name": "Travel", "type": "Cost"}),
    ];
    for mut r in resources {
        let _ = s.run("resource.add", &json!({"name": r["name"]}));
        let id = s.project.resources.last().map_or(0, |r| r.id.0);
        if let Some(m) = r.as_object_mut() {
            m.remove("name");
            m.insert("id".into(), id.into());
        }
        let _ = s.run("resource.set", &r);
    }
    // Who does what: task Unique IDs 1–8 as added above.
    let assigned = [
        (1, "Priya Shah"),
        (2, "Priya Shah[50%], Tom Okafor"),
        (3, "Priya Shah"),
        (5, "Tom Okafor"),
        (6, "Removals crew[200%], Packing boxes[2 boxes/h]"),
        (7, "Removals crew[300%], Priya Shah, Travel[£150.00]"),
    ];
    for (id, text) in assigned {
        let _ = s.run("task.setResourceNames", &json!({"id": id, "text": text}));
    }
    // Costs of the tasks themselves: the furniture, and the cabling contractor's quote.
    for (id, amount) in [(3, 480_000), (5, 125_000)] {
        let _ = s.run("task.setFixedCost", &json!({"id": id, "amount": amount}));
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
    fn columns_are_one_undo_step_and_saved_with_the_project() {
        use model::Column;
        let mut s = sample();
        let before = s.project.clone();
        assert!(s.project.columns.is_default());
        // Older files have no columns: every column shows, and a default project writes none.
        let text = s.save_json().unwrap();
        assert!(!text.contains("\"columns\""), "{text}");
        s.run("view.setColumns", &json!({"columns": ["ID", "Task Name", "start", "Finish", "Resource Names"]})).unwrap();
        assert_eq!(s.project.columns.shown(), &[Column::Start, Column::Finish, Column::ResourceNames]);
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project.columns, s.project.columns);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.project, before, "one undo step");
        let err = s.run("view.setColumns", &json!({"columns": ["Start", "Colour"]})).unwrap_err().to_string();
        assert!(err.contains("not a column"), "{err}");
        assert_eq!(s.project, before);
        // A column a newer version added is dropped, not refused.
        let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
        v["columns"] = json!(["Finish", "Sparkle", 7, "Finish", "Cost"]);
        let newer = Session::load_json(&v.to_string()).unwrap();
        assert_eq!(newer.project.columns.shown(), &[Column::Finish, Column::Cost]);
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
        assert!(s.run("task.setDuration", &json!({"id": 1, "duration": "5x"})).is_err());
        assert!(s.run("task.setDuration", &json!({"id": 1, "duration": 1.5})).unwrap_err().to_string().contains("whole number or text"));
        assert!(s.run("task.setDuration", &json!({"id": 1, "duration": "3ed"})).unwrap_err().to_string().contains("elapsed durations"));
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
                if ["id", "ids", "text", "before", "to", "task", "resource"].contains(&k.as_str()) {
                    assert!(p["description"].as_str().unwrap().contains("Unique ID"), "{}.{k}", c.id);
                }
            }
        }
        let mut s = sample();
        s.run("task.delete", &json!({"ids": [1]})).unwrap();
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

    #[test]
    fn calendar_commands_set_weekdays_and_exceptions_and_refuse_a_week_with_none() {
        let mut s = sample();
        let before = s.project.clone();
        s.run("calendar.setWorking", &json!({"weekdays": ["Mon", "tuesday", "WED", "Thu", "Fri", "Sat"]})).unwrap();
        assert_eq!(s.project.calendar.working, [true, true, true, true, true, true, false]);
        for bad in [json!([]), json!(["Funday"]), json!([1]), json!("Mon")] {
            assert!(s.run("calendar.setWorking", &json!({"weekdays": bad})).is_err(), "{bad}");
        }
        let e = s.run("calendar.setWorking", &json!({"weekdays": []})).unwrap_err().to_string();
        assert!(e.contains("at least one working weekday"), "{e}");
        s.run("calendar.addException", &json!({"date": "2026-11-04"})).unwrap();
        s.run("calendar.addException", &json!({"date": "2026-11-08", "working": true})).unwrap();
        assert_eq!(s.project.calendar.exceptions.len(), 2);
        assert!(s.run("calendar.addException", &json!({"date": "2026-02-30"})).is_err());
        s.run("calendar.removeException", &json!({"date": "2026-11-04"})).unwrap();
        assert!(s.run("calendar.removeException", &json!({"date": "2026-11-04"})).unwrap_err().to_string().contains("no exception"));
        for _ in 0..4 {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before);
    }

    #[test]
    fn resource_commands_add_set_move_and_delete_by_unique_id() {
        let mut s = sample();
        assert_eq!(s.project.resources.len(), 5, "the sample has invented resources");
        assert!(s.project.resources.iter().all(|r| r.check().is_ok()));
        let tom = s.project.resource(ResourceId(2)).unwrap();
        assert_eq!((tom.max_units, tom.standard_rate, tom.initials.as_str()), (5_000, 5_250, "T"));
        let boxes = s.project.resource(ResourceId(4)).unwrap();
        assert_eq!((boxes.kind, boxes.material_label.as_str(), boxes.accrue_at), (model::ResourceType::Material, "boxes", model::AccrueAt::Start));
        let before = s.project.clone();

        s.run("resource.add", &json!({"name": "Sam Reid", "before": 1})).unwrap();
        assert_eq!(s.project.resources[0].id, ResourceId(6));
        s.run("resource.set", &json!({"id": 6, "maxUnits": 37.5, "standardRate": 1250, "group": " Site "})).unwrap();
        let sam = s.project.resource(ResourceId(6)).unwrap();
        assert_eq!((sam.max_units, sam.standard_rate, sam.group.as_str()), (3_750, 1_250, "Site"));
        assert!(s.run("resource.set", &json!({"id": 6, "materialLabel": "tons"})).is_err(), "a work resource has no material label");
        s.run("resource.set", &json!({"id": 6, "type": "Material", "materialLabel": "tons"})).unwrap();
        assert!(s.run("resource.set", &json!({"id": 6, "maxUnits": 100.001})).is_err());
        assert!(s.run("resource.set", &json!({"id": 99, "group": "x"})).is_err());
        assert!(s.run("resource.rename", &json!({"id": 6, "name": "priya shah"})).unwrap_err().to_string().contains("already"));
        s.run("resource.rename", &json!({"id": 6, "name": "Sand"})).unwrap();
        s.run("resource.move", &json!({"ids": [6], "to": 5})).unwrap();
        assert_eq!(s.project.resource_row_of(ResourceId(6)), Some(5));
        s.run("resource.delete", &json!({"ids": [6, 1]})).unwrap();
        s.run("resource.add", &json!({})).unwrap();
        assert_eq!(s.project.resources.last().unwrap().id, ResourceId(7), "a deleted Unique ID is never reused");
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
        assert_eq!(commands().iter().find(|c| c.id == "resource.delete").unwrap().effect, Effect::Destructive);
        while s.can_undo() {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before, "each resource command is one undo step");
    }

    #[test]
    fn working_times_and_project_options_are_commands() {
        let mut s = sample();
        let before = s.project.clone();
        let dates = s.schedule.as_ref().unwrap().finish;
        s.run("calendar.setWorkingTimes", &json!({"weekday": "Fri", "times": ["8:00-12:00"]})).unwrap();
        assert_eq!(s.project.calendar.weekday_times(4), &[model::parse_working_time("8:00-12:00").unwrap()]);
        // The scheduler counts working time: a 4-hour Friday takes the plan past its old finish.
        assert!(s.schedule.as_ref().unwrap().finish > dates);
        s.run("calendar.setWorkingTimes", &json!({"weekday": "Sat", "times": ["9:00-13:00"]})).unwrap();
        assert!(s.project.calendar.working[5]);
        s.run("calendar.setWorkingTimes", &json!({"weekday": "Sat", "times": []})).unwrap();
        assert!(!s.project.calendar.working[5]);
        for bad in
            [json!(["13:00-17:00", "8:00-12:00"]), json!(["8:00-13:00", "12:00-17:00"]), json!(["17:00-8:00"]), json!(["25:00-26:00"]), json!([480])]
        {
            let after = s.project.clone();
            assert!(s.run("calendar.setWorkingTimes", &json!({"weekday": "Mon", "times": bad})).is_err(), "{bad}");
            assert_eq!(s.project, after);
        }
        // A worked Saturday with its own hours; a holiday has none.
        s.run("calendar.addException", &json!({"date": "2026-11-07", "times": ["9:00-13:00"]})).unwrap();
        let sat = Date::from_ymd(2026, 11, 7).unwrap();
        assert_eq!(s.project.calendar.exceptions.get(&sat), Some(&true));
        assert_eq!(s.project.calendar.times_on(sat), &[model::parse_working_time("9:00-13:00").unwrap()]);
        let e = s.run("calendar.addException", &json!({"date": "2026-11-04", "working": false, "times": ["9:00-13:00"]})).unwrap_err().to_string();
        assert!(e.contains("non-working exception has no working times"), "{e}");
        assert!(s.run("calendar.addException", &json!({"date": "2026-11-04", "working": true, "times": []})).is_err());
        // Options: defaults 8/40/20, whole minutes, bounded.
        assert_eq!(s.project.options, model::Options::default());
        s.run("project.setOptions", &json!({"hoursPerDay": 7.5, "daysPerMonth": 21})).unwrap();
        assert_eq!((s.project.options.minutes_per_day, s.project.options.minutes_per_week, s.project.options.days_per_month), (450, 2400, 21));
        for bad in [json!({"hoursPerDay": 7.501}), json!({"hoursPerDay": 0}), json!({"hoursPerWeek": 200}), json!({"daysPerMonth": 1.5})] {
            assert!(s.run("project.setOptions", &bad).is_err(), "{bad}");
        }
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
        while s.can_undo() {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before);
    }

    /// `tasks` in place of `task`: the Assign Resources dialog's selection, changed by one command
    /// (one undo step). Tasks that already have the resource keep their units; Remove and new
    /// units leave the tasks without it alone.
    #[test]
    fn assignment_commands_take_a_list_of_tasks_as_one_step() {
        let mut s = sample();
        let before = s.project.clone();
        s.run("assignment.add", &json!({"tasks": [2, 4, 5], "resource": 1})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(2)), "Priya Shah[50%], Tom Okafor[50%]", "already assigned: kept as it was");
        assert_eq!(s.project.resource_names(TaskId(4)), "Priya Shah");
        assert_eq!(s.project.resource_names(TaskId(5)), "Tom Okafor[50%], Priya Shah");
        s.run("assignment.setUnits", &json!({"tasks": [2, 4, 8], "resource": 1, "units": "25%"})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(2)), "Priya Shah[25%], Tom Okafor[50%]");
        assert_eq!(s.project.resource_names(TaskId(4)), "Priya Shah[25%]");
        assert_eq!(s.project.resource_names(TaskId(8)), "");
        s.run("assignment.remove", &json!({"tasks": [2, 4, 8], "resource": 1})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(2)), "Tom Okafor[50%]");
        assert_eq!(s.project.resource_names(TaskId(4)), "");
        let now = s.project.clone();
        for bad in [
            ("assignment.add", json!({"task": 4, "tasks": [4], "resource": 1})),
            ("assignment.add", json!({"tasks": [], "resource": 1})),
            ("assignment.add", json!({"tasks": [4, 99], "resource": 1})),
            ("assignment.add", json!({"tasks": [4, 2], "resource": 4, "units": "lots"})),
            ("assignment.add", json!({"tasks": [5, 5], "resource": 2})),
            ("assignment.remove", json!({"tasks": [4, 8], "resource": 1})),
            ("assignment.setUnits", json!({"tasks": [4, 8], "resource": 1, "units": "50%"})),
        ] {
            assert!(s.run(bad.0, &bad.1).is_err(), "{bad:?}");
            assert_eq!(s.project, now, "{bad:?}");
        }
        for _ in 0..3 {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before, "three commands, three undo steps");
    }

    #[test]
    fn assignment_commands_assign_change_and_remove_by_unique_id() {
        let mut s = sample();
        assert_eq!(s.project.resource_names(TaskId(7)), "Removals crew[300%], Priya Shah, Travel[£150.00]");
        assert_eq!(s.project.resource_names(TaskId(5)), "Tom Okafor[50%]", "Tom's Max Units is 50%, so that is his default");
        let before = s.project.clone();
        s.run("assignment.add", &json!({"task": 4, "resource": 4, "units": "5 boxes"})).unwrap();
        s.run("assignment.add", &json!({"task": 4, "resource": 1})).unwrap();
        s.run("assignment.setUnits", &json!({"task": 4, "resource": 1, "units": "25%"})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(4)), "Packing boxes[5 boxes], Priya Shah[25%]");
        for bad in [
            json!({"task": 4, "resource": 1}),
            json!({"task": 4, "resource": 2, "units": "5 boxes"}),
            json!({"task": 4, "resource": 99}),
            json!({"task": 99, "resource": 1}),
        ] {
            assert!(s.run("assignment.add", &bad).is_err(), "{bad}");
        }
        assert!(s.run("assignment.setUnits", &json!({"task": 4, "resource": 2, "units": "50%"})).is_err(), "not assigned");
        assert!(s.run("assignment.setUnits", &json!({"task": 4, "resource": 1, "units": "lots"})).is_err());
        s.run("assignment.remove", &json!({"task": 4, "resource": 4})).unwrap();
        assert!(s.run("assignment.remove", &json!({"task": 4, "resource": 4})).is_err());
        assert_eq!(s.project.resource_names(TaskId(4)), "Priya Shah[25%]");
        // Fixed Units, the default: Priya's 80 hours of work stay, so at 25% the task takes four
        // times as long; the material takes no part.
        assert_eq!(before.task(TaskId(4)).unwrap().duration, 10 * 480);
        assert_eq!(s.project.task(TaskId(4)).unwrap().duration, 40 * 480);
        assert_eq!(s.project.task_work(TaskId(4)), 80 * 60);
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
        for _ in 0..4 {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before, "each command is one undo step");
    }

    /// The sample's costs to the penny, worked by hand: Priya £45.00/hr, Tom £52.50/hr, the
    /// removals crew £38.00/hr and £150.00 a use, packing boxes £2.50 each, travel £150.00.
    #[test]
    fn sample_costs_to_the_penny_and_cost_commands() {
        let mut s = sample();
        let cost = |s: &Session, id: u32| s.project.cost_by_task()[&TaskId(id)];
        // 16h × £45 = £720; 12h × £45 + 12h × £52.50 = £1,170; 8h × £45 + £4,800 fixed.
        assert_eq!((cost(&s, 1), cost(&s, 2), cost(&s, 3)), (72_000, 117_000, 516_000));
        // 20h × £52.50 + £1,250 fixed; 32h × £38 + £150 + 32 boxes × £2.50; 24h × £38 + £150 + 8h × £45 + £150.
        assert_eq!((cost(&s, 5), cost(&s, 6), cost(&s, 7)), (230_000, 144_600, 157_200));
        assert_eq!(s.project.total_cost(), 1_236_800);
        let before = s.project.clone();
        s.run("task.setFixedCost", &json!({"id": 4, "amount": 99})).unwrap();
        assert_eq!(cost(&s, 4), 99);
        for bad in [
            json!({"id": 4, "amount": -1}),
            json!({"id": 4, "amount": model::MAX_MONEY + 1}),
            json!({"id": 99, "amount": 1}),
            json!({"id": 4, "amount": 1.5}),
        ] {
            assert!(s.run("task.setFixedCost", &bad).is_err(), "{bad}");
        }
        // Currency: a known code brings its symbol and places; amounts keep their value.
        s.run("project.setOptions", &json!({"currency": "eur"})).unwrap();
        assert_eq!(s.project.currency.format(s.project.total_cost()), "€12,368.99");
        assert_eq!(s.project.resource_names(TaskId(7)), "Removals crew[300%], Priya Shah, Travel[€150.00]");
        s.run("project.setOptions", &json!({"currencySymbol": "E"})).unwrap();
        assert_eq!((s.project.currency.code.as_str(), s.project.currency.symbol.as_str()), ("EUR", "E"));
        s.run("project.setOptions", &json!({"currency": "JPY"})).unwrap();
        // Rates round half to even to whole yen (£52.50 → ¥52, £2.50 → ¥2, £0.99 → ¥1), so the
        // total is recosted from them: 720 + 1,164 + 5,160 + 1 + 2,290 + 1,430 + 1,572.
        assert_eq!(s.project.currency.format(s.project.total_cost()), "¥12,337");
        s.run("assignment.setUnits", &json!({"task": 7, "resource": 5, "units": "¥2,000"})).unwrap();
        assert!(s.run("assignment.setUnits", &json!({"task": 7, "resource": 5, "units": "¥1.50"})).is_err());
        s.run("project.setOptions", &json!({"currency": "BHD", "currencyDigits": 3})).unwrap();
        assert_eq!(s.project.currency.format(s.project.resource(ResourceId(1)).unwrap().standard_rate), "BD45.000");
        for bad in [
            json!({"currency": "POUNDS"}),
            json!({"currencyDigits": 5}),
            json!({"currencySymbol": "1"}),
            json!({"currency": "GBP", "hoursPerDay": 0}),
        ] {
            let after = s.project.clone();
            assert!(s.run("project.setOptions", &bad).is_err(), "{bad}");
            assert_eq!(s.project, after);
        }
        let round = Session::load_json(&s.save_json().unwrap()).unwrap();
        assert_eq!(round.project, s.project);
        while s.can_undo() {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(s.project, before, "each command is one undo step");
        // A file with a damaged currency or fixed cost is refused.
        let mut v: Value = serde_json::from_str(&round.save_json().unwrap()).unwrap();
        v["currency"]["digits"] = json!(9);
        assert!(Session::load_json(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&round.save_json().unwrap()).unwrap();
        v["tasks"][0]["fixed_cost"] = json!(-5);
        assert!(Session::load_json(&v.to_string()).is_err());
    }

    /// Typing an unknown name in Resource Names makes a work resource and assigns it: one command,
    /// so one undo step takes both away.
    #[test]
    fn auto_create_is_one_undo_step() {
        let mut s = sample();
        let before = s.project.clone();
        s.run("task.setResourceNames", &json!({"id": 4, "text": "Sam Reid[50%], Priya Shah"})).unwrap();
        let sam = s.project.resources.last().unwrap();
        assert_eq!((sam.name.as_str(), sam.kind, sam.max_units), ("Sam Reid", model::ResourceType::Work, model::FULL_TIME));
        assert_eq!(s.project.resource_names(TaskId(4)), "Sam Reid[50%], Priya Shah");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.project, before);
        // A refused text makes nothing, not even the new resource.
        assert!(s.run("task.setResourceNames", &json!({"id": 4, "text": "Sam Reid, Priya Shah[lots]"})).is_err());
        assert_eq!(s.project, before);
    }

    #[test]
    fn deleting_or_retyping_a_resource_drops_its_assignments_and_loads_are_checked() {
        let mut s = sample();
        s.run("resource.delete", &json!({"ids": [1]})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(2)), "Tom Okafor[50%]");
        s.run("resource.set", &json!({"id": 2, "type": "Material"})).unwrap();
        assert_eq!(s.project.resource_names(TaskId(2)), "");
        s.run("task.delete", &json!({"ids": [7]})).unwrap();
        assert!(s.project.assignments.iter().all(|a| a.task != TaskId(7)));
        // A file whose assignments name a missing resource, or carry units of another type, is refused.
        let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
        assert!(v["assignments"].is_array());
        v["assignments"][0]["resource"] = json!(99);
        assert!(Session::load_json(&v.to_string()).err().unwrap().to_string().contains("missing resource 99"));
        let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
        v["assignments"][0]["units"] = json!({"cost": 5});
        assert!(Session::load_json(&v.to_string()).is_err());
        // A file without assignments (from before them) loads with none.
        let mut v: Value = serde_json::from_str(&s.save_json().unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("assignments");
        assert!(Session::load_json(&v.to_string()).unwrap().project.assignments.is_empty());
    }

    /// Valid params from a schema, then junk built from it: each property wrong-typed, out of
    /// range or huge, each required one missing, an unknown one added, and non-object params.
    fn junk_from(schema: &Value) -> Vec<Value> {
        let props = schema["properties"].as_object().cloned().unwrap_or_default();
        let good = |p: &Value| match p["type"].as_str() {
            _ if p["enum"].is_array() => p["enum"][1].clone(),
            Some("integer") | Some("number") => p["minimum"].clone(),
            Some("string") if p["format"] == "date" => json!("2026-11-02"),
            Some("array") if p["items"]["enum"].is_array() => json!([p["items"]["enum"][0].clone()]),
            Some("array") => json!([2]),
            Some("boolean") => json!(true),
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
            json!([0, -1, 1.5, "1", null]),
            json!([u32::MAX, 1, 1, 2]),
            Value::Array(vec![json!("Mon"); 10_000]),
            json!([[], {}, null]),
            json!("9999-12-31"),
            json!("0001-01-01"),
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
