//! Duration and lag entry: the text typed into the Duration and Predecessors columns (or given to
//! a command), turned into working minutes, and the unit it was typed in, kept for display.
//!
//! As Microsoft Project (its DurationFormat values m, h, d, w and mo:
//! <https://learn.microsoft.com/en-us/office-project/xml-data-interchange/durationformat-element>),
//! a duration is a number with a unit: `30m`, `4h`, `1.5d`, `2w`, `1mo`; a bare number is days.
//! Days, weeks and months turn into minutes through the project options (hours per day, hours per
//! week, days per month), never through the calendar. A duration is stored in whole minutes, as
//! Project's Task.Duration is a count of minutes
//! (<https://learn.microsoft.com/en-us/office/vba/api/project.task.duration>), together with the
//! unit it was typed in, so `4h` shows as `4h` and `1.5d` as `1.5d`.
//!
//! Rounding: an amount that is not a whole number of minutes (`0.3h` at 8 hours a day is 18 minutes,
//! `1.33d` is 638.4) is rounded to the nearest minute, a half minute away from zero (`2.5m` is 3,
//! `-2.5m` is -3). Integer arithmetic on the decimal digits as typed (the first nine decimals; any
//! after are ignored), so the same text always gives the same minutes. Elapsed units (`ed`, `eh`,
//! …) and percent lags are refused for now with a message that says so.

use serde::{Deserialize, Serialize};

use crate::{MAX_MINUTES, ModelError, Options, days_text};

/// The unit a duration or lag was typed in, kept to show it the same way. Written to the file as
/// Microsoft's short form: `"m"`, `"h"`, `"d"`, `"w"`, `"mo"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum DurationUnit {
    #[serde(rename = "m")]
    Minutes,
    #[serde(rename = "h")]
    Hours,
    /// The default: a bare number, and every duration from before units were kept.
    #[default]
    #[serde(rename = "d")]
    Days,
    #[serde(rename = "w")]
    Weeks,
    #[serde(rename = "mo")]
    Months,
}

impl DurationUnit {
    /// Its short form, as typed and shown: `m`, `h`, `d`, `w`, `mo`.
    pub fn suffix(self) -> &'static str {
        match self {
            DurationUnit::Minutes => "m",
            DurationUnit::Hours => "h",
            DurationUnit::Days => "d",
            DurationUnit::Weeks => "w",
            DurationUnit::Months => "mo",
        }
    }

    pub fn is_days(&self) -> bool {
        *self == DurationUnit::Days
    }

    /// The unit a word names: the short form, or Project's longer ones (`hrs`, `days`, `wks`, `mons`).
    pub(crate) fn from_word(w: &str) -> Option<DurationUnit> {
        Some(match w {
            "m" | "min" | "mins" | "minute" | "minutes" => DurationUnit::Minutes,
            "h" | "hr" | "hrs" | "hour" | "hours" => DurationUnit::Hours,
            "" | "d" | "day" | "days" => DurationUnit::Days,
            "w" | "wk" | "wks" | "week" | "weeks" => DurationUnit::Weeks,
            "mo" | "mon" | "mons" | "month" | "months" => DurationUnit::Months,
            _ => return None,
        })
    }
}

impl Options {
    /// Working minutes in one `unit` at these options: a day is the hours per day, a week the
    /// hours per week, a month the days per month of days. Kept in the options' bounds whatever
    /// they hold, so it is always at least 1.
    pub fn minutes_in(&self, unit: DurationUnit) -> i64 {
        match unit {
            DurationUnit::Minutes => 1,
            DurationUnit::Hours => 60,
            DurationUnit::Days => self.day(),
            DurationUnit::Weeks => i64::from(self.minutes_per_week.clamp(60, crate::working_time::MAX_MINUTES_PER_WEEK)),
            DurationUnit::Months => i64::from(self.days_per_month.clamp(1, crate::working_time::MAX_DAYS_PER_MONTH)) * self.day(),
        }
    }
}

/// Decimals of an amount that count; any after are ignored.
const DECIMALS: u32 = 9;
const SCALE: i128 = 1_000_000_000;

/// Why text is not an amount.
enum Refusal {
    Elapsed,
    Percent,
    Junk,
}

/// Reads `[+|-]number[unit]` (the sign only when `signed`) into working minutes, rounded to the
/// nearest minute (a half away from zero) and clamped to [`MAX_MINUTES`] either way, and its unit.
fn parse_amount(text: &str, options: &Options, signed: bool) -> Result<(i64, DurationUnit), Refusal> {
    let t = text.trim().to_ascii_lowercase();
    let (negative, rest) = match t.strip_prefix('-') {
        Some(r) if signed => (true, r),
        _ => (false, if signed { t.strip_prefix('+').unwrap_or(&t) } else { t.as_str() }),
    };
    let rest = rest.trim_start();
    let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let (num, word) = (rest.get(..split).unwrap_or(""), rest.get(split..).unwrap_or("").trim());
    let unit = match DurationUnit::from_word(word) {
        Some(u) => u,
        None if word.len() > 1 && word.starts_with('e') && DurationUnit::from_word(word.get(1..).unwrap_or("x")).is_some() => {
            return Err(Refusal::Elapsed);
        }
        None if word == "%" || word == "e%" => return Err(Refusal::Percent),
        None => return Err(Refusal::Junk),
    };
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    if (whole.is_empty() && frac.is_empty()) || frac.contains('.') {
        return Err(Refusal::Junk);
    }
    // Past 15 digits the amount is far over the cap in any unit: clamp.
    let whole = if whole.len() > 15 { 1_000_000_000_000_000 } else { whole.parse::<i128>().unwrap_or(0) };
    let frac: i128 = frac.bytes().chain(std::iter::repeat(b'0')).take(DECIMALS as usize).fold(0, |n, b| n * 10 + i128::from(b - b'0'));
    let scaled = (whole * SCALE + frac) * i128::from(options.minutes_in(unit));
    let minutes = ((scaled + SCALE / 2) / SCALE).min(i128::from(MAX_MINUTES));
    let minutes = i64::try_from(minutes).unwrap_or(MAX_MINUTES);
    Ok((if negative { -minutes } else { minutes }, unit))
}

/// Parses a duration such as `3`, `1.5d`, `4h`, `30m`, `2 w` or `1MO` into working minutes at the
/// project's `options` (rounded to the minute, clamped to [`MAX_MINUTES`]) and the unit it was
/// typed in. A bare number is days. Negative and elapsed (`ed`, `ew`, …) durations are refused.
pub fn parse_duration(text: &str, options: &Options) -> Result<(i64, DurationUnit), ModelError> {
    parse_amount(text, options, false).map_err(|r| {
        let why = match r {
            Refusal::Elapsed => "elapsed durations aren't supported yet; use m, h, d, w or mo",
            Refusal::Percent | Refusal::Junk => "use a number with m, h, d, w or mo, such as 30m, 4h, 1.5d, 2w or 1mo",
        };
        ModelError::Invalid(format!("not a duration: {:?} ({why})", short(text)))
    })
}

/// Parses a lag such as `+2`, `-1d`, `+4h`, `+1.5d` or `+30m` (a bare number is days; the sign is
/// optional), into signed working minutes and the unit typed. Elapsed and percent lags are refused.
pub fn parse_lag(text: &str, options: &Options) -> Result<(i64, DurationUnit), ModelError> {
    parse_amount(text, options, true).map_err(|r| {
        let why = match r {
            Refusal::Elapsed => "elapsed lags aren't supported yet; use m, h, d, w or mo",
            Refusal::Percent => "percent lags aren't supported yet; use m, h, d, w or mo",
            Refusal::Junk => "a lag is a number with m, h, d, w or mo, such as +2, -1d, +4h or +1.5d",
        };
        ModelError::Invalid(format!("not a lag: {:?} ({why})", short(text)))
    })
}

fn short(text: &str) -> String {
    text.trim().chars().take(40).collect()
}

/// A duration as the Duration column shows it: in the unit it was typed in, to at most two
/// decimals, with its unit (`4h`, `1.5d`, `2w`).
pub fn duration_text(minutes: i64, unit: DurationUnit, options: &Options) -> String {
    format!("{}{}", days_text(minutes, options.minutes_in(unit)), unit.suffix())
}

/// A lag as the Predecessors column shows it: signed, in its unit, days without a suffix (`+2`,
/// `-1.5`, `+4h`); empty for none.
pub fn lag_text(minutes: i64, unit: DurationUnit, options: &Options) -> String {
    if minutes == 0 {
        return String::new();
    }
    let n = days_text(minutes, options.minutes_in(unit));
    let sign = if n.starts_with('-') { "" } else { "+" };
    let suffix = if unit.is_days() { "" } else { unit.suffix() };
    format!("{sign}{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_DAYS;

    const O: Options = Options { minutes_per_day: 480, minutes_per_week: 2400, days_per_month: 20 };
    use DurationUnit::*;

    #[test]
    fn units_become_working_minutes() {
        for (s, m, u) in [
            ("3", 3 * 480, Days),
            ("3d", 1440, Days),
            ("2w", 2 * 2400, Weeks),
            ("1mo", 20 * 480, Months),
            (" 4 D ", 4 * 480, Days),
            ("2 W", 4800, Weeks),
            ("0d", 0, Days),
            ("5 days", 2400, Days),
            ("1 week", 2400, Weeks),
            ("4h", 240, Hours),
            ("30m", 30, Minutes),
            ("1.5d", 720, Days),
            (".5d", 240, Days),
            ("2.25h", 135, Hours),
            ("90 mins", 90, Minutes),
            ("1.5", 720, Days),
            ("0.5mo", 10 * 480, Months),
        ] {
            assert_eq!(parse_duration(s, &O), Ok((m, u)), "{s}");
        }
    }

    #[test]
    fn days_weeks_and_months_follow_the_project_options() {
        let o = Options { minutes_per_day: 450, minutes_per_week: 37 * 60 + 30, days_per_month: 21 };
        assert_eq!(parse_duration("1d", &o), Ok((450, Days)));
        assert_eq!(parse_duration("1w", &o), Ok((2250, Weeks)));
        assert_eq!(parse_duration("1mo", &o), Ok((21 * 450, Months)));
        assert_eq!(parse_duration("1h", &o), Ok((60, Hours)), "hours never change");
    }

    #[test]
    fn part_minutes_round_to_the_nearest_half_away_from_zero() {
        assert_eq!(parse_duration("2.5m", &O), Ok((3, Minutes)));
        assert_eq!(parse_duration("2.4999m", &O), Ok((2, Minutes)));
        assert_eq!(parse_duration("0.3h", &O), Ok((18, Hours)));
        assert_eq!(parse_duration("1.33d", &O), Ok((638, Days)), "638.4");
        assert_eq!(parse_duration("0.001h", &O), Ok((0, Hours)), "under half a minute is none");
        assert_eq!(parse_lag("-2.5m", &O), Ok((-3, Minutes)));
        assert_eq!(parse_duration("1.0000000001m", &O), Ok((1, Minutes)), "past nine decimals is ignored");
    }

    #[test]
    fn clamps_to_max_minutes() {
        assert_eq!(parse_duration("100000000w", &O).unwrap().0, MAX_MINUTES);
        assert_eq!(parse_duration("99999999999999999999999d", &O).unwrap().0, MAX_MINUTES);
        assert_eq!(parse_duration(&format!("{}mo", i64::MAX), &O).unwrap().0, MAX_MINUTES);
        assert_eq!(parse_lag(&format!("-{}.5h", u64::MAX), &O).unwrap().0, -MAX_MINUTES);
        assert_eq!(parse_duration(&format!("{MAX_DAYS}d"), &O).unwrap().0, MAX_DAYS * 480);
    }

    #[test]
    fn elapsed_and_percent_are_refused_clearly() {
        for s in ["3ed", "1ew", "2emo", "4eh", "1em", "2 edays"] {
            assert!(parse_duration(s, &O).unwrap_err().to_string().contains("elapsed durations aren't supported yet"), "{s}");
            assert!(parse_lag(s, &O).unwrap_err().to_string().contains("elapsed lags aren't supported yet"), "{s}");
        }
        assert!(parse_lag("+50%", &O).unwrap_err().to_string().contains("percent lags aren't supported yet"));
    }

    #[test]
    fn junk_is_refused() {
        for s in ["", "d", "w", "-3d", "+3d", "1.5.1d", ".d", "3x", "3 d d", "abc", "3d4", "1,5d", "1e3d", "--1"] {
            assert!(parse_duration(s, &O).is_err(), "{s}");
        }
        for s in ["", "+", "-", "x", "--1", "+-1", "1.2.3"] {
            assert!(parse_lag(s, &O).is_err(), "{s}");
        }
        assert_eq!(parse_lag("-1d", &O), Ok((-480, Days)));
        assert_eq!(parse_lag(" + 4h", &O), Ok((240, Hours)));
    }

    #[test]
    fn display_keeps_the_unit_typed() {
        for (s, shown) in
            [("4h", "4h"), ("1.5d", "1.5d"), ("30m", "30m"), ("2w", "2w"), ("1mo", "1mo"), ("3", "3d"), ("1.333d", "1.33d"), ("0.5h", "0.5h")]
        {
            let (m, u) = parse_duration(s, &O).unwrap();
            assert_eq!(duration_text(m, u, &O), shown, "{s}");
        }
        assert_eq!(lag_text(0, Hours, &O), "");
        assert_eq!(lag_text(960, Days, &O), "+2");
        assert_eq!(lag_text(-720, Days, &O), "-1.5");
        assert_eq!(lag_text(240, Hours, &O), "+4h");
        assert_eq!(lag_text(-30, Minutes, &O), "-30m");
        // Hours per day changed after entry: a day count shows at the new hours, hours stay hours.
        let six = Options { minutes_per_day: 360, ..O };
        assert_eq!(duration_text(720, Days, &six), "2d");
        assert_eq!(duration_text(240, Hours, &six), "4h");
    }

    proptest::proptest! {
        #[test]
        fn parsing_never_panics(s in "\\PC{0,40}", day in 0u32..2000, week in 0u32..20000, month in 0u32..40) {
            let o = Options { minutes_per_day: day, minutes_per_week: week, days_per_month: month };
            if let Ok((m, u)) = parse_duration(&s, &o) {
                proptest::prop_assert!((0..=MAX_MINUTES).contains(&m));
                let _ = duration_text(m, u, &o);
            }
            if let Ok((m, u)) = parse_lag(&s, &o) {
                proptest::prop_assert!((-MAX_MINUTES..=MAX_MINUTES).contains(&m));
                let _ = lag_text(m, u, &o);
            }
        }

        #[test]
        fn numbers_with_units_parse(n in 0i64..1_000_000, cents in 0i64..100, unit in "(m|h|d|w|mo|M|H|D|W|MO| d| h| mo)?") {
            let u = DurationUnit::from_word(unit.trim().to_ascii_lowercase().as_str()).unwrap();
            let per = O.minutes_in(u);
            // n.cents units, rounded half up, in integer arithmetic.
            let want = ((n * 100 + cents) * per * 2 + 100) / 200;
            proptest::prop_assert_eq!(parse_duration(&format!("{n}.{cents:02}{unit}"), &O), Ok((want.min(MAX_MINUTES), u)));
        }

        #[test]
        fn shown_text_reads_back_to_the_same_unit(m in 0i64..10_000_000, k in 0usize..5) {
            let u = [Minutes, Hours, Days, Weeks, Months][k];
            let (back, unit) = parse_duration(&duration_text(m, u, &O), &O).unwrap();
            proptest::prop_assert_eq!(unit, u);
            // Two decimals of the unit: within half a hundredth of it.
            proptest::prop_assert!((back - m).abs() * 200 <= O.minutes_in(u) + 200);
        }
    }
}
