//! Working times (a from–to stretch of a day, in whole minutes since midnight) and the project
//! options that turn hours into days, weeks and months.
//!
//! The scheduler counts working time in minutes on the calendar's working times; the options only
//! convert days, weeks and months to minutes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ModelError, TaskType};

/// Minutes in a day: a working time ends at most here (24:00).
pub const MINUTES_IN_DAY: u16 = 1440;
/// Most working times one day holds. ponytail: a bound for hostile input, not a parity limit.
pub const MAX_WORKING_TIMES: usize = 10;

/// One stretch of working time in a day, `from` up to `to`, in minutes since midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkingTime {
    pub from: u16,
    pub to: u16,
}

/// Microsoft Project's Standard calendar day: 8:00–12:00 and 13:00–17:00.
pub const DEFAULT_WORKING_TIMES: [WorkingTime; 2] = [WorkingTime { from: 8 * 60, to: 12 * 60 }, WorkingTime { from: 13 * 60, to: 17 * 60 }];

impl WorkingTime {
    /// A working time from `from` to `to` (minutes since midnight), refused unless `from < to <= 24:00`.
    pub fn new(from: u16, to: u16) -> Result<WorkingTime, ModelError> {
        let t = WorkingTime { from, to };
        t.check()?;
        Ok(t)
    }

    fn check(self) -> Result<(), ModelError> {
        if self.to > MINUTES_IN_DAY || self.from >= self.to {
            return Err(ModelError::Invalid(format!(
                "a working time runs from a time to a later one within the day (0:00 to 24:00); not {}–{}",
                clock(self.from),
                clock(self.to)
            )));
        }
        Ok(())
    }

    /// Its length in minutes.
    pub fn minutes(self) -> u32 {
        u32::from(self.to.saturating_sub(self.from))
    }
}

impl fmt::Display for WorkingTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", clock(self.from), clock(self.to))
    }
}

/// `minutes` since midnight as `H:MM` (`8:00`, `24:00`).
pub fn clock(minutes: u16) -> String {
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// Reads a time of day: `8:00`, `08:30`, `17` or `24:00`, into minutes since midnight.
pub fn parse_clock(text: &str) -> Result<u16, ModelError> {
    let t = text.trim();
    let bad = || ModelError::Invalid(format!("not a time of day: {:?} (use H:MM, e.g. 8:00 or 17:30)", t.chars().take(20).collect::<String>()));
    let (h, m) = t.split_once(':').unwrap_or((t, "00"));
    let digits = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(h, 2) || !digits(m, 2) || m.len() != 2 {
        return Err(bad());
    }
    let (h, m) = (h.parse::<u16>().map_err(|_| bad())?, m.parse::<u16>().map_err(|_| bad())?);
    let total = h.checked_mul(60).and_then(|h| h.checked_add(m)).ok_or_else(bad)?;
    if m >= 60 || total > MINUTES_IN_DAY {
        return Err(bad());
    }
    Ok(total)
}

/// Reads one working time, `8:00-12:00` (an en dash works too).
pub fn parse_working_time(text: &str) -> Result<WorkingTime, ModelError> {
    let (from, to) = text.split_once(['-', '–']).ok_or_else(|| {
        ModelError::Invalid(format!("not a working time: {:?} (use from-to, e.g. 8:00-12:00)", text.trim().chars().take(30).collect::<String>()))
    })?;
    WorkingTime::new(parse_clock(from)?, parse_clock(to)?)
}

/// Checks a day's working times: each valid, in order, not overlapping, at most [`MAX_WORKING_TIMES`].
pub fn check_working_times(times: &[WorkingTime]) -> Result<(), ModelError> {
    if times.len() > MAX_WORKING_TIMES {
        return Err(ModelError::Invalid(format!("a day holds at most {MAX_WORKING_TIMES} working times")));
    }
    for t in times {
        t.check()?;
    }
    for pair in times.windows(2) {
        if let [a, b] = pair
            && b.from < a.to
        {
            return Err(ModelError::Invalid(format!("working times must be in order and not overlap: {a} then {b}")));
        }
    }
    Ok(())
}

/// Working minutes in a day with these times.
pub fn minutes_of(times: &[WorkingTime]) -> u32 {
    times.iter().map(|t| t.minutes()).sum()
}

/// Project options that convert hours to days, weeks and months for entry and display, as
/// Microsoft Project's Calendar options (they never change the calendar), and what new tasks
/// start as, as its Schedule options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Working minutes in a day (`d`): 480 is 8 hours.
    pub minutes_per_day: u32,
    /// Working minutes in a week (`w`): 2400 is 40 hours.
    pub minutes_per_week: u32,
    /// Working days in a month (`mo`).
    pub days_per_month: u32,
    /// The type new tasks get, as Project's "Default task type" schedule option: Fixed Units
    /// unless changed.
    #[serde(skip_serializing_if = "TaskType::is_fixed_units")]
    pub task_type: TaskType,
    /// Whether new tasks are effort-driven, as Project's "New tasks are effort driven": off
    /// unless changed.
    #[serde(skip_serializing_if = "is_false")]
    pub effort_driven: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Bounds on the options: a day of 1 to 24 hours, a week of 1 to 168, a month of 1 to 31 days.
pub const MAX_MINUTES_PER_DAY: u32 = 24 * 60;
pub const MAX_MINUTES_PER_WEEK: u32 = 168 * 60;
pub const MAX_DAYS_PER_MONTH: u32 = 31;

impl Default for Options {
    fn default() -> Self {
        Options { minutes_per_day: 8 * 60, minutes_per_week: 40 * 60, days_per_month: 20, task_type: TaskType::FixedUnits, effort_driven: false }
    }
}

impl Options {
    /// A day of duration in working minutes, kept in its bounds (1 to 24 hours) whatever the
    /// options hold, so it is always safe to multiply and divide by.
    pub fn day(&self) -> i64 {
        i64::from(self.minutes_per_day.clamp(60, MAX_MINUTES_PER_DAY))
    }

    /// Refuses options outside their bounds (a day of 1 to 24 hours, a week of 1 to 168, a month of 1 to 31 days).
    pub fn check(&self) -> Result<(), ModelError> {
        let ok = (60..=MAX_MINUTES_PER_DAY).contains(&self.minutes_per_day)
            && (60..=MAX_MINUTES_PER_WEEK).contains(&self.minutes_per_week)
            && (1..=MAX_DAYS_PER_MONTH).contains(&self.days_per_month);
        if ok { Ok(()) } else { Err(ModelError::Invalid("hours per day are 1 to 24, hours per week 1 to 168, and days per month 1 to 31".into())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_times_parse_and_print() {
        assert_eq!(parse_clock("8:00").unwrap(), 480);
        assert_eq!(parse_clock(" 08:30 ").unwrap(), 510);
        assert_eq!(parse_clock("17").unwrap(), 1020);
        assert_eq!(parse_clock("24:00").unwrap(), 1440);
        for bad in ["", "24:01", "8:60", "8:5", "-1:00", "8:00:00", "x", "999:00", "65535:00", "8 :00", "+8:00"] {
            assert!(parse_clock(bad).is_err(), "{bad}");
        }
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(1440), "24:00");
        let t = parse_working_time("13:00–17:00").unwrap();
        assert_eq!(t, WorkingTime { from: 780, to: 1020 });
        assert_eq!(t.to_string(), "13:00-17:00");
        assert_eq!(parse_working_time(&t.to_string()).unwrap(), t);
        assert!(parse_working_time("12:00-8:00").is_err());
        assert!(parse_working_time("8:00-8:00").is_err());
        assert!(parse_working_time("8:00").is_err());
    }

    #[test]
    fn a_day_s_working_times_are_in_order_without_overlap() {
        check_working_times(&DEFAULT_WORKING_TIMES).unwrap();
        assert_eq!(minutes_of(&DEFAULT_WORKING_TIMES), 480);
        check_working_times(&[]).unwrap();
        let t = |a, b| WorkingTime { from: a, to: b };
        // Touching is fine; overlapping, out of order, backwards or past midnight is not.
        check_working_times(&[t(0, 720), t(720, 1440)]).unwrap();
        assert!(check_working_times(&[t(480, 780), t(720, 1020)]).is_err());
        assert!(check_working_times(&[t(780, 1020), t(480, 720)]).is_err());
        assert!(check_working_times(&[t(720, 480)]).is_err());
        assert!(check_working_times(&[t(1000, 1500)]).is_err());
        let many: Vec<WorkingTime> = (0..=MAX_WORKING_TIMES as u16).map(|i| t(i * 10, i * 10 + 5)).collect();
        assert!(check_working_times(&many).is_err());
    }

    #[test]
    fn options_default_to_8_40_20_and_are_bounded() {
        let o = Options::default();
        assert_eq!((o.minutes_per_day, o.minutes_per_week, o.days_per_month), (480, 2400, 20));
        o.check().unwrap();
        assert!(Options { minutes_per_day: 0, ..o }.check().is_err());
        assert!(Options { minutes_per_week: MAX_MINUTES_PER_WEEK + 1, ..o }.check().is_err());
        assert!(Options { days_per_month: 0, ..o }.check().is_err());
    }

    proptest::proptest! {
        #[test]
        fn working_time_text_never_panics(s in "\\PC{0,30}") {
            if let Ok(t) = parse_working_time(&s) {
                proptest::prop_assert_eq!(parse_working_time(&t.to_string()).ok(), Some(t));
            }
        }
    }
}
