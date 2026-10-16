//! Resources: the people, equipment, materials and costs a project's tasks use, as Microsoft
//! Project's Resource Sheet. Each has a row ID (its place in the sheet, which renumbers) and a
//! Unique ID (permanent, never reused); commands and assignments use the Unique ID.
//!
//! Money is whole minor units (pence) and units are whole hundredths of a percent, so nothing here
//! is a float and the same plan always gives the same numbers. No contact details are held.
//!
//! Each edit is all or nothing: a refused one leaves the project exactly as it was.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ModelError, Project, TaskId, UnitsFormat};

/// Most resources in one project. ponytail: a cap, not a measured limit.
pub const MAX_RESOURCES: usize = 100_000;
/// Longest resource name, initials, group or material label, in characters.
pub const MAX_RESOURCE_TEXT: usize = 255;
/// Max Units is held in hundredths of a percent: 10,000 is 100% (full time).
pub const UNITS_PER_PERCENT: u32 = 100;
/// Full time: 100%.
pub const FULL_TIME: u32 = 100 * UNITS_PER_PERCENT;
/// Highest Max Units accepted: 10,000% (a pool of a hundred people).
pub const MAX_UNITS: u32 = 10_000 * UNITS_PER_PERCENT;
/// Largest amount of money accepted, in minor units (ten billion pounds in pence). Keeps every
/// cost sum in range.
pub const MAX_MONEY: i64 = 1_000_000_000_000;
/// The currency symbol shown until a project carries its own currency.
pub const DEFAULT_CURRENCY_SYMBOL: &str = "£";
/// Decimal places of the default currency's minor unit (pence).
pub const DEFAULT_MINOR_DIGITS: u32 = 2;
/// Characters a resource name can't hold: the Resource Names column separates and qualifies
/// names with them (`Ann[50%], Bob`), as in Microsoft Project.
pub const NAME_FORBIDDEN: [char; 3] = ['[', ']', ','];

/// A resource's Unique ID: permanent for the life of the project, never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub u32);

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What kind of resource it is, as Microsoft Project's Type field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ResourceType {
    /// A person or piece of equipment whose time is booked.
    #[default]
    Work,
    /// A consumable counted in its material label (tons, boxes).
    Material,
    /// A cost that doesn't depend on work, such as travel.
    Cost,
}

impl ResourceType {
    pub const ALL: [ResourceType; 3] = [ResourceType::Work, ResourceType::Material, ResourceType::Cost];
    pub const NAMES: [&'static str; 3] = ["Work", "Material", "Cost"];

    pub fn name(self) -> &'static str {
        match self {
            ResourceType::Work => "Work",
            ResourceType::Material => "Material",
            ResourceType::Cost => "Cost",
        }
    }

    /// From its name or the start of it, any case: `w`, `mat`, `Cost`.
    pub fn from_name(text: &str) -> Result<ResourceType, ModelError> {
        pick(text, &ResourceType::ALL, ResourceType::name, "resource type")
    }

    fn is_work(&self) -> bool {
        *self == ResourceType::Work
    }
}

/// When a resource's cost is incurred, as Microsoft Project's Accrue At field. Held now; used
/// once actual costs are tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AccrueAt {
    Start,
    End,
    /// Spread over the work as it is done.
    #[default]
    Prorated,
}

impl AccrueAt {
    pub const ALL: [AccrueAt; 3] = [AccrueAt::Start, AccrueAt::End, AccrueAt::Prorated];
    pub const NAMES: [&'static str; 3] = ["Start", "End", "Prorated"];

    pub fn name(self) -> &'static str {
        match self {
            AccrueAt::Start => "Start",
            AccrueAt::End => "End",
            AccrueAt::Prorated => "Prorated",
        }
    }

    /// From its name or the start of it, any case.
    pub fn from_name(text: &str) -> Result<AccrueAt, ModelError> {
        pick(text, &AccrueAt::ALL, AccrueAt::name, "Accrue At value")
    }

    fn is_prorated(&self) -> bool {
        *self == AccrueAt::Prorated
    }
}

/// The one choice whose name starts with `text` (any case); refused when none or several do.
pub(crate) fn pick<T: Copy>(text: &str, all: &[T], name: fn(T) -> &'static str, what: &str) -> Result<T, ModelError> {
    let typed = text.trim().to_lowercase();
    let names = || all.iter().map(|t| name(*t)).collect::<Vec<_>>().join(", ");
    let hits: Vec<T> = all.iter().copied().filter(|t| !typed.is_empty() && name(*t).to_lowercase().starts_with(&typed)).collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        _ => {
            let text: String = text.chars().take(20).collect();
            Err(ModelError::Invalid(format!("`{text}` is not a {what}; expected one of {}", names())))
        }
    }
}

/// One row of the Resource Sheet. Fields that don't apply to its type (Max Units of a material)
/// are kept as they were, unshown, so switching type back and forth loses nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub id: ResourceId,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initials: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(rename = "type", default, skip_serializing_if = "ResourceType::is_work")]
    pub kind: ResourceType,
    /// Work resources: hundredths of a percent ([`UNITS_PER_PERCENT`]); 10,000 is 100%.
    #[serde(default = "full_time", skip_serializing_if = "is_full_time")]
    pub max_units: u32,
    /// Material resources: what it is counted in (tons, boxes).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub material_label: String,
    /// Minor units per hour of work; for a material, per unit of its label.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub standard_rate: i64,
    /// Minor units per hour of overtime work (work resources).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub overtime_rate: i64,
    /// Minor units charged once per assignment.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cost_per_use: i64,
    #[serde(default, skip_serializing_if = "AccrueAt::is_prorated")]
    pub accrue_at: AccrueAt,
}

fn full_time() -> u32 {
    FULL_TIME
}

fn is_full_time(u: &u32) -> bool {
    *u == FULL_TIME
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// A resource field that can be set, with its new value. Which apply depends on the type:
/// [`Resource::takes`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceField {
    Initials(String),
    Group(String),
    Type(ResourceType),
    MaxUnits(u32),
    MaterialLabel(String),
    StandardRate(i64),
    OvertimeRate(i64),
    CostPerUse(i64),
    AccrueAt(AccrueAt),
}

impl ResourceField {
    /// The field's name as the Resource Sheet heads its column.
    pub fn label(&self) -> &'static str {
        match self {
            ResourceField::Initials(_) => "Initials",
            ResourceField::Group(_) => "Group",
            ResourceField::Type(_) => "Type",
            ResourceField::MaxUnits(_) => "Max Units",
            ResourceField::MaterialLabel(_) => "Material",
            ResourceField::StandardRate(_) => "Standard Rate",
            ResourceField::OvertimeRate(_) => "Overtime Rate",
            ResourceField::CostPerUse(_) => "Cost/Use",
            ResourceField::AccrueAt(_) => "Accrue At",
        }
    }
}

impl Resource {
    pub fn new(id: ResourceId, name: String, kind: ResourceType) -> Resource {
        let initials = name.chars().next().map(String::from).unwrap_or_default();
        Resource {
            id,
            name,
            initials,
            group: String::new(),
            kind,
            max_units: FULL_TIME,
            material_label: String::new(),
            standard_rate: 0,
            overtime_rate: 0,
            cost_per_use: 0,
            accrue_at: AccrueAt::default(),
        }
    }

    /// Whether a field applies to this resource's type, as in Microsoft Project: Max Units and
    /// Overtime Rate to work resources, the material label to materials, Standard Rate and
    /// Cost/Use to both; cost resources take only the shared fields.
    pub fn takes(kind: ResourceType, field: &ResourceField) -> bool {
        use ResourceType::{Material, Work};
        match field {
            ResourceField::Initials(_) | ResourceField::Group(_) | ResourceField::Type(_) | ResourceField::AccrueAt(_) => true,
            ResourceField::MaxUnits(_) | ResourceField::OvertimeRate(_) => kind == Work,
            ResourceField::MaterialLabel(_) => kind == Material,
            ResourceField::StandardRate(_) | ResourceField::CostPerUse(_) => kind != ResourceType::Cost,
        }
    }

    /// Refuses a field no command could have set: for a loaded file.
    pub fn check(&self) -> Result<(), ModelError> {
        if self.id.0 == 0 {
            return Err(ModelError::Invalid("resource id 0".into()));
        }
        if check_name(&self.name)? != self.name {
            return Err(ModelError::Invalid(format!("resource {}: the name has spaces or control characters at its ends", self.id)));
        }
        for (what, text) in [("initials", &self.initials), ("group", &self.group), ("material label", &self.material_label)] {
            if clean(text) != *text {
                return Err(ModelError::Invalid(format!("resource {}: the {what} are too long or hold control characters", self.id)));
            }
        }
        check_units(self.max_units)?;
        for n in [self.standard_rate, self.overtime_rate, self.cost_per_use] {
            check_money(n)?;
        }
        Ok(())
    }
}

/// Text as a resource field holds it: one line, trimmed, at most [`MAX_RESOURCE_TEXT`] characters.
pub fn clean(text: &str) -> String {
    let one_line: String = text.chars().filter(|c| !c.is_control()).collect();
    one_line.trim().chars().take(MAX_RESOURCE_TEXT).collect::<String>().trim_end().to_owned()
}

/// A resource name, cleaned ([`clean`]); refused when empty or holding `[`, `]` or `,`.
pub fn check_name(text: &str) -> Result<String, ModelError> {
    let name = clean(text);
    if name.is_empty() {
        return Err(ModelError::Invalid("a resource needs a name".into()));
    }
    if name.contains(NAME_FORBIDDEN) {
        return Err(ModelError::Invalid("a resource name can't contain [ ] or a comma".into()));
    }
    Ok(name)
}

pub fn check_units(units: u32) -> Result<u32, ModelError> {
    if units <= MAX_UNITS {
        Ok(units)
    } else {
        Err(ModelError::Invalid(format!("Max Units must be between 0% and {}%", MAX_UNITS / UNITS_PER_PERCENT)))
    }
}

pub fn check_money(minor: i64) -> Result<i64, ModelError> {
    if (0..=MAX_MONEY).contains(&minor) {
        Ok(minor)
    } else {
        Err(ModelError::Invalid(format!("an amount must be between 0 and {}", format_money(MAX_MONEY, "", DEFAULT_MINOR_DIGITS))))
    }
}

/// Reads a non-negative decimal into whole units of `10^-digits`: `"12.5"` with 2 digits is 1250.
/// Refuses more decimal places than `digits`, signs, exponents and anything over `max`. Exact:
/// no floating point.
pub(crate) fn parse_fixed(text: &str, digits: u32, max: i64) -> Option<i64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if (whole.is_empty() && frac.is_empty()) || !whole.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    if frac.len() > usize::try_from(digits).ok()? {
        return None;
    }
    let scale = 10i64.checked_pow(digits)?;
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let pad = digits.saturating_sub(u32::try_from(frac.len()).ok()?);
    let frac: i64 = if frac.is_empty() { 0 } else { frac.parse::<i64>().ok()?.checked_mul(10i64.checked_pow(pad)?)? };
    let n = whole.checked_mul(scale)?.checked_add(frac)?;
    (n <= max).then_some(n)
}

/// Writes `n` units of `10^-digits` as a decimal with exactly `digits` places.
pub(crate) fn fixed_text(n: i64, digits: u32) -> String {
    let scale = 10u64.saturating_pow(digits);
    let (sign, n) = (if n < 0 { "-" } else { "" }, n.unsigned_abs());
    if digits == 0 {
        return format!("{sign}{n}");
    }
    format!("{sign}{}.{:0width$}", n / scale, n % scale, width = digits as usize)
}

/// Max Units typed as Microsoft Project's percent format: `50%` or `50`, up to two decimal places.
pub fn parse_units(text: &str) -> Result<u32, ModelError> {
    parse_units_as(text, UnitsFormat::Percentage)
}

/// Units as typed when the project shows them in `format`: a number with `%` is always a
/// percentage (`50%`, up to two decimal places); a bare number is a percentage (`50`) or, as a
/// decimal, a share of full time (`0.5`, `1`, up to four decimal places).
pub fn parse_units_as(text: &str, format: UnitsFormat) -> Result<u32, ModelError> {
    let max = i64::from(MAX_UNITS);
    let t = text.trim();
    let n = match t.strip_suffix('%') {
        Some(percent) => parse_fixed(percent.trim(), 2, max),
        None if format == UnitsFormat::Decimal => parse_fixed(t, 4, max),
        None => parse_fixed(t, 2, max),
    };
    n.and_then(|n| u32::try_from(n).ok()).ok_or_else(|| {
        let text: String = text.chars().take(20).collect();
        let most = MAX_UNITS / UNITS_PER_PERCENT;
        ModelError::Invalid(match format {
            UnitsFormat::Percentage => format!("`{text}` is not a percentage between 0% and {most}%"),
            UnitsFormat::Decimal => format!("`{text}` is not units between 0 and {} (1 is full time) or a percentage", most / 100),
        })
    })
}

/// Max Units as the sheet shows it: `100%`, `50%`, `37.5%`.
pub fn units_text(units: u32) -> String {
    units_text_as(units, UnitsFormat::Percentage)
}

/// Units as shown in `format`: `100%`, `37.5%` or `1`, `0.375`.
pub fn units_text_as(units: u32, format: UnitsFormat) -> String {
    match format {
        UnitsFormat::Percentage => {
            let text = fixed_text(i64::from(units), 2);
            format!("{}%", text.trim_end_matches('0').trim_end_matches('.'))
        }
        UnitsFormat::Decimal => {
            let text = fixed_text(i64::from(units), 4);
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
    }
}

/// An amount typed into a money cell: an optional currency `symbol`, digits with optional
/// thousands commas, up to `digits` decimal places, and an optional `/h` or `/hr` on a rate.
/// Returns whole minor units.
pub fn parse_money(text: &str, symbol: &str, digits: u32) -> Result<i64, ModelError> {
    let bad = || {
        let text: String = text.chars().take(30).collect();
        ModelError::Invalid(format!("`{text}` is not an amount of money (up to {digits} decimal places, not negative)"))
    };
    let mut t = text.trim();
    for suffix in ["/hour", "/hr", "/h"] {
        if let Some(rest) = t.strip_suffix(suffix) {
            t = rest.trim_end();
            break;
        }
    }
    if !symbol.is_empty() {
        t = t.strip_prefix(symbol).unwrap_or(t).trim_start();
    }
    let t: String = t.chars().filter(|c| *c != ',').collect();
    let n = parse_fixed(&t, digits, i64::MAX).ok_or_else(bad)?;
    check_money(n)
}

/// An amount as the sheet shows it: `£1,234.50`.
pub fn format_money(minor: i64, symbol: &str, digits: u32) -> String {
    let text = fixed_text(minor, digits);
    let (sign, body) = text.strip_prefix('-').map_or(("", text.as_str()), |b| ("-", b));
    let (whole, frac) = body.split_once('.').map_or((body, None), |(w, f)| (w, Some(f)));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{symbol}{grouped}.{f}"),
        None => format!("{sign}{symbol}{grouped}"),
    }
}

impl Project {
    pub fn resource(&self, id: ResourceId) -> Result<&Resource, ModelError> {
        self.resources.iter().find(|r| r.id == id).ok_or_else(|| ModelError::Invalid(format!("no resource {id}")))
    }

    fn resource_mut(&mut self, id: ResourceId) -> Result<&mut Resource, ModelError> {
        self.resources.iter_mut().find(|r| r.id == id).ok_or_else(|| ModelError::Invalid(format!("no resource {id}")))
    }

    /// The 1-based row a resource is on in the Resource Sheet: its ID, which renumbers.
    pub fn resource_row_of(&self, id: ResourceId) -> Option<usize> {
        self.resources.iter().position(|r| r.id == id).map(|i| i.saturating_add(1))
    }

    /// Whether another resource already has this name (any case).
    fn name_taken(&self, name: &str, except: Option<ResourceId>) -> bool {
        let lower = name.to_lowercase();
        self.resources.iter().any(|r| Some(r.id) != except && r.name.to_lowercase() == lower)
    }

    /// A name no resource has yet, for a resource added without one: `Resource 4`.
    fn free_name(&self, id: ResourceId) -> String {
        let mut n = id.0;
        // Bounded: each existing resource can block at most one number.
        for _ in 0..=self.resources.len() {
            let name = format!("Resource {n}");
            if !self.name_taken(&name, None) {
                return name;
            }
            n = n.saturating_add(1);
        }
        format!("Resource {id}+")
    }

    /// Adds a resource above `before`, or at the end, and returns its new Unique ID. A missing
    /// name gets one that isn't taken; a given one must be free (any case).
    pub fn add_resource(&mut self, name: Option<&str>, kind: ResourceType, before: Option<ResourceId>) -> Result<ResourceId, ModelError> {
        if self.resources.len() >= MAX_RESOURCES {
            return Err(ModelError::Invalid(format!("a project holds at most {MAX_RESOURCES} resources")));
        }
        let row = match before {
            Some(b) => self.resource_row_of(b).and_then(|r| r.checked_sub(1)).ok_or_else(|| ModelError::Invalid(format!("no resource {b}")))?,
            None => self.resources.len(),
        };
        let id = ResourceId(self.next_resource_id.max(1));
        let next = id.0.checked_add(1).ok_or_else(|| ModelError::Invalid("no Unique IDs left for resources".into()))?;
        let name = match name {
            Some(n) => {
                let n = check_name(n)?;
                if self.name_taken(&n, None) {
                    return Err(ModelError::Invalid(format!("there is already a resource called {n}")));
                }
                n
            }
            None => self.free_name(id),
        };
        self.resources.insert(row, Resource::new(id, name, kind));
        self.next_resource_id = next;
        Ok(id)
    }

    pub fn rename_resource(&mut self, id: ResourceId, name: &str) -> Result<(), ModelError> {
        let name = check_name(name)?;
        if self.name_taken(&name, Some(id)) {
            return Err(ModelError::Invalid(format!("there is already a resource called {name}")));
        }
        self.resource_mut(id)?.name = name;
        Ok(())
    }

    /// Sets fields of one resource, in order, all or nothing. A field that doesn't apply to the
    /// resource's type (after any `Type` earlier in the list) is refused.
    pub fn set_resource(&mut self, id: ResourceId, fields: &[ResourceField]) -> Result<(), ModelError> {
        let mut r = self.resource(id)?.clone();
        for f in fields {
            if !Resource::takes(r.kind, f) {
                return Err(ModelError::Invalid(format!("a {} resource has no {}", r.kind.name().to_lowercase(), f.label())));
            }
            match f {
                ResourceField::Initials(s) => r.initials = clean(s),
                ResourceField::Group(s) => r.group = clean(s),
                // Changing the type drops the resource's assignments below (as Microsoft Project);
                // the sheet asks first.
                ResourceField::Type(k) => r.kind = *k,
                ResourceField::MaxUnits(u) => r.max_units = check_units(*u)?,
                ResourceField::MaterialLabel(s) => r.material_label = clean(s),
                ResourceField::StandardRate(n) => r.standard_rate = check_money(*n)?,
                ResourceField::OvertimeRate(n) => r.overtime_rate = check_money(*n)?,
                ResourceField::CostPerUse(n) => r.cost_per_use = check_money(*n)?,
                ResourceField::AccrueAt(a) => r.accrue_at = *a,
            }
        }
        let retyped = self.resource(id)?.kind != r.kind;
        *self.resource_mut(id)? = r;
        if retyped {
            let tasks: BTreeSet<TaskId> = self.assignments.iter().filter(|a| a.resource == id).map(|a| a.task).collect();
            self.assignments.retain(|a| a.resource != id);
            self.settle_all(&tasks);
        }
        Ok(())
    }

    /// Deletes resources and their assignments, all or nothing: refused if any is missing. Their
    /// Unique IDs are never handed out again.
    pub fn delete_resources(&mut self, ids: &[ResourceId]) -> Result<(), ModelError> {
        for id in ids {
            self.resource(*id)?;
        }
        let gone: BTreeSet<ResourceId> = ids.iter().copied().collect();
        self.resources.retain(|r| !gone.contains(&r.id));
        // Each task's duration is then its longest work assignment left.
        let tasks: BTreeSet<TaskId> = self.assignments.iter().filter(|a| gone.contains(&a.resource)).map(|a| a.task).collect();
        self.assignments.retain(|a| !gone.contains(&a.resource));
        self.settle_all(&tasks);
        Ok(())
    }

    /// Moves resources above `to` (to the end if `None`), keeping their order. Refused if any is
    /// missing or `to` is one of them.
    pub fn move_resources(&mut self, ids: &[ResourceId], to: Option<ResourceId>) -> Result<(), ModelError> {
        for id in ids {
            self.resource(*id)?;
        }
        let moving: BTreeSet<ResourceId> = ids.iter().copied().collect();
        if let Some(t) = to {
            self.resource(t)?;
            if moving.contains(&t) {
                return Ok(());
            }
        }
        let (taken, mut kept): (Vec<Resource>, Vec<Resource>) = std::mem::take(&mut self.resources).into_iter().partition(|r| moving.contains(&r.id));
        let at = to.and_then(|t| kept.iter().position(|r| r.id == t)).unwrap_or(kept.len());
        kept.splice(at..at, taken);
        self.resources = kept;
        Ok(())
    }

    /// Refuses resources no command could have made (for a loaded file): too many, a Unique ID of
    /// 0 or used twice, a field out of range, two with the same name; and assignments to a missing
    /// task or resource ([`Project::check_assignments`]). Repairs `next_resource_id` so a Unique ID
    /// is never reused.
    pub fn check_resources(&mut self) -> Result<(), ModelError> {
        if self.resources.len() > MAX_RESOURCES {
            return Err(ModelError::Invalid(format!("more than {MAX_RESOURCES} resources")));
        }
        let (mut ids, mut names) = (BTreeSet::new(), BTreeSet::new());
        for r in &self.resources {
            r.check()?;
            if !ids.insert(r.id) {
                return Err(ModelError::Invalid(format!("resource id {} is used twice", r.id)));
            }
            if !names.insert(r.name.to_lowercase()) {
                return Err(ModelError::Invalid(format!("two resources are called {}", r.name)));
            }
        }
        let max = ids.last().map_or(0, |r| r.0);
        self.next_resource_id = self.next_resource_id.max(max.saturating_add(1)).max(1);
        self.check_assignments()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Date;

    fn project() -> Project {
        Project::new("t", Date::from_ymd(2026, 1, 5).unwrap())
    }

    #[test]
    fn unique_ids_are_never_reused_and_rows_renumber() {
        let mut p = project();
        let a = p.add_resource(Some(" Ann  Lee "), ResourceType::Work, None).unwrap();
        let b = p.add_resource(None, ResourceType::Material, None).unwrap();
        let c = p.add_resource(Some("Crane"), ResourceType::Work, Some(a)).unwrap();
        assert_eq!(p.resource(a).unwrap().name, "Ann  Lee");
        assert_eq!(p.resource(a).unwrap().initials, "A");
        assert_eq!(p.resource(b).unwrap().name, "Resource 2");
        assert_eq!((p.resource_row_of(c), p.resource_row_of(a), p.resource_row_of(b)), (Some(1), Some(2), Some(3)));
        p.delete_resources(&[c]).unwrap();
        assert_eq!(p.resource_row_of(a), Some(1));
        assert_eq!(p.add_resource(None, ResourceType::Cost, None).unwrap(), ResourceId(4));
        assert!(p.delete_resources(&[a, ResourceId(99)]).is_err());
        assert!(p.resource(a).is_ok(), "all or nothing");
        p.move_resources(&[a], None).unwrap();
        assert_eq!(p.resources.iter().map(|r| r.id.0).collect::<Vec<_>>(), [2, 4, 1]);
        p.move_resources(&[ResourceId(1), ResourceId(4)], Some(ResourceId(2))).unwrap();
        assert_eq!(p.resources.iter().map(|r| r.id.0).collect::<Vec<_>>(), [4, 1, 2]);
    }

    #[test]
    fn names_are_cleaned_unique_and_never_hold_separators() {
        let mut p = project();
        p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
        assert!(p.add_resource(Some("ann"), ResourceType::Work, None).is_err());
        assert!(p.add_resource(Some("   "), ResourceType::Work, None).is_err());
        for bad in ["Ann[50%]", "Ann, Bob", "x]"] {
            assert!(p.add_resource(Some(bad), ResourceType::Work, None).is_err(), "{bad}");
        }
        let long = "x".repeat(1000);
        let id = p.add_resource(Some(&format!("\u{7}{long}\n")), ResourceType::Work, None).unwrap();
        assert_eq!(p.resource(id).unwrap().name.chars().count(), MAX_RESOURCE_TEXT);
        assert!(p.rename_resource(id, "ANN").is_err());
        p.rename_resource(id, "Bob").unwrap();
        p.rename_resource(id, "bob").unwrap();
        // A name already taken by a typed name is skipped by an unnamed add.
        p.rename_resource(id, "Resource 3").unwrap();
        let n = p.add_resource(None, ResourceType::Work, None).unwrap();
        assert_eq!(p.resource(n).unwrap().name, "Resource 4");
    }

    #[test]
    fn fields_follow_the_type_and_stay_in_range() {
        let mut p = project();
        let id = p.add_resource(Some("Concrete"), ResourceType::Material, None).unwrap();
        let before = p.clone();
        assert!(p.set_resource(id, &[ResourceField::MaterialLabel("tons".into()), ResourceField::MaxUnits(5000)]).is_err());
        assert_eq!(p, before, "all or nothing");
        p.set_resource(id, &[ResourceField::MaterialLabel(" tons ".into()), ResourceField::StandardRate(12_50)]).unwrap();
        assert!(p.set_resource(id, &[ResourceField::OvertimeRate(1)]).is_err());
        p.set_resource(id, &[ResourceField::Type(ResourceType::Work), ResourceField::MaxUnits(5000)]).unwrap();
        let r = p.resource(id).unwrap();
        assert_eq!((r.kind, r.max_units, r.material_label.as_str(), r.standard_rate), (ResourceType::Work, 5000, "tons", 1250));
        assert!(p.set_resource(id, &[ResourceField::MaxUnits(MAX_UNITS + 1)]).is_err());
        assert!(p.set_resource(id, &[ResourceField::StandardRate(-1)]).is_err());
        assert!(p.set_resource(id, &[ResourceField::CostPerUse(MAX_MONEY + 1)]).is_err());
        p.set_resource(id, &[ResourceField::Type(ResourceType::Cost)]).unwrap();
        assert!(p.set_resource(id, &[ResourceField::StandardRate(1)]).is_err());
        p.set_resource(id, &[ResourceField::AccrueAt(AccrueAt::End), ResourceField::Group("Site".into())]).unwrap();
        assert!(p.set_resource(ResourceId(9), &[]).is_err());
    }

    #[test]
    fn units_and_money_read_and_write_exactly() {
        assert_eq!(parse_units("50%").unwrap(), 5000);
        assert_eq!(parse_units(" 37.5 ").unwrap(), 3750);
        assert_eq!(parse_units("0").unwrap(), 0);
        assert_eq!(parse_units("10000%").unwrap(), MAX_UNITS);
        for bad in ["", "%", "-5%", "1e3", "10000.01", "12.345", "abc", "99999999999999999999999"] {
            assert!(parse_units(bad).is_err(), "{bad}");
        }
        assert_eq!(units_text(FULL_TIME), "100%");
        assert_eq!(units_text(3750), "37.5%");
        assert_eq!(units_text(3725), "37.25%");
        assert_eq!(units_text(5), "0.05%");
        assert_eq!(units_text(0), "0%");
    }

    /// Units as decimals, as Project's "Show assignment units as a: Decimal" option
    /// (<https://support.microsoft.com/en-us/office/change-the-default-unit-format-for-assignments-d1490024-3ce2-4552-8327-6c1cb6e7ca07>):
    /// full time is 1; a number with `%` is still a percentage.
    #[test]
    fn units_read_and_write_as_decimals() {
        let d = UnitsFormat::Decimal;
        assert_eq!(units_text_as(FULL_TIME, d), "1");
        assert_eq!(units_text_as(5000, d), "0.5");
        assert_eq!(units_text_as(3750, d), "0.375");
        assert_eq!(units_text_as(5, d), "0.0005");
        assert_eq!(units_text_as(0, d), "0");
        assert_eq!(units_text_as(MAX_UNITS, d), "100");
        assert_eq!(parse_units_as("0.5", d).unwrap(), 5000);
        assert_eq!(parse_units_as(" 1 ", d).unwrap(), FULL_TIME);
        assert_eq!(parse_units_as(".375", d).unwrap(), 3750);
        assert_eq!(parse_units_as("50%", d).unwrap(), 5000, "a percentage either way");
        assert_eq!(parse_units_as("50", UnitsFormat::Percentage).unwrap(), 5000);
        for bad in ["", "%", "-1", "100.0001", "0.00001", "1e2", "lots", "0.5%%", "101"] {
            assert!(parse_units_as(bad, d).is_err(), "{bad}");
        }
        assert!(parse_units_as("x", d).unwrap_err().to_string().contains("1 is full time"));

        assert_eq!(parse_money("£12.50/hr", "£", 2).unwrap(), 1250);
        assert_eq!(parse_money("1,234", "£", 2).unwrap(), 123_400);
        assert_eq!(parse_money(".5", "£", 2).unwrap(), 50);
        assert_eq!(parse_money("7", "", 0).unwrap(), 7);
        for bad in ["", "£", "-1", "1.234", "1e5", "£ x", "10000000001", "$5"] {
            assert!(parse_money(bad, "£", 2).is_err(), "{bad}");
        }
        assert_eq!(format_money(123_456_789, "£", 2), "£1,234,567.89");
        assert_eq!(format_money(5, "£", 2), "£0.05");
        assert_eq!(format_money(0, "£", 2), "£0.00");
        assert_eq!(format_money(1000, "¥", 0), "¥1,000");
        assert_eq!(format_money(MAX_MONEY, "£", 2), "£10,000,000,000.00");
    }

    #[test]
    fn a_loaded_resource_list_is_checked_and_next_id_repaired() {
        let mut p = project();
        p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
        p.resources.push(Resource::new(ResourceId(40), "Bob".into(), ResourceType::Work));
        p.next_resource_id = 3;
        p.check_resources().unwrap();
        assert_eq!(p.next_resource_id, 41);
        let mut q = p.clone();
        q.resources.push(Resource::new(ResourceId(40), "Cy".into(), ResourceType::Work));
        assert!(q.check_resources().is_err());
        let mut q = p.clone();
        q.resources.push(Resource::new(ResourceId(41), "bob".into(), ResourceType::Work));
        assert!(q.check_resources().is_err());
        for broken in [
            Resource { max_units: MAX_UNITS + 1, ..Resource::new(ResourceId(50), "X".into(), ResourceType::Work) },
            Resource { standard_rate: -1, ..Resource::new(ResourceId(50), "X".into(), ResourceType::Work) },
            Resource { name: " X".into(), ..Resource::new(ResourceId(50), "X".into(), ResourceType::Work) },
            Resource { name: "A,B".into(), ..Resource::new(ResourceId(50), "X".into(), ResourceType::Work) },
            Resource { group: "a\nb".into(), ..Resource::new(ResourceId(50), "X".into(), ResourceType::Work) },
            Resource::new(ResourceId(0), "X".into(), ResourceType::Work),
        ] {
            let mut q = p.clone();
            q.resources.push(broken);
            assert!(q.check_resources().is_err());
        }
    }

    proptest::proptest! {
        #[test]
        fn typed_units_and_money_never_panic(s in "\\PC{0,40}") {
            for format in UnitsFormat::ALL {
                if let Ok(u) = parse_units_as(&s, format) {
                    // Whichever format it was typed in, each writes it back exactly.
                    for shown in UnitsFormat::ALL {
                        proptest::prop_assert_eq!(parse_units_as(&units_text_as(u, shown), shown).ok(), Some(u));
                    }
                }
            }
            if let Ok(m) = parse_money(&s, "£", 2) {
                proptest::prop_assert_eq!(parse_money(&format_money(m, "£", 2), "£", 2).ok(), Some(m));
            }
            let _ = check_name(&s);
            let _ = ResourceType::from_name(&s);
        }
    }
}
