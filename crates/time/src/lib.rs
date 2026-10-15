//! PlanCraft dates: a civil date as a day number, its weekday and its ISO text.
//!
//! Plans are scheduled in whole working days, so there is no time of day and no time zone here.
//! ponytail: day granularity only; add hours (and `jiff`, as MailCraft does) when tasks need
//! part-day durations.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Days since 1970-01-01 (proleptic Gregorian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Date(i32);

/// Years outside this range are refused, so day arithmetic never overflows.
const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9999;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateError(pub String);

impl fmt::Display for DateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a date: {}", self.0)
    }
}

impl std::error::Error for DateError {}

impl Date {
    pub const EPOCH: Date = Date(0);

    pub fn from_ymd(y: i32, m: u32, d: u32) -> Option<Date> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        // Howard Hinnant's days_from_civil.
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m as i32 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d as i32 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(Date(era * 146_097 + doe - 719_468))
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        let z = self.0 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        (yoe + era * 400 + i32::from(m <= 2), m, d)
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday.
        (self.0 + 3).rem_euclid(7) as u8
    }

    /// The date `n` days later (earlier when negative), or `None` past year 9999 or before year 1.
    pub fn add_days(self, n: i64) -> Option<Date> {
        let d = i32::try_from(i64::from(self.0).checked_add(n)?).ok()?;
        let (y, _, _) = Date(d).ymd();
        (MIN_YEAR..=MAX_YEAR).contains(&y).then_some(Date(d))
    }

    pub fn days_since(self, earlier: Date) -> i64 {
        i64::from(self.0) - i64::from(earlier.0)
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl FromStr for Date {
    type Err = DateError;
    /// `YYYY-MM-DD`.
    fn from_str(s: &str) -> Result<Date, DateError> {
        let bad = || DateError(s.chars().take(40).collect());
        let mut it = s.trim().splitn(3, '-');
        let mut part = || it.next().and_then(|p| p.parse::<i64>().ok()).ok_or_else(bad);
        let (y, m, d) = (part()?, part()?, part()?);
        let (y, m, d) = (i32::try_from(y).map_err(|_| bad())?, u32::try_from(m).map_err(|_| bad())?, u32::try_from(d).map_err(|_| bad())?);
        Date::from_ymd(y, m, d).ok_or_else(bad)
    }
}

impl From<Date> for String {
    fn from(d: Date) -> String {
        d.to_string()
    }
}

impl TryFrom<String> for Date {
    type Error = DateError;
    fn try_from(s: String) -> Result<Date, DateError> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_weekdays() {
        let d = Date::from_ymd(2026, 10, 15).unwrap();
        assert_eq!(d.to_string(), "2026-10-15");
        assert_eq!(d.weekday(), 3); // Thursday
        assert_eq!("2024-02-29".parse::<Date>().unwrap().ymd(), (2024, 2, 29));
        assert!("2023-02-29".parse::<Date>().is_err());
        assert!("junk".parse::<Date>().is_err());
        assert!("99999999999-1-1".parse::<Date>().is_err());
        for n in -800_000..800_000 {
            if let Some(d) = Date::from_ymd(1970, 1, 1).and_then(|e| e.add_days(n)) {
                let (y, m, dd) = d.ymd();
                assert_eq!(Date::from_ymd(y, m, dd), Some(d));
            }
        }
        assert_eq!(Date::from_ymd(9999, 12, 31).unwrap().add_days(1), None);
    }
}
