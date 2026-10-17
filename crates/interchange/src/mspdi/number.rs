//! Numbers as MSPDI writes them: `xs:decimal` amounts (`42.37`, a rate, a cost) and `xs:float`
//! units (`1`, `0.5`, `6.5`, perhaps `1.0E0`), read exactly into whole units of `10^-places`, with
//! no floating point: a decimal amount of money becomes minor units by its currency's places, as
//! ADR 0006 has it, rounded half to even when the file holds finer amounts than that.

use crate::model::div_half_even;

/// Most significant digits read; any after them are only counted, as a rounding.
const MAX_DIGITS: usize = 36;

/// How a number read compares with what ProjectCraft holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Read {
    /// Held as the file has it.
    Exact,
    /// Finer than ProjectCraft holds: rounded half to even.
    Rounded,
    /// Negative, too large or unreadable: 0, the largest held, or the default.
    OutOfRange,
}

/// `text` in whole units of `10^-places` (`"42.37"` with 2 places is 4237), and whether it was
/// rounded (half to even) to get there. A huge value saturates; `None` when unreadable.
pub(super) fn scaled(text: &str, places: u32) -> Option<(i128, bool)> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (mantissa, exponent) = match t.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (t, 0),
    };
    let (whole, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && frac.is_empty()) || !whole.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let all = format!("{whole}{frac}");
    let digits = all.trim_start_matches('0');
    if digits.is_empty() {
        return Some((0, false));
    }
    // Digits past the most kept are rounded off here: a value's tail that far down never reaches
    // a minor unit unless the exponent brings it up, and then it saturates anyway.
    let kept = digits.get(..MAX_DIGITS).unwrap_or(digits);
    let dropped = digits.len().saturating_sub(kept.len());
    let mut rounded = digits.get(MAX_DIGITS..).is_some_and(|rest| rest.bytes().any(|b| b != b'0'));
    let m: i128 = kept.parse().ok()?;
    let shift = i64::from(exponent) - i64::try_from(frac.len()).ok()? + i64::try_from(dropped).ok()? + i64::from(places);
    let value = if shift >= 0 {
        u32::try_from(shift).ok().and_then(|s| 10i128.checked_pow(s)).and_then(|p| m.checked_mul(p)).unwrap_or(i128::MAX)
    } else {
        match u32::try_from(-shift).ok().and_then(|s| 10i128.checked_pow(s)) {
            Some(p) => {
                rounded |= m % p != 0;
                div_half_even(m, p)
            }
            // Smaller than one unit by more than 38 places.
            None => {
                rounded = true;
                0
            }
        }
    };
    Some((if negative { value.saturating_neg() } else { value }, rounded))
}

/// `text` as whole units of `10^-places` between 0 and `max`; `default` when unreadable.
pub(super) fn within(text: &str, places: u32, max: i64, default: i64) -> (i64, Read) {
    match scaled(text, places) {
        None => (default, Read::OutOfRange),
        Some((v, _)) if v < 0 => (0, Read::OutOfRange),
        Some((v, _)) if v > i128::from(max) => (max, Read::OutOfRange),
        Some((v, rounded)) => (i64::try_from(v).unwrap_or(max), if rounded { Read::Rounded } else { Read::Exact }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_scale_exactly_and_round_half_to_even() {
        for (text, places, want) in [
            ("42.37", 2, Some((4237, false))),
            ("15", 2, Some((1500, false))),
            ("63.555", 2, Some((6356, true))),
            ("63.545", 2, Some((6354, true))),
            ("0.5", 4, Some((5000, false))),
            ("1.0E0", 4, Some((10_000, false))),
            ("2.5e-1", 2, Some((25, false))),
            ("-20", 2, Some((-2000, false))),
            ("+7", 0, Some((7, false))),
            (".5", 1, Some((5, false))),
            ("5.", 1, Some((50, false))),
            ("000", 2, Some((0, false))),
            ("1e40", 2, Some((i128::MAX, false))),
            ("1e-60", 2, Some((0, true))),
            ("12345678901234567890123456789012345678", 0, Some((123_456_789_012_345_678_901_234_567_890_123_456 * 100, true))),
            ("1234567890123456789012345678901234567890", 0, Some((i128::MAX, true))),
            ("", 2, None),
            (".", 2, None),
            ("1.2.3", 2, None),
            ("abc", 2, None),
            ("NaN", 2, None),
            ("INF", 2, None),
            ("1e", 2, None),
            ("1e99999999999", 2, None),
            ("--1", 2, None),
            ("1 000", 2, None),
        ] {
            assert_eq!(scaled(text, places), want, "{text}");
        }
    }

    #[test]
    fn values_are_kept_within_range() {
        assert_eq!(within("12.5", 2, 10_000, 7), (1250, Read::Exact));
        assert_eq!(within("12.505", 2, 10_000, 7), (1250, Read::Rounded));
        assert_eq!(within("-1", 2, 10_000, 7), (0, Read::OutOfRange));
        assert_eq!(within("1e9", 2, 10_000, 7), (10_000, Read::OutOfRange));
        assert_eq!(within("x", 2, 10_000, 7), (7, Read::OutOfRange));
    }
}
