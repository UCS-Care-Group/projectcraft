//! A task's scheduling fields from Microsoft Project XML: its constraint
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/constrainttype-element>) with
//! its date, its deadline, its task type
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/type-element-multiple-parents>)
//! whether it is effort-driven and its fixed cost (a decimal amount of the project currency, read
//! exactly into minor units); and the project's defaults for new tasks
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/defaulttasktype-element>).
//!
//! `ConstraintType` codes: 0 As Soon As Possible, 1 As Late As Possible, 2 Must Start On, 3 Must
//! Finish On, 4 Start No Earlier Than, 5 Start No Later Than, 6 Finish No Earlier Than, 7 Finish No
//! Later Than; every type but 0 and 1 needs a `ConstraintDate`. Task `Type` codes: 0 Fixed Units,
//! 1 Fixed Duration, 2 Fixed Work. Dates are `xs:dateTime`; ProjectCraft holds constraints and
//! deadlines to the day, so the day is kept and the time of day left (the scheduler starts a
//! start constraint at the day's first working minute and ends a finish constraint at its last).
//! A constraint whose time of day would move the task (a start constraint after the day's first
//! working minute, a finish constraint before its last, on the project calendar) is a report line;
//! midnight is read as the day alone.
//!
//! Every file is hostile: an unknown code, a missing or unreadable date, and a date further than
//! [`MAX_DAYS`] days from the project start are each a report line, never an error.

use crate::model::{Calendar, Constraint, Date, MAX_DAYS, MAX_MONEY, Options, Task, TaskType};
use crate::{Outcome, ReportLine};

use super::number::{self, Read};

use super::{date_of, int, line, rows_named, truthy};

/// The fields as the file writes them, read when the task closes.
#[derive(Debug, Default)]
pub(super) struct TaskFields {
    constraint_type: Option<String>,
    constraint_date: Option<String>,
    deadline: Option<String>,
    task_type: Option<String>,
    effort_driven: Option<bool>,
    stop: Option<String>,
    resume: Option<String>,
    fixed_cost: Option<String>,
    fixed_cost_accrual: Option<String>,
}

/// Longest text kept of a field; a date or a code is far shorter.
const MAX_FIELD: usize = 64;

fn kept(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_FIELD).collect())
}

impl TaskFields {
    /// Takes `name`'s text when it is one of these fields; whether it was.
    pub(super) fn read(&mut self, name: &str, text: &str) -> bool {
        match name {
            "ConstraintType" => self.constraint_type = kept(text),
            "ConstraintDate" => self.constraint_date = kept(text),
            "Deadline" => self.deadline = kept(text),
            "Type" => self.task_type = kept(text),
            "EffortDriven" => self.effort_driven = Some(truthy(text)),
            "Stop" => self.stop = kept(text),
            "Resume" => self.resume = kept(text),
            "FixedCost" => self.fixed_cost = kept(text),
            "FixedCostAccrual" => self.fixed_cost_accrual = kept(text),
            _ => return false,
        }
        true
    }
}

/// Something about these fields that the report mentions. The order is the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Note {
    Alap,
    UnknownConstraint,
    NoConstraintDate,
    FarConstraintDate,
    ConstraintTime,
    BadDeadline,
    FarDeadline,
    UnknownTaskType,
    FixedWorkEffortDriven,
    Split,
    FixedCost,
    FixedCostRounded,
    FixedCostAccrual,
}

impl Note {
    const ALL: [Note; 13] = [
        Note::Alap,
        Note::UnknownConstraint,
        Note::NoConstraintDate,
        Note::FarConstraintDate,
        Note::ConstraintTime,
        Note::BadDeadline,
        Note::FarDeadline,
        Note::UnknownTaskType,
        Note::FixedWorkEffortDriven,
        Note::Split,
        Note::FixedCost,
        Note::FixedCostRounded,
        Note::FixedCostAccrual,
    ];

    fn bit(self) -> u32 {
        1 << (self as u8)
    }

    /// Outcome, feature and why.
    fn line(self) -> (Outcome, &'static str, &'static str) {
        use Outcome::*;
        match self {
            Note::Alap => (Converted, "As Late As Possible", "not supported yet; As Soon As Possible used"),
            Note::UnknownConstraint => {
                (Converted, "Constraint", "the constraint type code isn't one of Project's (0 to 7); As Soon As Possible used")
            }
            Note::NoConstraintDate => {
                (Converted, "Constraint date", "the constraint needs a date and the file gives none that can be read; As Soon As Possible used")
            }
            Note::FarConstraintDate => (Converted, "Constraint date", "more than 100,000 days from the project start; moved to 100,000 days from it"),
            Note::ConstraintTime => (
                Converted,
                "Constraint time of day",
                "ProjectCraft holds a constraint to the day: a start constraint starts at the day's first working minute and a finish constraint ends at its last, so the file's time of day is left and the task's dates may differ from the file's",
            ),
            Note::BadDeadline => (Dropped, "Deadline", "unreadable; the task has no deadline"),
            Note::FarDeadline => (Converted, "Deadline", "more than 100,000 days from the project start; moved to 100,000 days from it"),
            Note::UnknownTaskType => (Converted, "Task type", "the task type code isn't one of Project's (0 to 2); Fixed Units used"),
            Note::FixedWorkEffortDriven => (Converted, "Effort-driven", "a Fixed Work task is always effort-driven; made effort-driven"),
            Note::Split => {
                (Dropped, "Split (Stop, Resume)", "work stops on one date and resumes on a later one; ProjectCraft schedules each task in one piece")
            }
            Note::FixedCost => {
                (Converted, "Fixed cost", "negative, unreadable or larger than ProjectCraft holds; 0 or the largest amount (ten billion) used")
            }
            Note::FixedCostRounded => {
                (Converted, "Amounts finer than the currency", "rounded half to even to the currency's smallest unit (the penny for pounds)")
            }
            Note::FixedCostAccrual => (
                Dropped,
                "Fixed cost accrual",
                "accrued at the start or end; ProjectCraft holds when costs accrue for tracking, which comes later, and keeps the fixed cost",
            ),
        }
    }
}

/// A date of the file within [`MAX_DAYS`] days of the project start, and whether it was moved there.
fn near(d: Date, start: Date) -> (Date, bool) {
    let away = d.days_since(start);
    if away.abs() <= MAX_DAYS {
        return (d, false);
    }
    (start.add_days(MAX_DAYS * away.signum()).unwrap_or(d), true)
}

/// The time of day of an `xs:dateTime` (`2026-11-02T13:30:00`), in minutes from midnight.
fn time_of(text: &str) -> Option<u16> {
    let t = text.trim().get(10..)?.strip_prefix('T')?;
    let hours: u16 = t.get(..2)?.parse().ok()?;
    let minutes: u16 = t.get(3..5)?.parse().ok()?;
    (t.get(2..3) == Some(":") && hours <= 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// Whether a constraint's time of day in the file would put the task elsewhere than ProjectCraft
/// does: a start constraint (`start` true) after the day's first working minute, a finish
/// constraint before its last. Midnight, a date without a time, and a day off are the day alone.
fn time_moves(text: &str, d: Date, start: bool, calendar: &Calendar) -> bool {
    let times = calendar.times_on(d);
    match (time_of(text), times.first(), times.last()) {
        (Some(t), Some(first), Some(last)) if t != 0 => {
            if start {
                t > first.from
            } else {
                t < last.to
            }
        }
        _ => false,
    }
}

/// Sets the task's constraint, deadline, type, effort-driven flag and fixed cost (in minor units
/// of a currency with `digits` decimal places) from the file's fields; the report notes about
/// them, as bits. `calendar` is the project calendar, to tell whether a constraint's time of day
/// matters.
pub(super) fn apply(f: &TaskFields, t: &mut Task, start: Date, digits: u32, calendar: &Calendar) -> u32 {
    let mut notes = 0u32;
    let mut note = |n: Note| notes |= n.bit();
    t.constraint = match f.constraint_type.as_deref().map(int) {
        None | Some(Some(0)) => Constraint::Asap,
        Some(Some(1)) => {
            note(Note::Alap);
            Constraint::Asap
        }
        Some(Some(code @ 2..=7)) => match f.constraint_date.as_deref().and_then(date_of) {
            None => {
                note(Note::NoConstraintDate);
                Constraint::Asap
            }
            Some(d) => {
                let (d, moved) = near(d, start);
                if moved {
                    note(Note::FarConstraintDate);
                } else if f.constraint_date.as_deref().is_some_and(|text| time_moves(text, d, matches!(code, 2 | 4 | 5), calendar)) {
                    note(Note::ConstraintTime);
                }
                match code {
                    2 => Constraint::MustStartOn(d),
                    3 => Constraint::MustFinishOn(d),
                    4 => Constraint::StartNoEarlierThan(d),
                    5 => Constraint::StartNoLaterThan(d),
                    6 => Constraint::FinishNoEarlierThan(d),
                    _ => Constraint::FinishNoLaterThan(d),
                }
            }
        },
        Some(_) => {
            note(Note::UnknownConstraint);
            Constraint::Asap
        }
    };
    t.deadline = match f.deadline.as_deref() {
        None => None,
        Some(text) => match date_of(text) {
            None => {
                note(Note::BadDeadline);
                None
            }
            Some(d) => {
                let (d, moved) = near(d, start);
                if moved {
                    note(Note::FarDeadline);
                }
                Some(d)
            }
        },
    };
    t.task_type = match f.task_type.as_deref().map(int) {
        None | Some(Some(0)) => TaskType::FixedUnits,
        Some(Some(1)) => TaskType::FixedDuration,
        Some(Some(2)) => TaskType::FixedWork,
        Some(_) => {
            note(Note::UnknownTaskType);
            TaskType::FixedUnits
        }
    };
    t.effort_driven = f.effort_driven.unwrap_or(false);
    if t.task_type == TaskType::FixedWork && !t.effort_driven {
        if f.effort_driven.is_some() {
            note(Note::FixedWorkEffortDriven);
        }
        t.effort_driven = true;
    }
    if let (Some(stop), Some(resume)) = (f.stop.as_deref(), f.resume.as_deref())
        && let (Some(s), Some(r)) = (date_of(stop), date_of(resume))
        && r > s
    {
        note(Note::Split);
    }
    if let Some(text) = f.fixed_cost.as_deref() {
        let (minor, read) = number::within(text, digits, MAX_MONEY, 0);
        match read {
            Read::Exact => {}
            Read::Rounded => note(Note::FixedCostRounded),
            Read::OutOfRange => note(Note::FixedCost),
        }
        t.fixed_cost = minor;
    }
    // 1 start, 2 end, 3 prorated (Project's default); only of a task with a fixed cost.
    let has_cost = f.fixed_cost.as_deref().and_then(|t| number::scaled(t, digits)).is_some_and(|(v, _)| v != 0);
    if has_cost && f.fixed_cost_accrual.as_deref().map(int).is_some_and(|a| matches!(a, Some(1 | 2))) {
        note(Note::FixedCostAccrual);
    }
    notes
}

/// One line per note, naming the first few tasks that have it.
pub(super) fn lines(tasks: &[Task], notes: &[u32]) -> Vec<ReportLine> {
    let mut out = Vec::new();
    for n in Note::ALL {
        let rows: Vec<usize> = notes.iter().enumerate().filter(|(_, bits)| *bits & n.bit() != 0).map(|(i, _)| i).collect();
        if rows.is_empty() {
            continue;
        }
        let (outcome, feature, why) = n.line();
        out.push(line(outcome, feature, &rows_named(tasks, &rows), why));
    }
    out
}

/// The project's `DefaultTaskType` and `NewTasksEffortDriven`, as the file writes them.
#[derive(Debug, Default)]
pub(super) struct ProjectDefaults {
    task_type: Option<String>,
    effort_driven: Option<bool>,
}

impl ProjectDefaults {
    /// Takes `name`'s text when it is one of these fields; whether it was.
    pub(super) fn read(&mut self, name: &str, text: &str) -> bool {
        match name {
            "DefaultTaskType" => self.task_type = kept(text),
            "NewTasksEffortDriven" => self.effort_driven = Some(truthy(text)),
            _ => return false,
        }
        true
    }

    /// Sets the type new tasks get and whether they are effort-driven.
    pub(super) fn apply(&self, o: &mut Options, lines: &mut Vec<ReportLine>) {
        o.task_type = match self.task_type.as_deref().map(int) {
            None | Some(Some(0)) => TaskType::FixedUnits,
            Some(Some(1)) => TaskType::FixedDuration,
            Some(Some(2)) => TaskType::FixedWork,
            Some(_) => {
                lines.push(line(
                    Outcome::Converted,
                    "Default task type",
                    "Project",
                    "the task type code isn't one of Project's (0 to 2); Fixed Units used",
                ));
                TaskType::FixedUnits
            }
        };
        o.effort_driven = self.effort_driven.unwrap_or(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        Date::from_ymd(y, m, day).unwrap()
    }

    #[test]
    fn dates_further_than_the_model_s_longest_span_are_moved_in() {
        let start = d(2026, 3, 2);
        assert_eq!(near(d(2030, 1, 1), start), (d(2030, 1, 1), false));
        assert!(!near(start.add_days(MAX_DAYS).unwrap(), start).1);
        assert_eq!(near(d(9999, 12, 31), start), (start.add_days(MAX_DAYS).unwrap(), true));
        assert_eq!(near(d(1, 1, 1), start), (start.add_days(-MAX_DAYS).unwrap(), true));
        // A start near the end of time can't move a date further out than year 9999.
        let late = d(9999, 1, 1);
        assert_eq!(near(d(1, 1, 1), late), (late.add_days(-MAX_DAYS).unwrap(), true));
    }

    #[test]
    fn a_constraint_s_time_of_day_is_noted_only_when_it_would_move_the_task() {
        let c = Calendar::default();
        let monday = d(2026, 10, 12);
        assert_eq!(time_of("2026-10-12T13:30:00"), Some(13 * 60 + 30));
        assert_eq!((time_of("2026-10-12"), time_of("2026-10-12T1"), time_of("2026-10-12T99:00:00")), (None, None, None));
        // Start constraints: after 08:00 moves the task; at or before it, or midnight, doesn't.
        assert!(time_moves("2026-10-12T13:00:00", monday, true, &c));
        assert!(time_moves("2026-10-12T17:00:00", monday, true, &c), "at the day's end Project starts the next day");
        assert!(!time_moves("2026-10-12T08:00:00", monday, true, &c));
        assert!(!time_moves("2026-10-12T07:00:00", monday, true, &c));
        assert!(!time_moves("2026-10-12T00:00:00", monday, true, &c));
        assert!(!time_moves("2026-10-12", monday, true, &c));
        // Finish constraints: before 17:00 moves the task; at or after it, or midnight, doesn't.
        assert!(time_moves("2026-10-12T12:00:00", monday, false, &c));
        assert!(!time_moves("2026-10-12T17:00:00", monday, false, &c));
        assert!(!time_moves("2026-10-12T23:59:00", monday, false, &c));
        assert!(!time_moves("2026-10-12T00:00:00", monday, false, &c));
        // A day off has no working minutes to be after.
        assert!(!time_moves("2026-10-11T13:00:00", d(2026, 10, 11), true, &c));
    }

    #[test]
    fn every_note_has_its_own_bit_and_line() {
        let bits: Vec<u32> = Note::ALL.iter().map(|n| n.bit()).collect();
        assert!(bits.iter().enumerate().all(|(i, b)| *b == 1 << i));
        assert!(Note::ALL.iter().all(|n| !n.line().2.is_empty()));
    }
}
