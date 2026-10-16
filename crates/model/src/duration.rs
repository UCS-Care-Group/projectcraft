//! Duration entry: the text typed into the Duration column (or given to the CLI) as working days.

use crate::{MAX_DAYS, ModelError};

/// Working days in a week for `w`. ponytail: a project setting when calendars get options.
pub const DAYS_PER_WEEK: i64 = 5;
/// Working days in a month for `mo`. ponytail: a project setting when calendars get options.
pub const DAYS_PER_MONTH: i64 = 20;

/// Parses a duration such as `3`, `3d`, `2 w` or `1MO` into whole working days, clamped to
/// [`MAX_DAYS`]. A bare number is days. Part-day (`h`, `m`) and elapsed (`ed`, `ew`, …) units are
/// refused until part-day durations exist.
pub fn parse_duration(text: &str) -> Result<i64, ModelError> {
    let t = text.trim().to_ascii_lowercase();
    let split = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (num, unit) = (t.get(..split).unwrap_or(""), t.get(split..).unwrap_or("").trim());
    let per = match unit {
        "" | "d" | "day" | "days" => 1,
        "w" | "wk" | "wks" | "week" | "weeks" => DAYS_PER_WEEK,
        "mo" | "mon" | "mons" | "month" | "months" => DAYS_PER_MONTH,
        "h" | "hr" | "hrs" | "hour" | "hours" | "m" | "min" | "mins" | "minute" | "minutes" => {
            return Err(invalid(text, "part-day durations aren't supported yet; use d, w or mo"));
        }
        "em" | "eh" | "ed" | "ew" | "emo" => return Err(invalid(text, "elapsed durations aren't supported yet; use d, w or mo")),
        _ => return Err(invalid(text, "use a whole number of days, weeks or months, such as 3d, 2w or 1mo")),
    };
    if num.is_empty() {
        return Err(invalid(text, "use a whole number of days, weeks or months, such as 3d, 2w or 1mo"));
    }
    // Every char of `num` is a digit, so a parse failure means it overflowed: clamp.
    let n = num.parse::<i64>().unwrap_or(MAX_DAYS);
    Ok(n.saturating_mul(per).min(MAX_DAYS))
}

fn invalid(text: &str, why: &str) -> ModelError {
    ModelError::Invalid(format!("not a duration: {:?} ({why})", text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_become_working_days() {
        for (s, d) in [("3", 3), ("3d", 3), ("2w", 10), ("1mo", 20), (" 4 D ", 4), ("2 W", 10), ("3Mo", 60), ("0d", 0), ("5 days", 5), ("1 week", 5)]
        {
            assert_eq!(parse_duration(s), Ok(d), "{s}");
        }
    }

    #[test]
    fn clamps_to_max_days() {
        assert_eq!(parse_duration("100000w"), Ok(MAX_DAYS));
        assert_eq!(parse_duration("99999999999999999999999d"), Ok(MAX_DAYS));
        assert_eq!(parse_duration(&format!("{}mo", i64::MAX)), Ok(MAX_DAYS));
    }

    #[test]
    fn part_day_and_elapsed_are_refused_clearly() {
        for s in ["4h", "30m", "2 hrs"] {
            assert!(parse_duration(s).unwrap_err().to_string().contains("part-day durations aren't supported yet; use d, w or mo"), "{s}");
        }
        for s in ["3ed", "1ew", "2emo", "4eh", "1em"] {
            assert!(parse_duration(s).unwrap_err().to_string().contains("elapsed durations aren't supported yet"), "{s}");
        }
    }

    #[test]
    fn junk_is_refused() {
        for s in ["", "d", "w", "-3d", "1.5d", "3x", "3 d d", "abc", "3d4"] {
            assert!(parse_duration(s).is_err(), "{s}");
        }
    }

    proptest::proptest! {
        #[test]
        fn parse_duration_never_panics(s in "\\PC{0,40}") {
            if let Ok(d) = parse_duration(&s) {
                proptest::prop_assert!((0..=MAX_DAYS).contains(&d));
            }
        }

        #[test]
        fn digits_with_units_parse(n in 0i64..1_000_000, unit in "(d|w|mo|D|W|MO| d| w| mo)?") {
            let per = match unit.trim().to_ascii_lowercase().as_str() { "w" => 5, "mo" => 20, _ => 1 };
            proptest::prop_assert_eq!(parse_duration(&format!("{n}{unit}")), Ok((n * per).min(MAX_DAYS)));
        }
    }
}
