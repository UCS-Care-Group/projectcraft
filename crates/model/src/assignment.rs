//! Assignments: one resource on one task, with **units**, as Microsoft Project's.
//!
//! - A work resource's units are the share of its working time it gives the task, in hundredths
//!   of a percent (`50%` is 5,000). A new assignment takes 100%, or the resource's Max Units if
//!   lower. Its work is the task's duration × units.
//! - A material resource's units are a quantity of its material label, in hundredths: **fixed**
//!   (`5 tons`, whatever the duration) or **variable** (`2 tons/d`, a rate per time that scales
//!   with the duration), as Microsoft describes in "Learn more about resource units"
//!   (<https://support.microsoft.com/en-us/project/learn-more-about-resource-units>).
//! - A cost resource takes an amount of money per assignment, in minor units.
//!
//! Assignments use the resource's and the task's Unique IDs. The **Resource Names** column shows
//! and reads them as Project writes them: `Ann[50%], Bob, Concrete[2 tons/d]`. A name that isn't
//! a resource yet makes a new work resource, in the same edit.
//!
//! A work assignment keeps its own work and duration; how they change as units, duration and
//! work are edited, and as resources come and go, is the task type's ([`crate::TaskType`]).
//!
//! Each edit is all or nothing: a refused one leaves the project exactly as it was.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::resource::{check_money, fixed_text, parse_fixed};
use crate::task_type::Crew;
use crate::{
    Currency, DurationUnit, FULL_TIME, MAX_MINUTES, MAX_UNITS, MAX_WORK, ModelError, Options, Outline, Project, ResourceId, ResourceType, TaskId,
    check_name, parse_units, units_text,
};

/// Most assignments in one project. ponytail: a cap, not a measured limit.
pub const MAX_ASSIGNMENTS: usize = 1_000_000;
/// Most resources on one task.
pub const MAX_TASK_ASSIGNMENTS: usize = 1_000;
/// Largest material quantity or rate, in hundredths: a million of its label.
pub const MAX_QUANTITY: i64 = 1_000_000 * 100;
/// Longest Resource Names text read, in characters.
pub const MAX_NAMES_TEXT: usize = 100_000;

/// An assignment's units, by its resource's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    /// A work resource: hundredths of a percent of its working time; 10,000 is 100%.
    #[serde(rename = "work")]
    Work(u32),
    /// A material, fixed consumption: hundredths of its label (500 is 5 tons).
    #[serde(rename = "quantity")]
    Fixed(i64),
    /// A material, variable consumption: hundredths of its label per `per` of duration (200 per
    /// day is 2 tons/d).
    #[serde(rename = "rate")]
    Rate { quantity: i64, per: DurationUnit },
    /// A cost resource: the amount, in minor units (pence).
    #[serde(rename = "cost")]
    Cost(i64),
}

impl Units {
    /// The resource type these units belong to.
    pub fn kind(self) -> ResourceType {
        match self {
            Units::Work(_) => ResourceType::Work,
            Units::Fixed(_) | Units::Rate { .. } => ResourceType::Material,
            Units::Cost(_) => ResourceType::Cost,
        }
    }

    /// Refuses units out of range: for a loaded file.
    fn check(self) -> Result<Units, ModelError> {
        let ok = match self {
            Units::Work(u) => u <= MAX_UNITS,
            Units::Fixed(q) | Units::Rate { quantity: q, .. } => (0..=MAX_QUANTITY).contains(&q),
            Units::Cost(m) => check_money(m).is_ok(),
        };
        if ok { Ok(self) } else { Err(ModelError::Invalid("assignment units out of range".into())) }
    }
}

/// One resource on one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub task: TaskId,
    pub resource: ResourceId,
    pub units: Units,
    /// A work resource's work in working minutes: duration × units when it is assigned, then as
    /// the task type says ([`crate::TaskType`]). 0 for a material or cost. A file from before
    /// task types has none: it is worked out on load ([`Project::fill_assignment_work`]).
    #[serde(default = "unset")]
    pub work: i64,
    /// How long a work resource works on the task, in working minutes from its start (work ÷
    /// units); the task's duration is the longest. 0 for a material or cost.
    #[serde(default = "unset")]
    pub duration: i64,
}

/// Work or duration not in the file: filled on load.
const UNSET: i64 = -1;

fn unset() -> i64 {
    UNSET
}

/// How much an assignment uses: working minutes of a work resource, hundredths of a material's
/// label, or nothing for a cost resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentWork {
    Minutes(i64),
    Quantity(i64),
    None,
}

/// The units a new assignment of this resource gets: 100% (or its Max Units if lower), one of a
/// material's label, no cost.
pub fn default_units(kind: ResourceType, max_units: u32) -> Units {
    match kind {
        ResourceType::Work => Units::Work(max_units.min(FULL_TIME)),
        ResourceType::Material => Units::Fixed(100),
        ResourceType::Cost => Units::Cost(0),
    }
}

/// A quantity in hundredths as typed and shown: `5`, `2.5`, `2.25`.
pub fn quantity_text(q: i64) -> String {
    let text = fixed_text(q, 2);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `minutes × rate`, rounded to the nearest whole unit (a half away from zero), clamped to `i64`.
fn scale(minutes: i64, rate: i64, per: i64) -> i64 {
    let per = i128::from(per.max(1));
    let n = (i128::from(minutes) * i128::from(rate) * 2 + per) / (2 * per);
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// What an assignment uses over a task of `duration` working minutes, at the project's `options`
/// (which say how long a day of a `2 tons/d` rate is): a work resource's own work, a material's
/// quantity (a rate scales with the duration).
pub fn assignment_work(a: &Assignment, duration: i64, options: &Options) -> AssignmentWork {
    match a.units {
        Units::Work(_) => AssignmentWork::Minutes(a.work.max(0)),
        Units::Fixed(q) => AssignmentWork::Quantity(q),
        Units::Rate { quantity, per } => AssignmentWork::Quantity(scale(duration, quantity, options.minutes_in(per))),
        Units::Cost(_) => AssignmentWork::None,
    }
}

/// Reads the text between an assignment's brackets for a resource of `kind`: a percentage for a
/// work resource (`50%`, `50`), a quantity with an optional label and rate for a material
/// (`5 tons`, `5`, `2 tons/d`, `2/day`), an amount in the project `currency` for a cost resource
/// (`£200`).
pub fn parse_assignment_units(kind: ResourceType, text: &str, currency: &Currency) -> Result<Units, ModelError> {
    match kind {
        ResourceType::Work => Ok(Units::Work(parse_units(text)?)),
        ResourceType::Cost => Ok(Units::Cost(currency.parse(text)?)),
        ResourceType::Material => {
            let bad = || {
                let text: String = text.trim().chars().take(30).collect();
                ModelError::Invalid(format!("`{text}` is not a material quantity, such as 5 tons or 2 tons/d (up to two decimal places)"))
            };
            let (amount, per) = match text.rsplit_once('/') {
                Some((a, p)) => {
                    (a, Some(DurationUnit::from_word(p.trim().to_lowercase().as_str()).filter(|_| !p.trim().is_empty()).ok_or_else(bad)?))
                }
                None => (text, None),
            };
            let amount = amount.trim();
            // The number, then the label, which is the resource's to say: whatever is typed after
            // the number is not kept.
            let split = amount.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(amount.len());
            let number = amount.get(..split).unwrap_or("");
            let q = parse_fixed(number, 2, MAX_QUANTITY).ok_or_else(bad)?;
            Ok(match per {
                Some(per) => Units::Rate { quantity: q, per },
                None => Units::Fixed(q),
            })
        }
    }
}

/// One part of the Resource Names text: a name and, if bracketed, its units text.
fn split_names(text: &str) -> Result<Vec<(String, Option<String>)>, ModelError> {
    if text.chars().count() > MAX_NAMES_TEXT {
        return Err(ModelError::Invalid(format!("the resource names are longer than {MAX_NAMES_TEXT} characters")));
    }
    let unbalanced = || ModelError::Invalid("the resource names have a [ without its ] (write units as Ann[50%])".into());
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '[' if depth == 0 => {
                depth = 1;
                part.push(c);
            }
            ']' if depth == 1 => {
                depth = 0;
                part.push(c);
            }
            '[' | ']' => return Err(unbalanced()),
            ',' if depth == 0 => parts.push(std::mem::take(&mut part)),
            c => part.push(c),
        }
    }
    if depth != 0 {
        return Err(unbalanced());
    }
    parts.push(part);
    let mut out = Vec::new();
    for p in parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if out.len() >= MAX_TASK_ASSIGNMENTS {
            return Err(ModelError::Invalid(format!("a task takes at most {MAX_TASK_ASSIGNMENTS} resources")));
        }
        match p.split_once('[') {
            None => out.push((p.to_owned(), None)),
            Some((name, rest)) => {
                let inner = rest.strip_suffix(']').filter(|i| !i.contains(']')).ok_or_else(|| {
                    let p: String = p.chars().take(40).collect();
                    ModelError::Invalid(format!("`{p}`: nothing may follow the ] of a resource's units"))
                })?;
                out.push((name.to_owned(), Some(inner.to_owned())));
            }
        }
    }
    Ok(out)
}

impl Project {
    /// A task's assignments, in the order they were made.
    pub fn assignments_of(&self, task: TaskId) -> impl Iterator<Item = &Assignment> {
        self.assignments.iter().filter(move |a| a.task == task)
    }

    /// Whether a resource has any assignment.
    pub fn is_assigned(&self, resource: ResourceId) -> bool {
        self.assignments.iter().any(|a| a.resource == resource)
    }

    /// An assignment's units as the brackets show them, without the brackets: `50%`, `5 tons`,
    /// `2 tons/d`, `£200.00`; empty when Project leaves the brackets off (100% of a work
    /// resource, no cost).
    pub fn units_text(&self, a: &Assignment) -> String {
        let label = self.resource(a.resource).map(|r| r.material_label.clone()).unwrap_or_default();
        let with_label = |q: i64| if label.is_empty() { quantity_text(q) } else { format!("{} {label}", quantity_text(q)) };
        match a.units {
            Units::Work(FULL_TIME) | Units::Cost(0) => String::new(),
            Units::Work(u) => units_text(u),
            Units::Fixed(q) => with_label(q),
            Units::Rate { quantity, per } => format!("{}/{}", with_label(quantity), per.suffix()),
            Units::Cost(m) => self.currency.format(m),
        }
    }

    /// The units as an editor shows them: as [`Project::units_text`], but never empty (`100%`).
    pub fn units_text_full(&self, a: &Assignment) -> String {
        match a.units {
            Units::Work(u) => units_text(u),
            Units::Cost(m) => self.currency.format(m),
            _ => self.units_text(a),
        }
    }

    /// The Resource Names column: `Ann[50%], Bob, Concrete[2 tons/d]`.
    pub fn resource_names(&self, task: TaskId) -> String {
        let one = |a: &Assignment| {
            let name = self.resource(a.resource).map_or_else(|_| "?".to_owned(), |r| r.name.clone());
            match self.units_text(a) {
                u if u.is_empty() => name,
                u => format!("{name}[{u}]"),
            }
        };
        self.assignments_of(task).map(one).collect::<Vec<_>>().join(", ")
    }

    /// Units for this resource, refused when they belong to another type.
    fn units_for(&self, resource: ResourceId, units: Units) -> Result<Units, ModelError> {
        let r = self.resource(resource)?;
        if units.kind() != r.kind {
            return Err(ModelError::Invalid(format!("{} is a {} resource", r.name, r.kind.name().to_lowercase())));
        }
        units.check()
    }

    /// Assigns a resource to a task with `units`, or the default ([`default_units`]). Refused when
    /// it is already assigned there.
    pub fn assign(&mut self, task: TaskId, resource: ResourceId, units: Option<Units>) -> Result<(), ModelError> {
        let mut crew = self.crew(task)?;
        self.assign_to(&mut crew, resource, units, true)?;
        self.put_crew(crew);
        Ok(())
    }

    /// [`Project::assign`] on a task's [`Crew`], which the project's resources and assignment count
    /// are checked against; `share` false keeps an effort-driven task's work from being shared
    /// again (the resource is one of the first, assigned with others).
    fn assign_to(&self, crew: &mut Crew, resource: ResourceId, units: Option<Units>, share: bool) -> Result<(), ModelError> {
        let r = self.resource(resource)?;
        let units = self.units_for(resource, units.unwrap_or_else(|| default_units(r.kind, r.max_units)))?;
        if crew.list.iter().any(|a| a.resource == resource) {
            return Err(ModelError::Invalid(format!("{} is already assigned to this task", r.name)));
        }
        // The project's count as it will be: its own, less the task's as it was, plus the crew's.
        let others = self.assignments.len().saturating_sub(crew.taken());
        if others.saturating_add(crew.list.len()).saturating_add(1) > MAX_ASSIGNMENTS {
            return Err(ModelError::Invalid(format!("a project holds at most {MAX_ASSIGNMENTS} assignments")));
        }
        if crew.list.len() >= MAX_TASK_ASSIGNMENTS {
            return Err(ModelError::Invalid(format!("a task takes at most {MAX_TASK_ASSIGNMENTS} resources")));
        }
        crew.add(Assignment { task: crew.task, resource, units, work: 0, duration: 0 }, share)
    }

    /// Where a resource's assignment is in a crew; refused when it isn't assigned there.
    fn position_in(crew: &Crew, resource: ResourceId) -> Result<usize, ModelError> {
        crew.list
            .iter()
            .position(|a| a.resource == resource)
            .ok_or_else(|| ModelError::Invalid(format!("resource {resource} is not assigned to task {}", crew.task)))
    }

    /// Changes an assignment's units. A work resource's work or duration follows as the task
    /// type says ([`crate::TaskType`]).
    pub fn set_assignment_units(&mut self, task: TaskId, resource: ResourceId, units: Units) -> Result<(), ModelError> {
        let units = self.units_for(resource, units)?;
        let mut crew = self.crew(task)?;
        self.set_units_in(&mut crew, resource, units)?;
        self.put_crew(crew);
        Ok(())
    }

    /// [`Project::set_assignment_units`] on a task's [`Crew`], the units already checked.
    fn set_units_in(&self, crew: &mut Crew, resource: ResourceId, units: Units) -> Result<(), ModelError> {
        let i = Project::position_in(crew, resource)?;
        match units {
            Units::Work(u) => crew.set_units(i, u),
            _ => {
                if let Some(a) = crew.list.get_mut(i) {
                    a.units = units;
                }
                Ok(())
            }
        }
    }

    /// Takes a resource off a task. An effort-driven task keeps its work, shared among the work
    /// resources left ([`crate::TaskType`]).
    pub fn unassign(&mut self, task: TaskId, resource: ResourceId) -> Result<(), ModelError> {
        let mut crew = self.crew(task)?;
        crew.remove(Project::position_in(&crew, resource)?)?;
        self.put_crew(crew);
        Ok(())
    }

    /// Replaces a task's assignments with the Resource Names text, as Project reads the column:
    /// `Ann[50%], Bob, Concrete[2 tons/d]`. A name (any case) that isn't a resource becomes a new
    /// work resource with 100% Max Units, in the same edit. Resources no longer named come off
    /// first, then changed units change, then new ones are assigned, each as its own command
    /// would ([`crate::TaskType`]); the assignments end in the order typed. All or nothing.
    /// Returns the resources it made.
    pub fn set_resource_names(&mut self, task: TaskId, text: &str) -> Result<Vec<ResourceId>, ModelError> {
        self.task(task)?;
        let parts = split_names(text)?;
        let mut p = self.clone();
        let mut made = Vec::new();
        let mut seen = BTreeSet::new();
        let mut wanted = Vec::new();
        for (name, units) in parts {
            let name = check_name(&name)?;
            let lower = name.to_lowercase();
            let id = match p.resources.iter().find(|r| r.name.to_lowercase() == lower) {
                Some(r) => r.id,
                None => {
                    let id = p.add_resource(Some(&name), ResourceType::Work, None)?;
                    made.push(id);
                    id
                }
            };
            if !seen.insert(id) {
                return Err(ModelError::Invalid(format!("{name} is listed twice")));
            }
            let r = p.resource(id)?;
            let units = match units {
                Some(u) => parse_assignment_units(r.kind, &u, &p.currency).map_err(|e| ModelError::Invalid(format!("{}: {e}", r.name)))?,
                // Unchanged when it was already assigned with units Project leaves unshown.
                None => p
                    .assignments_of(task)
                    .find(|a| a.resource == id && p.units_text(a).is_empty())
                    .map_or_else(|| default_units(r.kind, r.max_units), |a| a.units),
            };
            wanted.push((id, units.check()?));
        }
        // One copy of the task's assignments for every step, written back once.
        let mut crew = p.crew(task)?;
        while let Some(i) = crew.list.iter().position(|a| !seen.contains(&a.resource)) {
            crew.remove(i)?;
        }
        // Resources assigned together to a task with no work are its first: none shares another's.
        let first = crew.work_total() == 0;
        for &(id, units) in &wanted {
            if crew.list.iter().any(|a| a.resource == id && a.units != units) {
                let units = p.units_for(id, units)?;
                p.set_units_in(&mut crew, id, units)?;
            }
        }
        for &(id, units) in &wanted {
            if !crew.list.iter().any(|a| a.resource == id) {
                p.assign_to(&mut crew, id, Some(units), !first)?;
            }
        }
        // The order typed.
        crew.list.sort_by_key(|a| wanted.iter().position(|(id, _)| *id == a.resource));
        p.put_crew(crew);
        *self = p;
        Ok(made)
    }

    /// Each task's work in working minutes: its work resources' work, and for a summary its own
    /// assignments' and all its subtasks'. Tasks with none are left out.
    pub fn work_by_task(&self) -> BTreeMap<TaskId, i64> {
        let mut own: BTreeMap<TaskId, i64> = BTreeMap::new();
        for a in &self.assignments {
            let Ok(t) = self.task(a.task) else { continue };
            if let AssignmentWork::Minutes(m) = assignment_work(a, t.duration, &self.options) {
                let w = own.entry(a.task).or_insert(0);
                *w = w.saturating_add(m);
            }
        }
        if own.is_empty() {
            return own;
        }
        let outline = Outline::of(self);
        let mut total = BTreeMap::new();
        // The open summaries above the current row.
        let mut open: Vec<(usize, TaskId)> = Vec::new();
        for (row, t) in self.tasks.iter().enumerate() {
            while open.last().is_some_and(|(r, _)| outline.subtree_end(*r) <= row) {
                open.pop();
            }
            if let Some(w) = own.get(&t.id).copied() {
                for id in std::iter::once(t.id).chain(open.iter().map(|(_, id)| *id)) {
                    let sum = total.entry(id).or_insert(0i64);
                    *sum = sum.saturating_add(w);
                }
            }
            if outline.is_summary(row) {
                open.push((row, t.id));
            }
        }
        total
    }

    /// Refuses assignments no command could have made (for a loaded file): too many, to a
    /// missing task or resource, twice to one task, units of another type or out of range.
    pub fn check_assignments(&self) -> Result<(), ModelError> {
        if self.assignments.len() > MAX_ASSIGNMENTS {
            return Err(ModelError::Invalid(format!("more than {MAX_ASSIGNMENTS} assignments")));
        }
        let tasks: BTreeSet<TaskId> = self.tasks.iter().map(|t| t.id).collect();
        let kinds: BTreeMap<ResourceId, ResourceType> = self.resources.iter().map(|r| (r.id, r.kind)).collect();
        let mut pairs = BTreeSet::new();
        let mut per_task: BTreeMap<TaskId, usize> = BTreeMap::new();
        for a in &self.assignments {
            if !tasks.contains(&a.task) {
                return Err(ModelError::Invalid(format!("an assignment names missing task {}", a.task)));
            }
            let kind = kinds.get(&a.resource).ok_or_else(|| ModelError::Invalid(format!("an assignment names missing resource {}", a.resource)))?;
            if a.units.kind() != *kind {
                return Err(ModelError::Invalid(format!("resource {}'s assignment has units of another resource type", a.resource)));
            }
            a.units.check()?;
            let (work, duration) = match a.units {
                Units::Work(_) => ((0..=MAX_WORK).contains(&a.work), (0..=MAX_MINUTES).contains(&a.duration)),
                _ => (a.work == 0, a.duration == 0),
            };
            if !(work && duration) {
                return Err(ModelError::Invalid(format!(
                    "resource {}'s assignment to task {} has its work or duration out of range",
                    a.resource, a.task
                )));
            }
            if !pairs.insert((a.task, a.resource)) {
                return Err(ModelError::Invalid(format!("resource {} is assigned to task {} twice", a.resource, a.task)));
            }
            let n = per_task.entry(a.task).or_insert(0);
            *n += 1;
            if *n > MAX_TASK_ASSIGNMENTS {
                return Err(ModelError::Invalid(format!("task {} has more than {MAX_TASK_ASSIGNMENTS} resources", a.task)));
            }
        }
        Ok(())
    }

    /// Works out each assignment's work and duration where a file has none (from before task
    /// types): a work resource works the task's duration at its units; a material or cost has
    /// none. Then repairs each task's duration to its longest work assignment's (those with
    /// work), as every edit keeps it, so a file that disagrees still opens. Summaries' are rolled
    /// up when scheduled. Run on load, before [`Project::check_assignments`].
    pub fn fill_assignment_work(&mut self) {
        let durations: BTreeMap<TaskId, i64> = self.tasks.iter().map(|t| (t.id, t.duration.clamp(0, MAX_MINUTES))).collect();
        for a in &mut self.assignments {
            let d = durations.get(&a.task).copied().unwrap_or(0);
            let (work, duration) = match a.units {
                Units::Work(u) => (crate::task_type::work_of(d, u), d),
                _ => (0, 0),
            };
            if a.work == UNSET {
                a.work = work;
            }
            if a.duration == UNSET {
                a.duration = duration;
            }
        }
        // Only durations in range: one that isn't is refused by the check that follows.
        let mut longest: BTreeMap<TaskId, i64> = BTreeMap::new();
        for a in self.assignments.iter().filter(|a| matches!(a.units, Units::Work(_)) && a.work > 0 && (0..=MAX_MINUTES).contains(&a.duration)) {
            let d = longest.entry(a.task).or_insert(0);
            *d = (*d).max(a.duration);
        }
        if longest.is_empty() {
            return;
        }
        let outline = Outline::of(self);
        for (row, t) in self.tasks.iter_mut().enumerate() {
            if let Some(d) = longest.get(&t.id).filter(|_| !outline.is_summary(row)) {
                t.duration = *d;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Date, ResourceField};

    /// A day of duration at the default 8 hours.
    const D: i64 = 480;

    /// An assignment of these units, for [`assignment_work`].
    fn of(units: Units) -> Assignment {
        Assignment { task: TaskId(1), resource: ResourceId(1), units, work: 0, duration: 0 }
    }

    fn project() -> (Project, TaskId) {
        let mut p = Project::new("t", Date::from_ymd(2026, 1, 5).unwrap());
        let t = p.add_task("Pour", 5 * D).unwrap();
        p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
        let bob = p.add_resource(Some("Bob"), ResourceType::Work, None).unwrap();
        p.set_resource(bob, &[ResourceField::MaxUnits(5_000)]).unwrap();
        let c = p.add_resource(Some("Concrete"), ResourceType::Material, None).unwrap();
        p.set_resource(c, &[ResourceField::MaterialLabel("tons".into())]).unwrap();
        p.add_resource(Some("Travel"), ResourceType::Cost, None).unwrap();
        (p, t)
    }

    #[test]
    fn resource_names_read_and_write_as_project_does() {
        let (mut p, t) = project();
        let made = p.set_resource_names(t, " ann[50%] ,Bob, Concrete[2 tons/d], Travel[£1,200.50]").unwrap();
        assert!(made.is_empty());
        assert_eq!(p.resource_names(t), "Ann[50%], Bob[50%], Concrete[2 tons/d], Travel[£1,200.50]");
        let units: Vec<Units> = p.assignments_of(t).map(|a| a.units).collect();
        assert_eq!(
            units,
            [Units::Work(5_000), Units::Work(5_000), Units::Rate { quantity: 200, per: DurationUnit::Days }, Units::Cost(120_050)],
            "Bob takes his Max Units (50%), lower than 100%"
        );
        // Round trip, and an empty text removes them all.
        let text = p.resource_names(t);
        let before = p.clone();
        p.set_resource_names(t, &text).unwrap();
        assert_eq!(p, before);
        p.set_resource_names(t, "  ").unwrap();
        assert!(p.assignments.is_empty());
        // 100% shows without brackets; a fixed material and a bare amount.
        p.set_resource_names(t, "Ann[100%], Concrete[5], Travel").unwrap();
        assert_eq!(p.resource_names(t), "Ann, Concrete[5 tons], Travel");
        p.set_resource_names(t, "Concrete[2.5 boxes/wk]").unwrap();
        assert_eq!(p.resource_names(t), "Concrete[2.5 tons/w]", "the label is the resource's");
    }

    #[test]
    fn bad_resource_names_change_nothing() {
        let (mut p, t) = project();
        p.set_resource_names(t, "Ann").unwrap();
        let before = p.clone();
        for bad in [
            "Ann[50%",
            "Ann]",
            "Ann[[50%]]",
            "Ann[50%]x",
            "Ann, ann",
            "Ann[lots]",
            "Ann[-5%]",
            "Concrete[5 tons/fortnight]",
            "Concrete[x]",
            "Concrete[/d]",
            "Concrete[2/]",
            "Concrete[99999999999]",
            "Travel[-£5]",
            "[50%]",
            "New one, Ann[x]",
        ] {
            assert!(p.set_resource_names(t, bad).is_err(), "{bad}");
            assert_eq!(p, before, "{bad}");
        }
        assert!(p.set_resource_names(TaskId(99), "Ann").is_err());
        let many = vec!["x"; MAX_TASK_ASSIGNMENTS + 1].join(",");
        assert!(p.set_resource_names(t, &many).is_err());
    }

    #[test]
    fn an_unknown_name_makes_a_work_resource_at_full_time() {
        let (mut p, t) = project();
        let made = p.set_resource_names(t, "Ann, Sam Reid[25%], Pat").unwrap();
        assert_eq!(made.len(), 2);
        let sam = p.resource(made[0]).unwrap();
        assert_eq!((sam.name.as_str(), sam.kind, sam.max_units), ("Sam Reid", ResourceType::Work, FULL_TIME));
        assert_eq!(p.resource_names(t), "Ann, Sam Reid[25%], Pat");
    }

    #[test]
    fn assign_change_and_remove_by_unique_id() {
        let (mut p, t) = project();
        let (ann, bob, concrete, travel) = (ResourceId(1), ResourceId(2), ResourceId(3), ResourceId(4));
        p.assign(t, bob, None).unwrap();
        p.assign(t, ann, Some(Units::Work(2_500))).unwrap();
        assert!(p.assign(t, ann, None).is_err(), "already assigned");
        assert!(p.assign(t, concrete, Some(Units::Work(1))).is_err(), "units of another type");
        assert!(p.assign(t, ResourceId(9), None).is_err());
        assert!(p.assign(TaskId(9), ann, None).is_err());
        assert!(p.assign(t, travel, Some(Units::Cost(-1))).is_err());
        p.assign(t, concrete, None).unwrap();
        assert_eq!(p.resource_names(t), "Bob[50%], Ann[25%], Concrete[1 tons]");
        p.set_assignment_units(t, concrete, Units::Rate { quantity: 300, per: DurationUnit::Hours }).unwrap();
        assert!(p.set_assignment_units(t, travel, Units::Cost(5)).is_err(), "not assigned");
        assert!(p.set_assignment_units(t, ann, Units::Work(MAX_UNITS + 1)).is_err());
        p.unassign(t, bob).unwrap();
        assert!(p.unassign(t, bob).is_err());
        assert_eq!(p.resource_names(t), "Ann[25%], Concrete[3 tons/h]");
        p.check_assignments().unwrap();
    }

    /// Microsoft's "Learn more about resource units": steel at 80 tons on a 2-day task uses 80
    /// tons; at 80 tons/day it uses 160 tons, and as the duration changes, so does the quantity.
    /// <https://support.microsoft.com/en-us/project/learn-more-about-resource-units>
    #[test]
    fn material_fixed_and_variable_quantities_worked_example() {
        let (mut p, t) = project();
        p.task_mut(t).unwrap().duration = 2 * D;
        let o = p.options;
        let fixed = parse_assignment_units(ResourceType::Material, "80 tons", &Currency::default()).unwrap();
        let variable = parse_assignment_units(ResourceType::Material, "80 tons/day", &Currency::default()).unwrap();
        assert_eq!(assignment_work(&of(fixed), 2 * D, &o), AssignmentWork::Quantity(8_000));
        assert_eq!(assignment_work(&of(variable), 2 * D, &o), AssignmentWork::Quantity(16_000));
        assert_eq!(assignment_work(&of(fixed), 4 * D, &o), AssignmentWork::Quantity(8_000), "fixed: the same whatever the duration");
        assert_eq!(assignment_work(&of(variable), 4 * D, &o), AssignmentWork::Quantity(32_000), "variable: in proportion");
        // A rate per hour over half a day: 4 hours × 2.5 = 10.
        let hourly = parse_assignment_units(ResourceType::Material, "2.5/h", &Currency::default()).unwrap();
        assert_eq!(assignment_work(&of(hourly), 240, &o), AssignmentWork::Quantity(1_000));
        assert_eq!(assignment_work(&of(Units::Cost(500)), 2 * D, &o), AssignmentWork::None);
    }

    /// Work = Duration × Units, from Microsoft's "Assignment Units fields": a 5-day task at 50% is
    /// 20 hours of work. <https://support.microsoft.com/en-us/project/assignment-units-fields>
    /// A summary's work is its own assignments' and its subtasks'; summaries take resources.
    #[test]
    fn work_is_duration_times_units_and_summaries_roll_it_up() {
        let (mut p, t) = project();
        let sub = p.add_task("Cure", 2 * D).unwrap();
        p.indent(&[sub]).unwrap();
        p.set_resource_names(t, "Ann, Concrete[2 tons/d]").unwrap();
        p.set_resource_names(sub, "Ann[50%], Bob[25%]").unwrap();
        assert_eq!(crate::task_type::work_of(5 * D, 5_000), 20 * 60);
        let work = p.work_by_task();
        // The subtask: 2 days × 75% = 12 hours. The summary: 5 days of its own (the duration it
        // stores) at 100% plus its subtask's; the material adds no hours.
        assert_eq!(work.get(&sub), Some(&(12 * 60)));
        assert_eq!(work.get(&t), Some(&(40 * 60 + 12 * 60)));
        assert!(p.add_task("Idle", D).is_ok());
        assert_eq!(p.work_by_task().len(), 2, "a task with no work resource has none");
    }

    #[test]
    fn deleting_a_task_or_resource_or_changing_a_type_drops_its_assignments() {
        let (mut p, t) = project();
        let other = p.add_task("Other", D).unwrap();
        p.set_resource_names(t, "Ann, Bob, Concrete").unwrap();
        p.set_resource_names(other, "Ann, Bob").unwrap();
        p.delete_resources(&[ResourceId(1)]).unwrap();
        assert_eq!(p.resource_names(t), "Bob[50%], Concrete[1 tons]");
        p.set_resource(ResourceId(2), &[ResourceField::Type(ResourceType::Material)]).unwrap();
        assert_eq!(p.resource_names(t), "Concrete[1 tons]");
        assert_eq!(p.resource_names(other), "");
        // Setting the type it already has keeps its assignments.
        p.set_resource(ResourceId(3), &[ResourceField::Type(ResourceType::Material)]).unwrap();
        assert_eq!(p.resource_names(t), "Concrete[1 tons]");
        p.delete_tasks(&[t]).unwrap();
        assert!(p.assignments.is_empty());
    }

    #[test]
    fn loaded_assignments_are_checked() {
        let (mut p, t) = project();
        p.set_resource_names(t, "Ann, Concrete[5]").unwrap();
        p.check_assignments().unwrap();
        let a =
            |task: u32, resource: u32, units: Units| Assignment { task: TaskId(task), resource: ResourceId(resource), units, work: 0, duration: 0 };
        for broken in [
            a(9, 1, Units::Work(1)),
            a(1, 9, Units::Work(1)),
            a(1, 2, Units::Cost(1)),
            a(1, 1, Units::Work(1)),
            a(1, 2, Units::Work(MAX_UNITS + 1)),
            a(1, 3, Units::Fixed(-1)),
            a(1, 4, Units::Cost(crate::MAX_MONEY + 1)),
        ] {
            let mut q = p.clone();
            q.assignments.push(broken);
            assert!(q.check_assignments().is_err(), "{broken:?}");
        }
    }

    proptest::proptest! {
        /// Any text: never a panic; what is read writes back to the same assignments.
        #[test]
        fn resource_names_never_panic_and_round_trip(s in "\\PC{0,60}") {
            let (mut p, t) = project();
            if p.set_resource_names(t, &s).is_ok() {
                let text = p.resource_names(t);
                let before = p.clone();
                proptest::prop_assert!(p.set_resource_names(t, &text).is_ok(), "{text}");
                proptest::prop_assert_eq!(&p, &before);
            }
            for kind in ResourceType::ALL {
                let _ = parse_assignment_units(kind, &s, &Currency::default());
            }
        }

        #[test]
        fn typed_names_from_the_grammar_round_trip(
            parts in proptest::collection::vec(("[A-Za-z][A-Za-z ]{0,8}", proptest::option::of("[0-9]{1,3}(\\.[0-9]{1,2})?%?")), 0..6)
        ) {
            let text = parts.iter().map(|(n, u)| match u {
                Some(u) => format!("{n}[{u}]"),
                None => n.clone(),
            }).collect::<Vec<_>>().join(", ");
            let (mut p, t) = project();
            if p.set_resource_names(t, &text).is_ok() {
                let shown = p.resource_names(t);
                let before = p.clone();
                p.set_resource_names(t, &shown).unwrap();
                proptest::prop_assert_eq!(p, before);
            }
        }
    }
}
