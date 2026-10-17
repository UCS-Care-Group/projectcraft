//! Microsoft Project XML (MSPDI) export, by Microsoft's published schema documentation
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/project-element>), in the order
//! its schema (`mspdi_pj12.xsd`) gives each element's children.
//!
//! Everything the project holds is written, in exactly the conventions the importer reads
//! ([`crate::mspdi`]), so a file exported and imported again is the same project and exports
//! byte for byte the same:
//!
//! - The project's name (`Title`), start, hours per day and week, days per month, default task
//!   type, whether new tasks are effort-driven, and its currency (`CurrencyCode`,
//!   `CurrencySymbol`, `CurrencyDigits`).
//! - The project calendar (UID 1, `Standard`): its seven weekdays (`DayType` 1 Sunday … 7
//!   Saturday) with their working times, and its exceptions, a run of days with the same times
//!   to an `Exception` (daily, `Type` 1). A `ToTime` of midnight is the end of the day.
//! - Each task with its Unique ID as `UID` and its row as `ID`: name, outline level, duration
//!   (an `xs:duration` of working time, `PT16H0M0S`) in its unit (`DurationFormat` 3 m, 5 h,
//!   7 d, 9 w, 11 mo), task type, effort-driven, fixed cost (a decimal amount of the currency),
//!   constraint and its date, deadline, priority (0 to 1,000), leveling delay (`LevelingDelay`
//!   in tenths of a minute, as MSPDI's other spans and Project Server's `ASSN_LEVELING_DELAY`,
//!   shown in days), and its links (`PredecessorLink`: `Type` 0 FF, 1 FS,
//!   2 SF, 3 SS; `LinkLag` in tenths of a minute; `LagFormat` the lag's unit). ProjectCraft's own
//!   computed Start, Finish, total slack (tenths of a minute), critical, summary, milestone and
//!   cost are written too, for the program that reads the file; the importer recomputes them.
//! - Each resource with its Unique ID and row: name, type (`Type` 0 material, 1 work; a cost
//!   resource is `IsCostResource`), initials, group, material label, Max Units (`1` is 100%),
//!   rates per hour (per unit of a material's label) and cost per use as decimal amounts, Accrue
//!   At (1 start, 2 end, 3 prorated), and a work resource's own calendar (based on the project
//!   calendar; only the weekdays that differ from it, and its own exceptions).
//! - Each assignment: units (`0.5` is 50%; a material's quantity of its label), work, and a cost
//!   resource's amount as its `Cost`.
//!
//! A material used at a rate (`2 tons/d`) has no field in the published schema: it is written as
//! the quantity it uses over its task, which is how it imports. The task sheet's columns and the
//! assignment units format are views, not part of the plan, and are not written.
//!
//! Nothing comes from the clock: the same project and schedule give the same bytes.

use std::fmt::Write as _;

use crate::model::{
    AccrueAt, Constraint, DurationUnit, LinkKind, MINUTES_IN_DAY, Moment, Outline, Project, Resource, ResourceCalendar, ResourceType, Schedule,
    TaskType, Units, WorkingTime, assignment_work,
};
use crate::mspdi::NAMESPACE;

/// The schema version written: Project 2007's, the one Microsoft documents.
const SAVE_VERSION: u32 = 12;
/// The project calendar's Unique ID; resource calendars follow it.
const PROJECT_CALENDAR: u32 = 1;
/// What the importer names a project whose file gives no title.
const UNNAMED: &str = "Imported project";
/// A day's start and end when it has no working time to take them from: 8:00 and 17:00.
const DAY_START: u16 = 8 * 60;
const DAY_END: u16 = 17 * 60;

/// The project as Microsoft Project XML, with `schedule`'s dates when there is one.
pub(crate) fn export(p: &Project, schedule: Option<&Schedule>) -> String {
    let mut w = Writer::default();
    w.out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    let _ = writeln!(w.out, "<Project xmlns=\"{NAMESPACE}\">");
    w.depth = 1;
    project_fields(&mut w, p, schedule);
    calendars(&mut w, p);
    tasks(&mut w, p, schedule);
    resources(&mut w, p);
    assignments(&mut w, p);
    w.out.push_str("</Project>\n");
    w.out
}

/// Builds the file a line an element, two spaces an indent.
#[derive(Default)]
struct Writer {
    out: String,
    depth: usize,
}

impl Writer {
    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }

    fn open(&mut self, name: &str) {
        self.indent();
        let _ = writeln!(self.out, "<{name}>");
        self.depth += 1;
    }

    fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.indent();
        let _ = writeln!(self.out, "</{name}>");
    }

    /// An element holding `value`, escaped.
    fn field(&mut self, name: &str, value: impl std::fmt::Display) {
        self.indent();
        let _ = write!(self.out, "<{name}>");
        let _ = write!(Escaped(&mut self.out), "{value}");
        let _ = writeln!(self.out, "</{name}>");
    }

    fn flag(&mut self, name: &str, on: bool) {
        self.field(name, u8::from(on));
    }
}

/// Escapes what is written through it as XML text: `&`, `<`, `>`, quotes, and a carriage return
/// (which a reader would otherwise fold into a line feed); characters XML 1.0 can't hold at all
/// (most control characters, U+FFFE and U+FFFF) are left out.
struct Escaped<'a>(&'a mut String);

impl std::fmt::Write for Escaped<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for c in s.chars() {
            match c {
                '&' => self.0.push_str("&amp;"),
                '<' => self.0.push_str("&lt;"),
                '>' => self.0.push_str("&gt;"),
                '"' => self.0.push_str("&quot;"),
                '\'' => self.0.push_str("&apos;"),
                '\r' => self.0.push_str("&#13;"),
                '\t' | '\n' => self.0.push(c),
                c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
                c => self.0.push(c),
            }
        }
        Ok(())
    }
}

/// Text as the importer reads it back: trimmed, without characters XML can't hold.
pub(crate) fn text(s: &str) -> String {
    s.chars()
        .filter(|c| !((*c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')) && *c != '\u{FFFE}' && *c != '\u{FFFF}')
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `2026-11-02T08:00:00`; minute 1440 is the end of the day, `T24:00:00`.
fn date_time(m: Moment) -> String {
    format!("{}T{:02}:{:02}:00", m.date, m.minute / 60, m.minute % 60)
}

/// `08:00:00`; the end of the day is midnight, `00:00:00`, as Project writes it.
fn clock(minute: u16) -> String {
    let m = if minute >= MINUTES_IN_DAY { 0 } else { minute };
    format!("{:02}:{:02}:00", m / 60, m % 60)
}

/// Working minutes as an `xs:duration` in hours and minutes: `PT16H0M0S`, `-PT1H30M0S`.
fn duration(minutes: i64) -> String {
    let sign = if minutes < 0 { "-" } else { "" };
    let m = minutes.unsigned_abs();
    format!("{sign}PT{}H{}M0S", m / 60, m % 60)
}

/// `value` in units of `10^-places` as a decimal: `4237` at 2 places is `42.37`. Trailing zeros
/// of the fraction are kept when `keep`, else dropped with the point (`1.5`, `1`).
fn decimal(value: i64, places: u32, keep: bool) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let v = value.unsigned_abs();
    let scale = 10u64.checked_pow(places).unwrap_or(1);
    let (whole, frac) = (v / scale, v % scale);
    if places == 0 || scale == 1 {
        return format!("{sign}{v}");
    }
    let mut f = format!("{frac:0width$}", width = places as usize);
    if !keep {
        f = f.trim_end_matches('0').to_owned();
    }
    if f.is_empty() { format!("{sign}{whole}") } else { format!("{sign}{whole}.{f}") }
}

/// The `DurationFormat` and `LagFormat` code of a working-time unit.
fn format_code(unit: DurationUnit) -> u8 {
    match unit {
        DurationUnit::Minutes => 3,
        DurationUnit::Hours => 5,
        DurationUnit::Days => 7,
        DurationUnit::Weeks => 9,
        DurationUnit::Months => 11,
    }
}

fn task_type_code(t: TaskType) -> u8 {
    match t {
        TaskType::FixedUnits => 0,
        TaskType::FixedDuration => 1,
        TaskType::FixedWork => 2,
    }
}

/// The first and last working minute of a day of the project calendar (8:00 and 17:00 on a day
/// off), to give a constraint or deadline the time of day Project gives it.
fn day_bounds(p: &Project, d: crate::model::Date) -> (u16, u16) {
    let times = p.calendar.times_on(d);
    (times.first().map_or(DAY_START, |t| t.from), times.last().map_or(DAY_END, |t| t.to))
}

fn project_fields(w: &mut Writer, p: &Project, schedule: Option<&Schedule>) {
    w.field("SaveVersion", SAVE_VERSION);
    let name = text(&p.name);
    w.field("Title", if name.is_empty() { UNNAMED } else { name.as_str() });
    w.flag("ScheduleFromStart", true);
    let (from, _) = day_bounds(p, p.start);
    w.field("StartDate", date_time(Moment { date: p.start, minute: from }));
    if let Some(s) = schedule {
        let (_, to) = day_bounds(p, s.finish);
        w.field("FinishDate", date_time(Moment { date: s.finish, minute: to }));
    }
    w.field("CurrencyDigits", p.currency.digits);
    w.field("CurrencySymbol", &p.currency.symbol);
    w.field("CurrencyCode", &p.currency.code);
    w.field("CalendarUID", PROJECT_CALENDAR);
    w.field("MinutesPerDay", p.options.minutes_per_day);
    w.field("MinutesPerWeek", p.options.minutes_per_week);
    w.field("DaysPerMonth", p.options.days_per_month);
    w.field("DefaultTaskType", task_type_code(p.options.task_type));
    w.flag("NewTasksEffortDriven", p.options.effort_driven);
}

/// MSPDI's `DayType` of weekday `i` (0 Monday): 1 Sunday … 7 Saturday.
fn day_type(i: usize) -> usize {
    (i + 1) % 7 + 1
}

fn working_times(w: &mut Writer, times: &[WorkingTime]) {
    if times.is_empty() {
        return;
    }
    w.open("WorkingTimes");
    for t in times {
        w.open("WorkingTime");
        w.field("FromTime", clock(t.from));
        w.field("ToTime", clock(t.to));
        w.close("WorkingTime");
    }
    w.close("WorkingTimes");
}

fn weekday(w: &mut Writer, i: usize, times: &[WorkingTime]) {
    w.open("WeekDay");
    w.field("DayType", day_type(i));
    w.flag("DayWorking", !times.is_empty());
    working_times(w, times);
    w.close("WeekDay");
}

/// Exception days as runs of consecutive days with the same times, each an `Exception` from its
/// first day to its last.
fn exceptions<'a>(w: &mut Writer, days: impl Iterator<Item = (crate::model::Date, &'a [WorkingTime])>) {
    let mut runs: Vec<(crate::model::Date, crate::model::Date, u32, &[WorkingTime])> = Vec::new();
    for (d, times) in days {
        match runs.last_mut() {
            Some((_, to, n, t)) if *t == times && to.add_days(1) == Some(d) => {
                *to = d;
                *n = n.saturating_add(1);
            }
            _ => runs.push((d, d, 1, times)),
        }
    }
    if runs.is_empty() {
        return;
    }
    w.open("Exceptions");
    for (from, to, n, times) in runs {
        w.open("Exception");
        w.flag("EnteredByOccurrences", false);
        w.open("TimePeriod");
        w.field("FromDate", date_time(Moment { date: from, minute: 0 }));
        w.field("ToDate", format!("{to}T23:59:00"));
        w.close("TimePeriod");
        w.field("Occurrences", n);
        // Daily, every day of the period.
        w.field("Type", 1);
        w.flag("DayWorking", !times.is_empty());
        working_times(w, times);
        w.close("Exception");
    }
    w.close("Exceptions");
}

/// The resources with a calendar of their own, each with the UID its calendar gets.
fn resource_calendars(p: &Project) -> Vec<(&Resource, u32)> {
    p.resources
        .iter()
        .filter(|r| r.kind == ResourceType::Work && !own_calendar(p, &r.calendar).is_empty())
        .zip(PROJECT_CALENDAR.saturating_add(1)..)
        .collect()
}

/// A resource calendar as the importer builds it: a weekday with the project's own times follows
/// the project calendar.
fn own_calendar(p: &Project, c: &ResourceCalendar) -> ResourceCalendar {
    let mut c = c.clone();
    for (i, day) in c.week.iter_mut().enumerate() {
        if day.as_deref().is_some_and(|t| t == p.calendar.weekday_times(i)) {
            *day = None;
        }
    }
    c
}

fn calendars(w: &mut Writer, p: &Project) {
    w.open("Calendars");
    w.open("Calendar");
    w.field("UID", PROJECT_CALENDAR);
    w.field("Name", "Standard");
    w.flag("IsBaseCalendar", true);
    w.open("WeekDays");
    // Sunday first, as `DayType` counts.
    for i in [6, 0, 1, 2, 3, 4, 5] {
        weekday(w, i, p.calendar.weekday_times(i));
    }
    w.close("WeekDays");
    exceptions(w, p.calendar.exceptions.keys().map(|d| (*d, p.calendar.times_on(*d))));
    w.close("Calendar");
    for (r, uid) in resource_calendars(p) {
        let c = own_calendar(p, &r.calendar);
        w.open("Calendar");
        w.field("UID", uid);
        w.field("Name", text(&r.name));
        w.flag("IsBaseCalendar", false);
        w.field("BaseCalendarUID", PROJECT_CALENDAR);
        let days: Vec<(usize, &Vec<WorkingTime>)> =
            [6, 0, 1, 2, 3, 4, 5].into_iter().filter_map(|i| c.week.get(i).and_then(Option::as_ref).map(|t| (i, t))).collect();
        if !days.is_empty() {
            w.open("WeekDays");
            for (i, t) in days {
                weekday(w, i, t);
            }
            w.close("WeekDays");
        }
        exceptions(w, c.exceptions.iter().map(|(d, t)| (*d, t.as_slice())));
        w.close("Calendar");
    }
    w.close("Calendars");
}

fn link_code(kind: LinkKind) -> u8 {
    match kind {
        LinkKind::FF => 0,
        LinkKind::FS => 1,
        LinkKind::SF => 2,
        LinkKind::SS => 3,
    }
}

/// `ConstraintType` code and date, with the time of day Project gives it: a start constraint at
/// the day's first working minute, a finish constraint at its last.
fn constraint(p: &Project, c: Constraint) -> (u8, Option<Moment>) {
    let at = |d, start: bool| {
        let (from, to) = day_bounds(p, d);
        Some(Moment { date: d, minute: if start { from } else { to } })
    };
    match c {
        Constraint::Asap => (0, None),
        Constraint::MustStartOn(d) => (2, at(d, true)),
        Constraint::MustFinishOn(d) => (3, at(d, false)),
        Constraint::StartNoEarlierThan(d) => (4, at(d, true)),
        Constraint::StartNoLaterThan(d) => (5, at(d, true)),
        Constraint::FinishNoEarlierThan(d) => (6, at(d, false)),
        Constraint::FinishNoLaterThan(d) => (7, at(d, false)),
    }
}

fn tasks(w: &mut Writer, p: &Project, schedule: Option<&Schedule>) {
    if p.tasks.is_empty() {
        return;
    }
    let outline = Outline::of(p);
    let costs = p.cost_by_task();
    w.open("Tasks");
    for (row, t) in p.tasks.iter().enumerate() {
        let dates = schedule.and_then(|s| s.tasks.get(&t.id));
        let summary = outline.is_summary(row);
        w.open("Task");
        w.field("UID", t.id.0);
        w.field("ID", row.saturating_add(1));
        w.field("Name", text(&t.name));
        w.field("Type", task_type_code(t.task_type));
        w.field("OutlineLevel", outline.level(row));
        w.field("Priority", t.priority);
        if let Some(d) = dates {
            w.field("Start", date_time(d.start_at));
            w.field("Finish", date_time(d.finish_at));
        }
        w.field("Duration", duration(t.duration));
        w.field("DurationFormat", format_code(t.duration_unit));
        w.flag("EffortDriven", t.effort_driven);
        w.flag("Milestone", t.duration == 0 && !summary);
        w.flag("Summary", summary);
        if let Some(d) = dates {
            w.flag("Critical", d.critical);
            w.field("TotalSlack", d.slack.saturating_mul(10));
        }
        w.field("FixedCost", decimal(t.fixed_cost, p.currency.digits, true));
        w.field("Cost", decimal(costs.get(&t.id).copied().unwrap_or(0), p.currency.digits, true));
        let (code, date) = constraint(p, t.constraint);
        w.field("ConstraintType", code);
        if let Some(m) = date {
            w.field("ConstraintDate", date_time(m));
        }
        if let Some(d) = t.deadline {
            let (_, to) = day_bounds(p, d);
            w.field("Deadline", date_time(Moment { date: d, minute: to }));
        }
        // Tenths of a minute, as MSPDI's other spans; shown in days.
        w.field("LevelingDelay", t.leveling_delay.saturating_mul(10));
        w.field("LevelingDelayFormat", format_code(DurationUnit::Days));
        for l in &t.preds {
            w.open("PredecessorLink");
            w.field("PredecessorUID", l.pred.0);
            w.field("Type", link_code(l.kind));
            w.flag("CrossProject", false);
            w.field("LinkLag", l.lag.saturating_mul(10));
            w.field("LagFormat", format_code(l.lag_unit));
            w.close("PredecessorLink");
        }
        w.close("Task");
    }
    w.close("Tasks");
}

fn accrue_code(a: AccrueAt) -> u8 {
    match a {
        AccrueAt::Start => 1,
        AccrueAt::End => 2,
        AccrueAt::Prorated => 3,
    }
}

fn resources(w: &mut Writer, p: &Project) {
    if p.resources.is_empty() {
        return;
    }
    let calendars = resource_calendars(p);
    let digits = p.currency.digits;
    w.open("Resources");
    for (row, r) in p.resources.iter().enumerate() {
        w.open("Resource");
        w.field("UID", r.id.0);
        w.field("ID", row.saturating_add(1));
        w.field("Name", text(&r.name));
        w.field("Type", u8::from(r.kind != ResourceType::Material));
        w.field("Initials", text(&r.initials));
        // Kept whatever the type, as the resource keeps it, so switching back loses nothing.
        w.field("MaterialLabel", text(&r.material_label));
        w.field("Group", text(&r.group));
        w.field("MaxUnits", decimal(i64::from(r.max_units), 4, false));
        w.field("AccrueAt", accrue_code(r.accrue_at));
        w.field("StandardRate", decimal(r.standard_rate, digits, true));
        // Per hour, or per unit of a material's label.
        w.field("StandardRateFormat", if r.kind == ResourceType::Material { 8 } else { 2 });
        w.field("OvertimeRate", decimal(r.overtime_rate, digits, true));
        w.field("OvertimeRateFormat", 2);
        w.field("CostPerUse", decimal(r.cost_per_use, digits, true));
        if let Some((_, uid)) = calendars.iter().find(|(c, _)| c.id == r.id) {
            w.field("CalendarUID", uid);
        }
        w.flag("IsCostResource", r.kind == ResourceType::Cost);
        w.close("Resource");
    }
    w.close("Resources");
}

fn assignments(w: &mut Writer, p: &Project) {
    if p.assignments.is_empty() {
        return;
    }
    let digits = p.currency.digits;
    w.open("Assignments");
    for (n, a) in p.assignments.iter().enumerate() {
        let duration = p.task(a.task).map_or(0, |t| t.duration);
        w.open("Assignment");
        w.field("UID", n.saturating_add(1));
        w.field("TaskUID", a.task.0);
        w.field("ResourceUID", a.resource.0);
        w.field("Cost", decimal(p.assignment_cost(a), digits, true));
        match a.units {
            Units::Work(u) => {
                w.field("Units", decimal(i64::from(u), 4, false));
                w.field("Work", self::duration(a.work.max(0)));
            }
            Units::Fixed(_) | Units::Rate { .. } => {
                let q = match assignment_work(a, duration, &p.options) {
                    crate::model::AssignmentWork::Quantity(q) => q,
                    _ => 0,
                };
                w.field("Units", decimal(q, 2, false));
            }
            Units::Cost(_) => {}
        }
        w.close("Assignment");
    }
    w.close("Assignments");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_exact() {
        assert_eq!(decimal(4237, 2, true), "42.37");
        assert_eq!(decimal(1500, 2, true), "15.00");
        assert_eq!(decimal(5, 2, true), "0.05");
        assert_eq!(decimal(-5, 2, true), "-0.05");
        assert_eq!(decimal(7, 0, true), "7");
        assert_eq!(decimal(10_000, 4, false), "1");
        assert_eq!(decimal(5_000, 4, false), "0.5");
        assert_eq!(decimal(12_345, 4, false), "1.2345");
        assert_eq!(decimal(i64::MIN, 2, true), "-92233720368547758.08");
    }

    #[test]
    fn durations_and_times_are_written_as_the_importer_reads_them() {
        assert_eq!(duration(960), "PT16H0M0S");
        assert_eq!(duration(90), "PT1H30M0S");
        assert_eq!(duration(-90), "-PT1H30M0S");
        for m in [0, 1, 59, 960, 1_000_000_000] {
            assert_eq!(crate::mspdi::duration_minutes(&duration(m)), Some(m));
        }
        assert_eq!(clock(480), "08:00:00");
        assert_eq!(clock(1440), "00:00:00");
        assert_eq!(day_type(0), 2, "Monday");
        assert_eq!(day_type(6), 1, "Sunday");
    }

    #[test]
    fn text_is_escaped_and_cleaned() {
        let mut s = String::new();
        let _ = write!(Escaped(&mut s), "a<b>&\"c'\r\n\t\u{1}\u{FFFF}é");
        assert_eq!(s, "a&lt;b&gt;&amp;&quot;c&apos;&#13;\n\té");
        assert_eq!(text("  Dig \u{7} "), "Dig");
    }
}
