//! Resources, assignments and their costs from Microsoft Project XML, by Microsoft's published
//! schema documentation: `Resource`
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/xml-schema-for-the-resources-element>)
//! and `Assignment`
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/xml-schema-for-the-assignments-element>).
//!
//! - A resource's `Type` is 0 material, 1 work, 2 cost (newer files also mark a cost resource with
//!   `IsCostResource`). `MaxUnits` is a share of full time (`1` is 100%). `StandardRate`,
//!   `OvertimeRate` and `CostPerUse` are decimal amounts of the project currency; a work
//!   resource's rates are per hour, as Microsoft's Project Server reference has the same fields
//!   ("the rate of pay per hour", `RES_STD_RATE`), `StandardRateFormat` being only the unit they
//!   are shown in. A material's rate is per unit of its `MaterialLabel`. `AccrueAt` is 1 start,
//!   2 end, 3 prorated. A resource's `CalendarUID` names its own calendar.
//! - `Rates` holds the cost rate tables, `RateTable` 0 to 4 (A to E). ProjectCraft holds one rate
//!   per resource, the resource's own fields: a table B to E with any amount in it is dropped, a
//!   report line a table, and so are rates that change over time in table A.
//! - An assignment's `Units` are a share of full time for a work resource (`0.5` is 50%) and a
//!   quantity of its label for a material; its `Work` is an `xs:duration`; a cost resource's
//!   amount is its `Cost`. `WorkContour` 0 is flat; any other contour (1 back loaded, 2 front
//!   loaded, 3 double peak, 4 early peak, 5 late peak, 6 bell, 7 turtle, 8 contoured) is made flat,
//!   a report line each.
//!
//! Every value is hostile: counts are capped at the model's ([`MAX_RESOURCES`],
//! [`MAX_ASSIGNMENTS`], [`MAX_TASK_ASSIGNMENTS`]), amounts, units and work are clamped to what the
//! model holds, money is read exactly ([`number`]), and unknown codes are report lines.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::model::{
    AccrueAt, Assignment, FULL_TIME, MAX_ASSIGNMENTS, MAX_EXCEPTIONS, MAX_MINUTES, MAX_MONEY, MAX_QUANTITY, MAX_RESOURCE_TEXT, MAX_RESOURCES,
    MAX_TASK_ASSIGNMENTS, MAX_UNITS, MAX_WORK, Outline, Project, Resource, ResourceCalendar, ResourceId, ResourceType, TaskId, Units, check_name,
    default_units,
};
use crate::{Outcome, ReportLine};

use super::number::{self, Read};
use super::{child, duration_minutes, int, is, line, named_list, plural, truthy};

const RESOURCE: &[&str] = &["Project", "Resources", "Resource"];
const RATE: &[&str] = &["Project", "Resources", "Resource", "Rates", "Rate"];
const AVAILABILITY: &[&str] = &["Project", "Resources", "Resource", "AvailabilityPeriods", "AvailabilityPeriod"];
const ASSIGNMENT: &[&str] = &["Project", "Assignments", "Assignment"];

/// Rates kept of one resource; the schema allows 25 (five tables of five periods).
const MAX_RATES: usize = 25;
/// Longest text kept of a number or code field.
const MAX_FIELD: usize = 64;
/// Exception days given to resources across the project; past it a resource's own exceptions are
/// dropped (a file sharing one big calendar among many resources can't multiply it).
const MAX_RESOURCE_EXCEPTION_DAYS: usize = 2 * MAX_EXCEPTIONS;
/// Contour names by `WorkContour` code.
const CONTOURS: [&str; 9] = ["Flat", "Back Loaded", "Front Loaded", "Double Peak", "Early Peak", "Late Peak", "Bell", "Turtle", "Contoured"];

fn kept(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_FIELD).collect())
}

/// Text as a resource field holds it: one line, trimmed, at most [`MAX_RESOURCE_TEXT`] characters
/// (the model's own rule, which a loaded resource is checked against).
fn clean(text: &str) -> String {
    let one_line: String = text.chars().filter(|c| !c.is_control()).collect();
    one_line.trim().chars().take(MAX_RESOURCE_TEXT).collect::<String>().trim_end().to_owned()
}

/// Longer than a name or label is kept, but cut so a hostile one costs nothing.
fn text_kept(text: &str) -> String {
    text.trim().chars().take(MAX_RESOURCE_TEXT.saturating_mul(2)).collect()
}

/// One `Rate` of a resource's `Rates`.
#[derive(Debug, Default)]
struct FileRate {
    table: Option<i64>,
    standard: Option<String>,
    overtime: Option<String>,
    per_use: Option<String>,
}

/// A resource as the file has it.
#[derive(Debug, Default)]
struct FileResource {
    uid: Option<i64>,
    id: Option<i64>,
    name: String,
    kind: Option<String>,
    cost: bool,
    null: bool,
    initials: String,
    group: String,
    material_label: String,
    max_units: Option<String>,
    standard_rate: Option<String>,
    overtime_rate: Option<String>,
    cost_per_use: Option<String>,
    accrue_at: Option<String>,
    calendar: Option<i64>,
    rates: Vec<FileRate>,
    availability: usize,
    details: bool,
    budget: bool,
}

/// An assignment as the file has it.
#[derive(Debug, Default)]
struct FileAssignment {
    task: Option<i64>,
    resource: Option<i64>,
    units: Option<String>,
    /// `None` if absent, `Some(None)` if unreadable.
    work: Option<Option<i64>>,
    cost: Option<String>,
    contour: Option<i64>,
    table: Option<i64>,
    fixed_material: Option<bool>,
    overtime: bool,
    delay: bool,
    progress: bool,
    timephased: bool,
}

/// Reads the `Resources` and `Assignments` collections as the file is walked.
#[derive(Debug, Default)]
pub(super) struct Reader {
    resources: Vec<FileResource>,
    resource: Option<FileResource>,
    rate: Option<FileRate>,
    /// Resources past [`MAX_RESOURCES`], only counted.
    resource_overflow: usize,
    /// Null resources (`IsNull`), blank rows of the sheet.
    blank: usize,
    assignments: Vec<FileAssignment>,
    assignment: Option<FileAssignment>,
    /// Assignments past [`MAX_ASSIGNMENTS`], only counted.
    assignment_overflow: usize,
}

/// Whether `path` is inside `Resources` or `Assignments`.
fn ours(path: &[String]) -> bool {
    matches!(path, [p, c, ..] if p == "Project" && (c == "Resources" || c == "Assignments"))
}

impl Reader {
    /// An element opened; whether it was one of these collections'.
    pub(super) fn open(&mut self, path: &[String]) -> bool {
        if !ours(path) {
            return false;
        }
        if is(path, RESOURCE) {
            self.resource = Some(FileResource::default());
        } else if is(path, RATE) {
            self.rate = Some(FileRate::default());
        } else if is(path, AVAILABILITY) {
            if let Some(r) = self.resource.as_mut() {
                r.availability = r.availability.saturating_add(1);
            }
        } else if is(path, ASSIGNMENT) {
            self.assignment = Some(FileAssignment::default());
        } else if child(path, ASSIGNMENT) == Some("TimephasedData")
            && let Some(a) = self.assignment.as_mut()
        {
            a.timephased = true;
        }
        true
    }

    /// An element closed with its text; whether it was one of these collections'.
    pub(super) fn close(&mut self, path: &[String], text: &str) -> bool {
        if !ours(path) {
            return false;
        }
        if is(path, RESOURCE) {
            if let Some(r) = self.resource.take() {
                self.keep_resource(r);
            }
        } else if is(path, RATE) {
            if let (Some(rate), Some(r)) = (self.rate.take(), self.resource.as_mut())
                && r.rates.len() < MAX_RATES
            {
                r.rates.push(rate);
            }
        } else if let Some(name) = child(path, RATE) {
            if let Some(rate) = self.rate.as_mut() {
                match name {
                    "RateTable" => rate.table = int(text),
                    "StandardRate" => rate.standard = kept(text),
                    "OvertimeRate" => rate.overtime = kept(text),
                    "CostPerUse" => rate.per_use = kept(text),
                    _ => {}
                }
            }
        } else if let Some(name) = child(path, RESOURCE) {
            if let Some(r) = self.resource.as_mut() {
                resource_field(r, name, text);
            }
        } else if is(path, ASSIGNMENT) {
            if let Some(a) = self.assignment.take() {
                self.keep_assignment(a);
            }
        } else if let Some(name) = child(path, ASSIGNMENT)
            && let Some(a) = self.assignment.as_mut()
        {
            assignment_field(a, name, text);
        }
        true
    }

    fn keep_resource(&mut self, r: FileResource) {
        // UID 0 is Project's own placeholder, not a resource.
        if r.uid == Some(0) {
            return;
        }
        if r.null {
            self.blank = self.blank.saturating_add(1);
        } else if self.resources.len() < MAX_RESOURCES {
            self.resources.push(r);
        } else {
            self.resource_overflow = self.resource_overflow.saturating_add(1);
        }
    }

    fn keep_assignment(&mut self, a: FileAssignment) {
        // A task without resources has an assignment to resource -65535.
        if a.resource.is_some_and(|r| r < 0) {
            return;
        }
        if self.assignments.len() < MAX_ASSIGNMENTS {
            self.assignments.push(a);
        } else {
            self.assignment_overflow = self.assignment_overflow.saturating_add(1);
        }
    }

    /// The calendars the resources name, by UID.
    pub(super) fn calendar_uids(&self) -> BTreeSet<i64> {
        self.resources.iter().filter_map(|r| r.calendar).filter(|c| *c >= 0).collect()
    }
}

fn resource_field(r: &mut FileResource, name: &str, text: &str) {
    match name {
        "UID" => r.uid = int(text),
        "ID" => r.id = int(text),
        "Name" => r.name = text_kept(text),
        "Type" => r.kind = kept(text),
        "IsCostResource" => r.cost = truthy(text),
        "IsNull" => r.null = truthy(text),
        "Initials" => r.initials = text_kept(text),
        "Group" => r.group = text_kept(text),
        "MaterialLabel" => r.material_label = text_kept(text),
        "MaxUnits" => r.max_units = kept(text),
        "StandardRate" => r.standard_rate = kept(text),
        "OvertimeRate" => r.overtime_rate = kept(text),
        "CostPerUse" => r.cost_per_use = kept(text),
        "AccrueAt" => r.accrue_at = kept(text),
        "CalendarUID" => r.calendar = int(text),
        "IsBudget" => r.budget |= truthy(text),
        "EmailAddress" | "Notes" | "Hyperlink" | "HyperlinkAddress" | "HyperlinkSubAddress" | "Code" | "NTAccount" | "Phonetics" => {
            r.details |= !text.trim().is_empty();
        }
        _ => {}
    }
}

fn assignment_field(a: &mut FileAssignment, name: &str, text: &str) {
    let nonzero_work = || duration_minutes(text).is_some_and(|m| m != 0);
    let nonzero = || number::scaled(text, 4).is_some_and(|(v, _)| v != 0);
    match name {
        "TaskUID" => a.task = int(text),
        "ResourceUID" => a.resource = int(text),
        "Units" => a.units = kept(text),
        "Work" => a.work = Some(duration_minutes(text)),
        "Cost" => a.cost = kept(text),
        "WorkContour" => a.contour = int(text),
        "CostRateTable" => a.table = int(text),
        "FixedMaterial" => a.fixed_material = Some(truthy(text)),
        "OvertimeWork" => a.overtime |= nonzero_work(),
        "Delay" | "LevelingDelay" => a.delay |= nonzero(),
        "ActualWork" | "ActualOvertimeWork" => a.progress |= nonzero_work(),
        "ActualCost" | "ActualOvertimeCost" | "PercentWorkComplete" => a.progress |= nonzero(),
        "ActualStart" | "ActualFinish" => a.progress |= !text.trim().is_empty(),
        _ => {}
    }
}

/// Something about a resource that the report mentions. The order is the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ResNote {
    Name,
    UnknownType,
    MaxUnits,
    Money,
    Rounded,
    Accrue,
    Budget,
    CalendarBase,
    CalendarMissing,
    NoWorkingWeekday,
    CalendarFull,
    TableB,
    TableC,
    TableD,
    TableE,
    RatesOverTime,
    Availability,
    Details,
}

impl ResNote {
    const ALL: [ResNote; 18] = [
        ResNote::Name,
        ResNote::UnknownType,
        ResNote::MaxUnits,
        ResNote::Money,
        ResNote::Rounded,
        ResNote::Accrue,
        ResNote::Budget,
        ResNote::CalendarBase,
        ResNote::CalendarMissing,
        ResNote::NoWorkingWeekday,
        ResNote::CalendarFull,
        ResNote::TableB,
        ResNote::TableC,
        ResNote::TableD,
        ResNote::TableE,
        ResNote::RatesOverTime,
        ResNote::Availability,
        ResNote::Details,
    ];

    fn bit(self) -> u32 {
        1 << (self as u8)
    }

    /// Outcome, feature and why.
    fn line(self) -> (Outcome, &'static str, &'static str) {
        use Outcome::*;
        const TABLE: &str = "ProjectCraft holds one set of rates per resource (table A); this table's rates are dropped";
        match self {
            ResNote::Name => (
                Converted,
                "Resource names",
                "empty, used twice (in any case) or holding [ ] or a comma, which separate names in the Resource Names column; made unique, with ( ) and ; instead",
            ),
            ResNote::UnknownType => {
                (Converted, "Resource type", "the type code isn't one of Project's (0 material, 1 work, 2 cost); a work resource")
            }
            ResNote::MaxUnits => (Converted, "Max Units", "unreadable or outside 0% to 10,000%; brought within it (100% if unreadable)"),
            ResNote::Money => {
                (Converted, "Rates and costs", "negative, unreadable or larger than ProjectCraft holds; 0 or the largest amount (ten billion) used")
            }
            ResNote::Rounded => {
                (Converted, "Amounts finer than the currency", "rounded half to even to the currency's smallest unit (the penny for pounds)")
            }
            ResNote::Accrue => (Converted, "Accrue At", "the code isn't one of Project's (1 start, 2 end, 3 prorated); Prorated used"),
            ResNote::Budget => (Converted, "Budget resources", "ProjectCraft has no budget resources; imported as ordinary resources"),
            ResNote::CalendarBase => (
                Converted,
                "Resource calendar base",
                "the resource's calendar is based on another base calendar than the project's; that calendar's working week is copied (not its exceptions) and the rest follows the project calendar",
            ),
            ResNote::CalendarMissing => {
                (Converted, "Resource calendar", "the calendar the resource names isn't in the file; the resource follows the project calendar")
            }
            ResNote::NoWorkingWeekday => {
                (Converted, "Resource working week", "the resource's calendar has no working weekday; it is scheduled on the project calendar")
            }
            ResNote::CalendarFull => (
                Dropped,
                "Resource calendar exceptions",
                "the file's resource calendars hold more exception days than ProjectCraft takes in one import; these resources' own exceptions are dropped",
            ),
            ResNote::TableB => (Dropped, "Cost rate table B", TABLE),
            ResNote::TableC => (Dropped, "Cost rate table C", TABLE),
            ResNote::TableD => (Dropped, "Cost rate table D", TABLE),
            ResNote::TableE => (Dropped, "Cost rate table E", TABLE),
            ResNote::RatesOverTime => {
                (Dropped, "Rates over time", "the resource's rates change on later dates; ProjectCraft holds one set, the resource's current rates")
            }
            ResNote::Availability => {
                (Dropped, "Resource availability", "availability that changes over time; ProjectCraft holds one Max Units for all time")
            }
            ResNote::Details => {
                (Dropped, "Resource details", "e-mail addresses, notes, hyperlinks, codes and accounts; ProjectCraft holds no contact details")
            }
        }
    }
}

/// Something about an assignment that the report mentions. The order is the report's; the eight
/// contours other than flat come first, by code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsgNote {
    Contour(usize),
    Units,
    Work,
    Cost,
    Rounded,
    CostTable,
    VariableMaterial,
    Overtime,
    Delay,
    Progress,
    Timephased,
}

impl AsgNote {
    const ALL: [AsgNote; 18] = [
        AsgNote::Contour(1),
        AsgNote::Contour(2),
        AsgNote::Contour(3),
        AsgNote::Contour(4),
        AsgNote::Contour(5),
        AsgNote::Contour(6),
        AsgNote::Contour(7),
        AsgNote::Contour(8),
        AsgNote::Units,
        AsgNote::Work,
        AsgNote::Cost,
        AsgNote::Rounded,
        AsgNote::CostTable,
        AsgNote::VariableMaterial,
        AsgNote::Overtime,
        AsgNote::Delay,
        AsgNote::Progress,
        AsgNote::Timephased,
    ];

    fn bit(self) -> u32 {
        let i = AsgNote::ALL.iter().position(|n| *n == self).unwrap_or(0);
        1 << i
    }

    /// Outcome, feature and why.
    fn line(self) -> (Outcome, String, &'static str) {
        use Outcome::*;
        let (outcome, feature, why) = match self {
            AsgNote::Contour(c) => {
                return (
                    Converted,
                    format!("Work contour: {}", CONTOURS.get(c).copied().unwrap_or("Unknown")),
                    "ProjectCraft spreads an assignment's work evenly over its duration; made flat (Project stretches a contoured assignment, so it may now finish sooner)",
                );
            }
            AsgNote::Units => (
                Converted,
                "Assignment units",
                "unreadable, negative or past what ProjectCraft holds (10,000% of a work resource, a million of a material); brought within it",
            ),
            AsgNote::Work => (Converted, "Assignment work", "unreadable, negative or longer than ProjectCraft holds; brought within it"),
            AsgNote::Cost => (Converted, "Assignment cost", "negative, unreadable or larger than ProjectCraft holds; 0 or the largest amount used"),
            AsgNote::Rounded => {
                (Converted, "Amounts finer than the currency", "rounded half to even to the currency's smallest unit (the penny for pounds)")
            }
            AsgNote::CostTable => {
                (Converted, "Cost rate table on assignments", "costed at the resource's own rates (table A), the only ones ProjectCraft holds")
            }
            AsgNote::VariableMaterial => (
                Converted,
                "Variable material consumption",
                "a material used at a rate over the task's duration; read as a fixed quantity of the file's units",
            ),
            AsgNote::Overtime => (Dropped, "Overtime work", "ProjectCraft doesn't schedule overtime yet; all the work is regular work"),
            AsgNote::Delay => (Dropped, "Assignment delay", "each assignment starts with its task in ProjectCraft"),
            AsgNote::Progress => (Dropped, "Assignment progress", "tracking comes in a later version"),
            AsgNote::Timephased => (Dropped, "Assignment timephased data", "ProjectCraft works out each day's work itself"),
        };
        (outcome, feature.to_owned(), why)
    }
}

/// A resource calendar as it is imported: its own week and exceptions, the name of another base
/// calendar it was based on, and lines about its exceptions.
#[derive(Debug, Clone, Default)]
pub(super) struct BuiltCalendar {
    pub(super) calendar: ResourceCalendar,
    pub(super) other_base: bool,
    pub(super) lines: Vec<ReportLine>,
}

/// Why an assignment was left out. The order is the report's.
const LEFT_OUT: [&str; 6] = [
    "to the project summary task, which ProjectCraft rolls up itself",
    "its task isn't in the file, or was a blank row",
    "its resource isn't in the file, or was left out",
    "the resource is already assigned to that task",
    "a task takes at most 1,000 resources",
    "a project holds at most 1,000,000 assignments",
];

/// An amount of a resource in minor units of a currency with `digits` places (0 when absent), noting a rounding or a clamp.
fn money(text: Option<&str>, digits: u32, notes: &mut u32) -> i64 {
    let Some(text) = text else { return 0 };
    let (v, read) = number::within(text, digits, MAX_MONEY, 0);
    match read {
        Read::Exact => {}
        Read::Rounded => *notes |= ResNote::Rounded.bit(),
        Read::OutOfRange => *notes |= ResNote::Money.bit(),
    }
    v
}

/// `amount × num ÷ den`, rounded to the nearest (a half away from zero), as the model rounds
/// work and duration; saturates.
fn ratio(amount: i64, num: i64, den: i64) -> i64 {
    if den <= 0 {
        return 0;
    }
    let n = i128::from(amount).saturating_mul(i128::from(num));
    let d = i128::from(den);
    let q = (n.saturating_mul(2).saturating_add(d)).div_euclid(d.saturating_mul(2));
    i64::try_from(q).unwrap_or(i64::MAX)
}

/// Unique names for the resources, cleaned as the model holds them, with whether each changed.
fn names(file: &[FileResource], ids: &[ResourceId]) -> Vec<(String, bool)> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(file.len());
    for (r, id) in file.iter().zip(ids) {
        let swapped: String = r
            .name
            .chars()
            .map(|c| match c {
                '[' => '(',
                ']' => ')',
                ',' => ';',
                c => c,
            })
            .collect();
        let base = check_name(&swapped).unwrap_or_else(|_| format!("Resource {id}"));
        let mut name = base.clone();
        let mut n = 2u64;
        // Bounded: each resource before it can take at most one suffix.
        while taken.contains(&name.to_lowercase()) && n <= u64::try_from(file.len()).unwrap_or(u64::MAX).saturating_add(2) {
            let suffix = format!(" ({n})");
            let room = MAX_RESOURCE_TEXT.saturating_sub(suffix.chars().count());
            name = format!("{}{suffix}", base.chars().take(room).collect::<String>().trim_end());
            n += 1;
        }
        taken.insert(name.to_lowercase());
        let changed = name != r.name;
        out.push((name, changed));
    }
    out
}

/// Builds the project's resources and assignments from what was read, with their report lines.
/// `task_uids` is each task's Unique ID in the file, by row; `calendars` the resource calendars
/// the resources name, built; amounts are in the project's currency.
pub(super) fn apply(p: &mut Project, reader: Reader, task_uids: &[Option<i64>], calendars: &BTreeMap<i64, BuiltCalendar>) -> Vec<ReportLine> {
    let mut lines = Vec::new();
    let Reader { resources: mut file, resource_overflow, blank, assignments, assignment_overflow, .. } = reader;
    let digits = p.currency.digits;
    if blank > 0 {
        lines.push(line(Outcome::Dropped, "Blank resource rows", &plural(blank, "row"), "ProjectCraft has no blank rows"));
    }
    if resource_overflow > 0 {
        lines.push(line(
            Outcome::Dropped,
            "Resources",
            &plural(resource_overflow, "resource"),
            &format!("a project holds at most {} resources; the rest and their assignments are dropped", super::thousands(MAX_RESOURCES)),
        ));
    }
    if file.iter().all(|r| r.id.is_some()) {
        file.sort_by_key(|r| r.id);
    }
    // The file's Unique IDs when every one is usable: positive, unique, and leaving room to add.
    let room = u32::MAX - u32::try_from(MAX_RESOURCES).unwrap_or(u32::MAX / 2);
    let mut seen = HashSet::new();
    let keep_uids = file.iter().all(|r| r.uid.and_then(|u| u32::try_from(u).ok()).is_some_and(|u| u >= 1 && u < room && seen.insert(u)));
    if !keep_uids && !file.is_empty() {
        lines.push(line(
            Outcome::Converted,
            "Resource Unique IDs",
            "Resources",
            "missing, repeated or out of range in the file; the resources are numbered from 1",
        ));
    }
    let ids: Vec<ResourceId> = file
        .iter()
        .enumerate()
        .map(|(row, r)| match (keep_uids, r.uid.and_then(|u| u32::try_from(u).ok())) {
            (true, Some(u)) => ResourceId(u),
            _ => ResourceId(u32::try_from(row).unwrap_or(u32::MAX - 1).saturating_add(1)),
        })
        .collect();
    // File UID to resource, the first of any repeated.
    let mut by_uid: HashMap<i64, usize> = HashMap::new();
    for (i, r) in file.iter().enumerate() {
        if let Some(u) = r.uid {
            by_uid.entry(u).or_insert(i);
        }
    }
    let named = names(&file, &ids);
    let mut notes = vec![0u32; file.len()];
    let mut exception_days = 0usize;
    let mut out = Vec::with_capacity(file.len());
    for (i, (r, id)) in file.iter().zip(&ids).enumerate() {
        let mut n = 0u32;
        let kind = match r.kind.as_deref().map(int) {
            _ if r.cost => ResourceType::Cost,
            None | Some(Some(1)) => ResourceType::Work,
            Some(Some(0)) => ResourceType::Material,
            Some(Some(2)) => ResourceType::Cost,
            Some(_) => {
                n |= ResNote::UnknownType.bit();
                ResourceType::Work
            }
        };
        let (name, renamed) = named.get(i).cloned().unwrap_or_else(|| (format!("Resource {id}"), true));
        if renamed {
            n |= ResNote::Name.bit();
        }
        let mut res = Resource::new(*id, name, kind);
        let initials = clean(&r.initials);
        if !initials.is_empty() {
            res.initials = initials;
        }
        res.group = clean(&r.group);
        res.material_label = clean(&r.material_label);
        if let Some(text) = r.max_units.as_deref() {
            let (u, read) = number::within(text, 4, i64::from(MAX_UNITS), i64::from(FULL_TIME));
            if read == Read::OutOfRange {
                n |= ResNote::MaxUnits.bit();
            }
            res.max_units = u32::try_from(u).unwrap_or(FULL_TIME);
        }
        // The resource's own fields are its current rates; table A's first rate stands in for any
        // the file leaves out.
        let table_a = r.rates.iter().find(|x| x.table.unwrap_or(0) == 0);
        let standard = r.standard_rate.as_deref().or_else(|| table_a.and_then(|a| a.standard.as_deref()));
        let overtime = r.overtime_rate.as_deref().or_else(|| table_a.and_then(|a| a.overtime.as_deref()));
        let per_use = r.cost_per_use.as_deref().or_else(|| table_a.and_then(|a| a.per_use.as_deref()));
        res.standard_rate = money(standard, digits, &mut n);
        res.overtime_rate = money(overtime, digits, &mut n);
        res.cost_per_use = money(per_use, digits, &mut n);
        res.accrue_at = match r.accrue_at.as_deref().map(int) {
            None | Some(Some(3)) => AccrueAt::Prorated,
            Some(Some(1)) => AccrueAt::Start,
            Some(Some(2)) => AccrueAt::End,
            Some(_) => {
                n |= ResNote::Accrue.bit();
                AccrueAt::Prorated
            }
        };
        n |= rate_notes(&r.rates, digits);
        if r.availability > 0 {
            n |= ResNote::Availability.bit();
        }
        if r.details {
            n |= ResNote::Details.bit();
        }
        if r.budget {
            n |= ResNote::Budget.bit();
        }
        if kind == ResourceType::Work
            && let Some(uid) = r.calendar.filter(|c| *c >= 0)
        {
            match calendars.get(&uid) {
                None => n |= ResNote::CalendarMissing.bit(),
                Some(built) => {
                    let mut cal = built.calendar.clone();
                    if exception_days.saturating_add(cal.exceptions.len()) > MAX_RESOURCE_EXCEPTION_DAYS {
                        cal.exceptions.clear();
                        n |= ResNote::CalendarFull.bit();
                    }
                    exception_days = exception_days.saturating_add(cal.exceptions.len());
                    if built.other_base {
                        n |= ResNote::CalendarBase.bit();
                    }
                    if !cal.is_empty() && !cal.works_a_weekday(&p.calendar) {
                        n |= ResNote::NoWorkingWeekday.bit();
                    }
                    for l in &built.lines {
                        lines.push(line(l.outcome, &l.feature, &format!("{}: {}", res.name, l.place), &l.why));
                    }
                    res.calendar = cal;
                }
            }
        }
        if let Some(slot) = notes.get_mut(i) {
            *slot = n;
        }
        out.push(res);
    }
    let resource_names: Vec<String> = out.iter().map(|r| r.name.clone()).collect();
    for note in ResNote::ALL {
        let rows: Vec<String> =
            notes.iter().enumerate().filter(|(_, b)| *b & note.bit() != 0).filter_map(|(i, _)| resource_names.get(i).cloned()).collect();
        if rows.is_empty() {
            continue;
        }
        let (outcome, feature, why) = note.line();
        let total = rows.len();
        lines.push(line(outcome, feature, &named_list(rows, total, "more resource"), why));
    }
    p.next_resource_id = ids.iter().map(|r| r.0).max().unwrap_or(0).saturating_add(1);
    p.resources = out;
    lines.extend(assign(p, assignments, assignment_overflow, task_uids, &by_uid, &ids));
    if let Err(e) = p.check_resources() {
        // Only a bug could get here: everything above is within the model's checks.
        p.resources.clear();
        p.assignments.clear();
        lines.push(line(Outcome::Dropped, "Resources", "All resources", &format!("they couldn't be imported: {e}")));
    }
    lines
}

/// Notes about a resource's rate tables: B to E with any amount, and table A changing over time.
fn rate_notes(rates: &[FileRate], digits: u32) -> u32 {
    let amount = |t: &Option<String>| t.as_deref().and_then(|t| number::scaled(t, digits)).map_or(0, |(v, _)| v);
    let mut n = 0u32;
    let mut a_sets: BTreeSet<(i128, i128, i128)> = BTreeSet::new();
    for r in rates {
        let set = (amount(&r.standard), amount(&r.overtime), amount(&r.per_use));
        match r.table.unwrap_or(0) {
            0 => {
                a_sets.insert(set);
            }
            t @ 1..=4 if set != (0, 0, 0) => {
                n |= [ResNote::TableB, ResNote::TableC, ResNote::TableD, ResNote::TableE].get((t - 1) as usize).map_or(0, |x| x.bit());
            }
            _ => {}
        }
    }
    if a_sets.len() > 1 {
        n |= ResNote::RatesOverTime.bit();
    }
    n
}

/// The file's assignments as the project's, with the lines about them.
fn assign(
    p: &mut Project,
    file: Vec<FileAssignment>,
    overflow: usize,
    task_uids: &[Option<i64>],
    by_uid: &HashMap<i64, usize>,
    ids: &[ResourceId],
) -> Vec<ReportLine> {
    let mut lines = Vec::new();
    let mut task_row: HashMap<i64, usize> = HashMap::new();
    for (row, uid) in task_uids.iter().enumerate() {
        if let Some(u) = uid {
            task_row.entry(*u).or_insert(row);
        }
    }
    let outline = Outline::of(p);
    let mut left_out = [0usize; LEFT_OUT.len()];
    left_out[5] = overflow;
    let mut pairs: HashSet<(TaskId, ResourceId)> = HashSet::new();
    let mut per_task: HashMap<TaskId, usize> = HashMap::new();
    let mut made: Vec<Assignment> = Vec::new();
    let mut notes: Vec<u32> = Vec::new();
    let mut labels: Vec<(usize, usize)> = Vec::new();
    for a in file {
        let Some(row) = a.task.and_then(|t| task_row.get(&t).copied()) else {
            let i = if a.task == Some(0) { 0 } else { 1 };
            left_out[i] += 1;
            continue;
        };
        let Some(res_index) = a.resource.and_then(|r| by_uid.get(&r).copied()) else {
            left_out[2] += 1;
            continue;
        };
        let (Some(task), Some(&rid), Some(res)) = (p.tasks.get(row), ids.get(res_index), p.resources.get(res_index)) else {
            left_out[2] += 1;
            continue;
        };
        if !pairs.insert((task.id, rid)) {
            left_out[3] += 1;
            continue;
        }
        let count = per_task.entry(task.id).or_insert(0);
        if *count >= MAX_TASK_ASSIGNMENTS {
            left_out[4] += 1;
            continue;
        }
        *count += 1;
        let mut n = 0u32;
        let mut note = |x: AsgNote| n |= x.bit();
        let duration = task.duration.clamp(0, MAX_MINUTES);
        let (units, work, length) = match res.kind {
            ResourceType::Work => {
                let units = match a.units.as_deref() {
                    None => match default_units(ResourceType::Work, res.max_units) {
                        Units::Work(u) => u,
                        _ => FULL_TIME,
                    },
                    Some(text) => {
                        let (u, read) = number::within(text, 4, i64::from(MAX_UNITS), i64::from(FULL_TIME));
                        if read == Read::OutOfRange {
                            note(AsgNote::Units);
                        }
                        u32::try_from(u).unwrap_or(FULL_TIME)
                    }
                };
                let from_duration = ratio(duration, i64::from(units), i64::from(FULL_TIME)).clamp(0, MAX_WORK);
                let work = match a.work {
                    None => from_duration,
                    Some(None) => {
                        note(AsgNote::Work);
                        from_duration
                    }
                    Some(Some(m)) if !(0..=MAX_WORK).contains(&m) => {
                        note(AsgNote::Work);
                        m.clamp(0, MAX_WORK)
                    }
                    Some(Some(m)) => m,
                };
                let length = if outline.is_summary(row) || units == 0 {
                    duration
                } else {
                    let l = ratio(work, i64::from(FULL_TIME), i64::from(units));
                    if l > MAX_MINUTES {
                        note(AsgNote::Work);
                    }
                    l.clamp(0, MAX_MINUTES)
                };
                // The work that fits in that length at these units, when it was cut.
                let work = if length == MAX_MINUTES && units > 0 { work.min(ratio(length, i64::from(units), i64::from(FULL_TIME))) } else { work };
                (Units::Work(units), work, length)
            }
            ResourceType::Material => {
                let q = match a.units.as_deref() {
                    None => 100,
                    Some(text) => {
                        let (q, read) = number::within(text, 2, MAX_QUANTITY, 100);
                        match read {
                            Read::OutOfRange => note(AsgNote::Units),
                            Read::Rounded | Read::Exact => {}
                        }
                        q
                    }
                };
                if a.fixed_material == Some(false) {
                    note(AsgNote::VariableMaterial);
                }
                (Units::Fixed(q), 0, 0)
            }
            ResourceType::Cost => {
                let c = match a.cost.as_deref() {
                    None => 0,
                    Some(text) => {
                        let (c, read) = number::within(text, p.currency.digits, MAX_MONEY, 0);
                        match read {
                            Read::OutOfRange => note(AsgNote::Cost),
                            Read::Rounded => note(AsgNote::Rounded),
                            Read::Exact => {}
                        }
                        c
                    }
                };
                (Units::Cost(c), 0, 0)
            }
        };
        if res.kind == ResourceType::Work
            && let Some(c) = a.contour.filter(|c| (1..=8).contains(c))
        {
            note(AsgNote::Contour(usize::try_from(c).unwrap_or(0)));
        }
        if a.table.is_some_and(|t| t != 0) {
            note(AsgNote::CostTable);
        }
        for (flag, x) in
            [(a.overtime, AsgNote::Overtime), (a.delay, AsgNote::Delay), (a.progress, AsgNote::Progress), (a.timephased, AsgNote::Timephased)]
        {
            if flag {
                note(x);
            }
        }
        made.push(Assignment { task: task.id, resource: rid, units, work, duration: length });
        notes.push(n);
        labels.push((res_index, row));
    }
    for (i, why) in LEFT_OUT.iter().enumerate() {
        let count = left_out.get(i).copied().unwrap_or(0);
        if count > 0 {
            lines.push(line(Outcome::Dropped, "Assignments", &plural(count, "assignment"), why));
        }
    }
    let label = |(res, row): (usize, usize)| {
        let who = p.resources.get(res).map_or("?", |r| r.name.as_str());
        let task = p.tasks.get(row).map_or_else(String::new, |t| format!("ID {} {}", row + 1, t.name.chars().take(40).collect::<String>()));
        format!("{who} on {task}")
    };
    for note in AsgNote::ALL {
        let with: Vec<usize> = notes.iter().enumerate().filter(|(_, b)| *b & note.bit() != 0).map(|(i, _)| i).collect();
        if with.is_empty() {
            continue;
        }
        let shown: Vec<String> = with.iter().take(super::NAMED_IN_LINE).filter_map(|i| labels.get(*i).copied()).map(label).collect();
        let (outcome, feature, why) = note.line();
        lines.push(line(outcome, &feature, &named_list(shown, with.len(), "more assignment"), why));
    }
    // A task's duration is its longest work assignment's, as every edit keeps it.
    let before: Vec<i64> = p.tasks.iter().map(|t| t.duration).collect();
    p.assignments = made;
    p.fill_assignment_work();
    let changed: Vec<usize> = p.tasks.iter().zip(&before).enumerate().filter(|(_, (t, b))| t.duration != **b).map(|(i, _)| i).collect();
    if !changed.is_empty() {
        lines.push(line(
            Outcome::Converted,
            "Duration from assignments",
            &super::rows_named(&p.tasks, &changed),
            "ProjectCraft holds a task's duration as its longest assignment's (work ÷ units, worked on the resource's calendar); worked out from the file's work and units",
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_note_has_its_own_bit_and_line() {
        let bits: Vec<u32> = ResNote::ALL.iter().map(|n| n.bit()).collect();
        assert!(bits.iter().enumerate().all(|(i, b)| *b == 1 << i));
        let bits: Vec<u32> = AsgNote::ALL.iter().map(|n| n.bit()).collect();
        assert!(bits.iter().enumerate().all(|(i, b)| *b == 1 << i));
        assert!(AsgNote::ALL.iter().all(|n| !n.line().2.is_empty()));
    }

    #[test]
    fn work_and_duration_round_as_the_model_does() {
        assert_eq!(ratio(450, 10_000, 3_333), 1350);
        assert_eq!(ratio(1, 1, 2), 1, "a half away from zero");
        assert_eq!(ratio(i64::MAX, i64::MAX, 1), i64::MAX);
        assert_eq!(ratio(5, 1, 0), 0);
    }
}
