//! Visual RTL BiDi reordering for `egui 0.36`, which renders strings in literal glyph order
//! without UAX #9 paragraph reordering.
//!
//! Assumptions and scope:
//! - Contiguous RTL script stretches (Hebrew, Arabic, Syriac, Thaana, N'Ko, Adlam) remain in
//!   logical order internally so `epaint`/`harfrust` shapes each word cleanly.
//! - Runs are ordered visually right-to-left per line, keeping numbers, ASCII/Latin words,
//!   path prefixes (`/tmp/...`, `.env`), currency/unit signs attached to numbers, and `{name}`
//!   template placeholders intact (placeholders are masked into Private Use Area codepoints before
//!   reordering and restored afterward).
//! - Paired brackets (`()`, `[]`, `{}`, `<>`, `«»`) resolve direction from their enclosed content
//!   and are mirrored only inside RTL runs.

#[derive(Clone, Copy, PartialEq, Eq)]
enum BidiClass {
    L,
    R,
    En,
    N,
}

pub(super) fn is_rtl_script(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0590..=0x05FF | 0x0600..=0x065F | 0x066E..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF
    )
}

fn is_bidi_digit(c: char) -> bool {
    c.is_ascii_digit() || matches!(u32::from(c), 0x0660..=0x0669 | 0x06F0..=0x06F9)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

fn bracket_close(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '<' => Some('>'),
        '«' => Some('»'),
        _ => None,
    }
}

fn is_closing_bracket(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '>' | '»')
}

const PLACEHOLDER_BASE: u32 = 0xE000;
const MAX_PLACEHOLDERS: usize = 64;

fn mask_placeholders(text: &str) -> (String, Vec<&str>) {
    let reserved = text.chars().any(|c| (PLACEHOLDER_BASE..PLACEHOLDER_BASE.saturating_add(MAX_PLACEHOLDERS as u32)).contains(&u32::from(c)));
    if reserved {
        return (text.to_owned(), Vec::new());
    }
    let mut masked = String::with_capacity(text.len());
    let mut names = Vec::new();
    let mut rest = text;
    while names.len() < MAX_PLACEHOLDERS
        && let Some(start) = rest.find('{')
        && let Some(len) = rest.get(start..).and_then(|s| s.find('}'))
    {
        let end = start.saturating_add(len).saturating_add(1);
        let (Some(before), Some(atom), Some(after)) = (rest.get(..start), rest.get(start..end), rest.get(end..)) else {
            break;
        };
        let Some(stand_in) = u32::try_from(names.len()).ok().and_then(|i| char::from_u32(PLACEHOLDER_BASE.saturating_add(i))) else {
            break;
        };
        masked.push_str(before);
        masked.push(stand_in);
        names.push(atom);
        rest = after;
    }
    masked.push_str(rest);
    (masked, names)
}

fn push_unmasked(out: &mut String, c: char, names: &[&str]) {
    let idx = u32::from(c).checked_sub(PLACEHOLDER_BASE).and_then(|i| usize::try_from(i).ok());
    if let Some(name) = idx.and_then(|i| names.get(i)) {
        out.push_str(name);
    } else {
        out.push(c);
    }
}

pub(super) fn push_visual_rtl_line(line: &str, out: &mut String) {
    let (masked, names) = mask_placeholders(line);
    let chars: Vec<char> = masked.chars().filter(|c| !is_bidi_control(*c)).collect();
    if chars.is_empty() {
        return;
    }
    let len = chars.len();
    let mut cls: Vec<BidiClass> = chars
        .iter()
        .map(|&c| {
            let is_placeholder = (PLACEHOLDER_BASE..PLACEHOLDER_BASE.saturating_add(MAX_PLACEHOLDERS as u32)).contains(&u32::from(c));
            if is_rtl_script(c) {
                BidiClass::R
            } else if is_bidi_digit(c) {
                BidiClass::En
            } else if is_placeholder || c.is_alphabetic() || matches!(c, '\u{2318}' | '\u{21E7}' | '\u{2325}' | '\u{2303}') {
                BidiClass::L
            } else {
                BidiClass::N
            }
        })
        .collect();

    for i in 1..len.saturating_sub(1) {
        if cls.get(i) == Some(&BidiClass::N)
            && chars.get(i).is_some_and(|c| matches!(c, '.' | ',' | ':' | '/' | '-' | '+' | '_'))
            && let (Some(prev), Some(next)) = (cls.get(i - 1), cls.get(i + 1))
        {
            if *prev == BidiClass::En && *next == BidiClass::En {
                if let Some(slot) = cls.get_mut(i) {
                    *slot = BidiClass::En;
                }
            } else if matches!(prev, BidiClass::L | BidiClass::En)
                && matches!(next, BidiClass::L | BidiClass::En)
                && let Some(slot) = cls.get_mut(i)
            {
                *slot = BidiClass::L;
            }
        }
    }

    for i in 0..len {
        let Some(&ch) = chars.get(i) else { continue };
        if cls.get(i) != Some(&BidiClass::N) {
            continue;
        }
        let next_en = cls.get(i + 1) == Some(&BidiClass::En);
        let prev_en = i > 0 && cls.get(i - 1) == Some(&BidiClass::En);
        let prev_is_r = i > 0 && cls.get(i - 1) == Some(&BidiClass::R);
        let attach = match ch {
            '%' | '°' | '$' | '€' | '£' | '₪' => prev_en || next_en,
            '+' | '-' => next_en && !prev_is_r,
            _ => false,
        };
        if attach && let Some(slot) = cls.get_mut(i) {
            *slot = BidiClass::En;
        }
    }

    // Keep leading path/dot/wildcard prefixes (e.g. `/tmp/...`, `.env`, `*.projectcraft`)
    // attached to following LTR/numeric runs.
    for i in (0..len.saturating_sub(1)).rev() {
        let is_path_prefix = chars.get(i).is_some_and(|c| matches!(c, '/' | '.' | '~' | '\\'));
        let is_glob_prefix = chars.get(i) == Some(&'*') && chars.get(i + 1) == Some(&'.') && cls.get(i + 1) == Some(&BidiClass::L);
        if cls.get(i) == Some(&BidiClass::N)
            && (is_path_prefix || is_glob_prefix)
            && matches!(cls.get(i + 1), Some(BidiClass::L | BidiClass::En))
            && (i == 0 || cls.get(i - 1) != Some(&BidiClass::R))
            && let Some(slot) = cls.get_mut(i)
        {
            *slot = BidiClass::L;
        }
    }

    let mut stack: Vec<(usize, char)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if cls.get(i) != Some(&BidiClass::N) {
            continue;
        }
        if let Some(close) = bracket_close(c) {
            if stack.len() < 63 {
                stack.push((i, close));
            }
        } else if is_closing_bracket(c)
            && let Some(pos) = stack.iter().rposition(|(_, expected)| *expected == c)
        {
            let (open_idx, _) = stack.remove(pos);
            pairs.push((open_idx, i));
        }
    }
    pairs.sort_by_key(|(open, _)| *open);
    for (open, close) in pairs {
        let inner = cls.get(open + 1..close).unwrap_or(&[]);
        let has_r = inner.contains(&BidiClass::R);
        let has_l = inner.iter().any(|c| matches!(c, BidiClass::L | BidiClass::En));
        let dir = if has_r {
            BidiClass::R
        } else if has_l {
            let prev_strong = cls.get(..open).unwrap_or(&[]).iter().rev().find(|c| **c != BidiClass::N).copied();
            if matches!(prev_strong, Some(BidiClass::L | BidiClass::En)) { BidiClass::L } else { BidiClass::R }
        } else {
            BidiClass::R
        };
        if let Some(s) = cls.get_mut(open) {
            *s = dir;
        }
        if let Some(s) = cls.get_mut(close) {
            *s = dir;
        }
    }

    for c in &mut cls {
        if *c == BidiClass::En {
            *c = BidiClass::L;
        }
    }
    let mut i = 0;
    while i < len {
        if cls.get(i) == Some(&BidiClass::N) {
            let start = i;
            while cls.get(i) == Some(&BidiClass::N) {
                i += 1;
            }
            let left_l = start > 0 && cls.get(start - 1) == Some(&BidiClass::L);
            let right_l = cls.get(i) == Some(&BidiClass::L);
            let resolved = if left_l && right_l { BidiClass::L } else { BidiClass::R };
            if let Some(slice) = cls.get_mut(start..i) {
                slice.fill(resolved);
            }
        } else {
            i += 1;
        }
    }

    let mut runs: Vec<(bool, usize, usize)> = Vec::new();
    let mut r_start = 0;
    while r_start < len {
        let rtl = cls.get(r_start) == Some(&BidiClass::R);
        let mut r_end = r_start + 1;
        while cls.get(r_end).is_some_and(|c| (*c == BidiClass::R) == rtl) {
            r_end += 1;
        }
        runs.push((rtl, r_start, r_end));
        r_start = r_end;
    }

    for (rtl, start, end) in runs.into_iter().rev() {
        let Some(slice) = chars.get(start..end) else { continue };
        if rtl {
            let mut stretch: Vec<char> = Vec::new();
            for &c in slice.iter().rev() {
                if is_rtl_script(c) || is_joiner(c) {
                    stretch.push(c);
                } else {
                    for sc in stretch.drain(..).rev() {
                        push_unmasked(out, sc, &names);
                    }
                    push_unmasked(out, mirrored(c), &names);
                }
            }
            for sc in stretch.drain(..).rev() {
                push_unmasked(out, sc, &names);
            }
        } else {
            for &c in slice {
                push_unmasked(out, c, &names);
            }
        }
    }
}

/// Rearrange `text` into visual right-to-left order for egui 0.36 while keeping contiguous RTL
/// script stretches in logical order so epaint's harfrust shapes them cleanly.
pub(super) fn visual_rtl(text: &str) -> String {
    if !text.chars().any(is_rtl_script) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for (idx, raw_line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let (line, cr) = raw_line.strip_suffix('\r').map_or((raw_line, false), |l| (l, true));
        push_visual_rtl_line(line, &mut out);
        if cr {
            out.push('\r');
        }
    }
    out
}
