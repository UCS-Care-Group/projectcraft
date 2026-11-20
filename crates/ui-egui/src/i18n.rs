//! Interface text. Every string the interface shows goes through [`t`] (the `tl!` macro), keyed by
//! its English text, so language catalogs work without touching the widgets. Includes English,
//! Hebrew (`he`, right-to-left with visual reordering for egui 0.36), and `Pseudo` (a test language
//! that highlights strings skipping translation).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

const HEBREW_TSV: &str = include_str!("i18n/he.tsv");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    English = 0,
    /// Accented look-alikes of the English text, for finding strings that skip translation.
    Pseudo = 1,
    /// Hebrew (עברית), right-to-left.
    Hebrew = 2,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::English, Lang::Pseudo, Lang::Hebrew];

    /// BCP 47 tag for this UI language.
    pub const fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Pseudo => "pseudo",
            Lang::Hebrew => "he",
        }
    }

    /// Display name of the language in itself.
    pub const fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Pseudo => "Pseudo",
            Lang::Hebrew => "עברית",
        }
    }

    /// Whether this language reads right-to-left.
    pub const fn is_rtl(self) -> bool {
        matches!(self, Lang::Hebrew)
    }

    /// Resolve a locale or language tag (normalizing legacy `iw` tags to [`Lang::Hebrew`]).
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let cleaned = tag.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
        let mut parts = cleaned.split('-');
        let primary = parts.next().unwrap_or("");
        match primary {
            "en" => Some(Lang::English),
            "pseudo" => Some(Lang::Pseudo),
            "he" | "iw" => Some(Lang::Hebrew),
            _ => None,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Pseudo,
        2 => Lang::Hebrew,
        _ => Lang::English,
    }
}

/// Whether `code` names a right-to-left language or ISO 15924 script (normalizing legacy `iw` to
/// `he`, ignoring private-use `x-*` tags, and honoring an explicit script subtag after any
/// 3-letter extlang subtags).
pub fn is_rtl_code(code: &str) -> bool {
    let cleaned = code.trim().split(['.', '@']).next().unwrap_or_default().replace('_', "-").to_ascii_lowercase();
    let mut parts = cleaned.split('-').filter(|p| !p.is_empty());
    let Some(base) = parts.next() else {
        return false;
    };
    if base == "x" {
        return false;
    }
    let next = parts.find(|p| !(p.len() == 3 && p.bytes().all(|b| b.is_ascii_alphabetic())));
    if let Some(script) = next.filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic())) {
        return matches!(script, "arab" | "hebr" | "nkoo" | "rohg" | "syrc" | "thaa" | "yezi");
    }
    matches!(base, "ar" | "ckb" | "dv" | "fa" | "he" | "iw" | "ps" | "sd" | "ug" | "ur" | "yi")
}

/// Whether the active interface language uses right-to-left script direction.
pub fn is_rtl() -> bool {
    current().is_rtl()
}

struct HebrewCatalog {
    en_to_visual: HashMap<&'static str, &'static str>,
    visual_to_logical: HashMap<&'static str, &'static str>,
}

fn hebrew_catalog() -> &'static HebrewCatalog {
    static CATALOG: OnceLock<HebrewCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut en_to_visual = HashMap::new();
        let mut visual_to_logical = HashMap::new();
        for line in HEBREW_TSV.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, logical)) = line.split_once('\t') else {
                continue;
            };
            if key.is_empty() || logical.trim().is_empty() {
                continue;
            }
            let visual_str = visual_rtl(logical);
            let visual: &'static str = if visual_str == logical {
                logical
            } else {
                // Static catalog entries are small and fixed for the lifetime of the process.
                Box::leak(visual_str.into_boxed_str())
            };
            en_to_visual.insert(key, visual);
            visual_to_logical.insert(visual, logical);
        }
        HebrewCatalog { en_to_visual, visual_to_logical }
    })
}

/// Recover the logical string for a catalog entry when `s` was visually reordered for RTL.
pub fn logical_of(s: &str) -> &str {
    if is_rtl() { hebrew_catalog().visual_to_logical.get(s).copied().unwrap_or(s) } else { s }
}

fn wrap_paragraph_logical<'a>(ctx: &egui::Context, paragraph: &'a str, font_id: &egui::FontId, max_width: f32) -> Vec<&'a str> {
    if paragraph.is_empty() || !max_width.is_finite() {
        return vec![paragraph];
    }
    let max_w = max_width.max(1.0);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut in_word = false;
    let mut word_start = 0usize;
    for (idx, ch) in paragraph.char_indices() {
        if ch.is_whitespace() {
            if in_word {
                spans.push((word_start, idx));
                in_word = false;
            }
        } else if !in_word {
            word_start = idx;
            in_word = true;
        }
    }
    if in_word {
        spans.push((word_start, paragraph.len()));
    }
    let Some(&(mut line_start, mut line_end)) = spans.first() else {
        return vec![paragraph];
    };
    let mut rows = Vec::new();
    for &(span_start, span_end) in spans.iter().skip(1) {
        let candidate = &paragraph[line_start..span_end];
        let width = ctx.fonts_mut(|f| f.layout_no_wrap(candidate.to_owned(), font_id.clone(), Default::default()).rect.width());
        if width <= max_w {
            line_end = span_end;
        } else {
            rows.push(&paragraph[line_start..line_end]);
            line_start = span_start;
            line_end = span_end;
        }
    }
    rows.push(&paragraph[line_start..line_end]);
    rows
}

/// Break `logical` text at `max_width` first using original logical substrings, then apply visual
/// RTL ordering to each display row containing RTL characters.
pub fn wrapped_visual_lines(ctx: &egui::Context, lang: Lang, logical: &str, font_id: egui::FontId, max_width: f32) -> String {
    if !lang.is_rtl() || !logical.chars().any(is_rtl_script) {
        return logical.to_owned();
    }
    let mut out = String::with_capacity(logical.len());
    let mut first_row = true;
    for paragraph in logical.split('\n') {
        for row_logical in wrap_paragraph_logical(ctx, paragraph, &font_id, max_width) {
            if !first_row {
                out.push('\n');
            }
            first_row = false;
            if row_logical.chars().any(is_rtl_script) {
                push_visual_rtl_line(row_logical, &mut out);
            } else {
                out.push_str(row_logical);
            }
        }
    }
    out
}

/// Render a wrapping label for an explicit `lang`, deferring RTL visual reordering until after
/// egui determines line breaks.
pub fn wrapped_label_for(ui: &mut egui::Ui, lang: Lang, text: &str, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let logical = if lang.is_rtl() { logical_of(text) } else { text };
    let needs_rtl_wrap = lang.is_rtl() && logical.chars().any(is_rtl_script);
    let shown =
        if needs_rtl_wrap { wrapped_visual_lines(ui.ctx(), lang, logical, font_id.clone(), ui.available_width()) } else { logical.to_owned() };
    let rich = style(egui::RichText::new(shown).font(font_id));
    if needs_rtl_wrap { ui.add(egui::Label::new(rich).extend()) } else { ui.add(egui::Label::new(rich).wrap()) }
}

/// Render a wrapping label, deferring RTL visual reordering until after egui determines line breaks.
pub fn wrapped_label(ui: &mut egui::Ui, text: &str, style: impl FnOnce(egui::RichText) -> egui::RichText) -> egui::Response {
    wrapped_label_for(ui, current(), text, style)
}

/// `s` in `lang` (in visual RTL order for egui 0.36 when [`Lang::Hebrew`] is passed).
pub fn t_for(lang: Lang, s: &str) -> &str {
    match lang {
        Lang::English => s,
        Lang::Pseudo => pseudo(s),
        Lang::Hebrew => hebrew_catalog().en_to_visual.get(s).copied().unwrap_or(s),
    }
}

/// `s` in the interface language (in visual RTL order for egui 0.36 when Hebrew is active).
pub fn t(s: &str) -> &str {
    t_for(current(), s)
}

/// The pseudo-language form of `s`, interned so it can be handed out as `&'static str`.
fn pseudo(s: &str) -> &'static str {
    use std::sync::Mutex;
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(done) = cache.get(s) {
        return done;
    }
    // `{placeholders}` stay as they are.
    let mut inside = false;
    let out: String = s
        .chars()
        .map(|c| {
            match c {
                '{' => inside = true,
                '}' => inside = false,
                _ => {}
            }
            if inside {
                return c;
            }
            match c {
                'a' => 'á',
                'e' => 'é',
                'i' => 'í',
                'o' => 'ó',
                'u' => 'ú',
                'A' => 'Á',
                'E' => 'É',
                'O' => 'Ó',
                'U' => 'Ú',
                _ => c,
            }
        })
        .collect();
    // The catalog is small and fixed (interface strings), so leaking each form once is bounded.
    let leaked: &'static str = Box::leak(out.into_boxed_str());
    cache.insert(s.to_string(), leaked);
    leaked
}

/// Fill `{name}` placeholders in `template` for `lang`, keeping logical text order (for
/// row-aware wrapping in [`wrapped_label`]).
pub fn fmt_logical_for(lang: Lang, template: &str, args: &[(&str, &str)]) -> String {
    let source = if lang == Lang::Hebrew { hebrew_catalog().visual_to_logical.get(template).copied().unwrap_or(template) } else { template };
    fill_placeholders(source, args)
}

/// Fill `{name}` placeholders in a (translated) template in one pass, keeping logical text order.
pub fn fmt_logical(template: &str, args: &[(&str, &str)]) -> String {
    fmt_logical_for(current(), template, args)
}

/// Fill `{name}` placeholders in `template` for `lang`. When [`Lang::Hebrew`] is selected,
/// recovers the logical template before substituting so inserted values reorder cleanly for egui 0.36.
pub fn fmt_for(lang: Lang, template: &str, args: &[(&str, &str)]) -> String {
    let out = fmt_logical_for(lang, template, args);
    if lang == Lang::Hebrew { visual_rtl(&out) } else { out }
}

/// Fill `{name}` placeholders in a (translated) template in one pass. When Hebrew is active,
/// recovers the logical template before substituting so inserted values (including Hebrew task or
/// resource names) reorder consistently for egui 0.36 display.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    fmt_for(current(), template, args)
}

fn fill_placeholders(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after_open = rest.get(open + 1..).unwrap_or_default();
        let Some(close) = after_open.find('}') else {
            out.push_str(rest.get(open..).unwrap_or_default());
            return out;
        };
        let key = after_open.get(..close).unwrap_or_default();
        if let Some((_, val)) = args.iter().find(|(k, _)| *k == key) {
            out.push_str(val);
        } else {
            out.push('{');
            out.push_str(key);
            out.push('}');
        }
        rest = after_open.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BidiClass {
    L,
    R,
    En,
    N,
}

fn is_rtl_script(c: char) -> bool {
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

fn push_visual_rtl_line(line: &str, out: &mut String) {
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

    // Keep leading path/dot prefixes (e.g. `/tmp/...`, `.env`) attached to following LTR/numeric runs.
    for i in (0..len.saturating_sub(1)).rev() {
        if cls.get(i) == Some(&BidiClass::N)
            && chars.get(i).is_some_and(|c| matches!(c, '/' | '.' | '~' | '\\'))
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
fn visual_rtl(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else { break };
            out.push(after[..close].to_owned());
            rest = &after[close + 1..];
        }
        out.sort();
        out
    }

    fn parsed_hebrew_rows() -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (idx, line) in HEBREW_TSV.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line.split_once('\t').unwrap_or_else(|| panic!("he.tsv:{}: missing tab", idx + 1));
            assert!(!k.is_empty() && !v.trim().is_empty(), "he.tsv:{}: empty column", idx + 1);
            assert!(!v.contains('\t'), "he.tsv:{}: extra tab", idx + 1);
            assert!(map.insert(k.to_owned(), v.to_owned()).is_none(), "he.tsv:{}: duplicate key {k:?}", idx + 1);
        }
        map
    }

    #[test]
    fn formatting_and_pseudo() {
        assert_eq!(fmt("Start {start} · Finish {finish}", &[("start", "Mon"), ("finish", "Fri")]), "Start Mon · Finish Fri");
        assert_eq!(pseudo("Task {name}"), "Tásk {name}");
        assert_eq!(t("Undo"), "Undo");
    }

    #[test]
    fn locale_tags_and_rtl_detection_handle_hebrew_and_legacy_iw() {
        assert_eq!(Lang::from_tag("en"), Some(Lang::English));
        assert_eq!(Lang::from_tag("en-US"), Some(Lang::English));
        assert_eq!(Lang::from_tag("he"), Some(Lang::Hebrew));
        assert_eq!(Lang::from_tag("he-IL"), Some(Lang::Hebrew));
        assert_eq!(Lang::from_tag("iw"), Some(Lang::Hebrew));
        assert_eq!(Lang::from_tag("iw_IL.UTF-8"), Some(Lang::Hebrew));
        assert_eq!(Lang::from_tag("xx"), None);

        assert!(is_rtl_code("he"));
        assert!(is_rtl_code("he-Hebr-IL"));
        assert!(is_rtl_code("he-u-nu-latn"));
        assert!(is_rtl_code("iw-IL"));
        assert!(is_rtl_code("ku-Arab-IQ"));
        assert!(!is_rtl_code("ku"));
        assert!(!is_rtl_code("ku-Latn-TR"));
        assert!(!is_rtl_code("en-x-hebr"));
        assert!(!is_rtl_code("x-Arab"));
        assert!(!is_rtl_code("x-Hebr"));
        assert!(!is_rtl_code("en"));

        assert_eq!(Lang::Hebrew.code(), "he");
        assert_eq!(Lang::Hebrew.name(), "עברית");
        assert!(Lang::Hebrew.is_rtl());
        assert!(!Lang::English.is_rtl());
    }

    #[test]
    fn hebrew_catalog_covers_every_ui_string_and_keeps_placeholders() {
        let catalog = parsed_hebrew_rows();
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut required = BTreeSet::new();
        for entry in std::fs::read_dir(&src_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let mut rest = text.as_str();
            while let Some(at) = rest.find("tl!(\"") {
                rest = &rest[at + 5..];
                let mut lit = String::new();
                let mut chars = rest.chars();
                let mut esc = false;
                for ch in &mut chars {
                    if esc {
                        match ch {
                            'n' => lit.push('\n'),
                            'r' => lit.push('\r'),
                            't' => lit.push('\t'),
                            other => lit.push(other),
                        }
                        esc = false;
                    } else if ch == '\\' {
                        esc = true;
                    } else if ch == '"' {
                        break;
                    } else {
                        lit.push(ch);
                    }
                }
                required.insert(lit);
                rest = chars.as_str();
            }
        }
        // Dynamic table keys routed through `tl!` in ui-egui.
        for dynamic in [
            // `lib.rs` COLUMNS, View menu, and `DAY_NAMES`, plus engine `WEEKDAYS`
            "Task Name",
            "Duration",
            "Work",
            "Start",
            "Finish",
            "Predecessors",
            "Resource Names",
            "Cost",
            "Fixed Cost",
            "Deadline",
            "Type",
            "Effort Driven",
            "Priority",
            "Leveling Delay",
            "Gantt Chart",
            "Resource Sheet",
            "Resource Usage",
            "Mon",
            "Tue",
            "Wed",
            "Thu",
            "Fri",
            "Sat",
            "Sun",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
            // `timescale.rs` MONTHS and Preset::label
            "Jan",
            "Feb",
            "Mar",
            "Apr",
            "May",
            "Jun",
            "Jul",
            "Aug",
            "Sep",
            "Oct",
            "Nov",
            "Dec",
            "Days",
            "Weeks",
            "Months",
            // `assign.rs` headers & `resource_sheet.rs` COLUMNS / lowercased type names
            "Resource Name",
            "Units",
            "Material",
            "Initials",
            "Group",
            "Max. Units",
            "Std. Rate",
            "Ovt. Rate",
            "Cost/Use",
            "Accrue At",
            "work",
            "material",
            "cost",
            // `options.rs` ROWS labels and units
            "Hours per day",
            "hours",
            "Hours per week",
            "Days per month",
            "days",
            "Currency",
            "ISO code",
            "Currency symbol",
            "Decimal places",
            "in amounts",
            // Model enum display strings: ResourceType, AccrueAt, TaskType, UnitsFormat, Yes/No, Outcome
            "End",
            "Prorated",
            "Fixed Units",
            "Fixed Duration",
            "Fixed Work",
            "Percentage",
            "Decimal",
            "Yes",
            "No",
            "Converted",
            "Dropped",
            "Not yet imported",
            "Dates differ",
        ] {
            required.insert(dynamic.to_owned());
        }
        let missing: Vec<_> = required.iter().filter(|k| !catalog.contains_key(k.as_str())).collect();
        assert!(missing.is_empty(), "missing Hebrew translations in he.tsv: {missing:?}");
        for (k, v) in &catalog {
            assert_eq!(placeholders(k), placeholders(v), "placeholders differ for {k:?}");
            assert_eq!(k.ends_with('…'), v.ends_with('…'), "ellipsis mismatch for {k:?}");
        }
    }

    #[test]
    fn hebrew_translations_and_fmt_reorder_for_rtl() {
        assert_eq!(t_for(Lang::Hebrew, "Save"), "שמירה");
        assert_eq!(t_for(Lang::Hebrew, "New Task"), "חדשה משימה");
        assert_eq!(t_for(Lang::Hebrew, "Assign Resources…"), "…משאבים הקצאת");
        assert_eq!(fmt_for(Lang::Hebrew, t_for(Lang::Hebrew, "Task: {name}"), &[("name", "Alpha")]), "Alpha :משימה");
        assert_eq!(fmt_for(Lang::Hebrew, t_for(Lang::Hebrew, "Task: {name}"), &[("name", "בדיקה ראשונה")]), "ראשונה בדיקה :משימה");
        assert_eq!(
            fmt_for(Lang::Hebrew, t_for(Lang::Hebrew, "Opened {path}"), &[("path", "/tmp/demo.projectcraft")]),
            "/tmp/demo.projectcraft :נפתח"
        );
        assert_eq!(fmt_for(Lang::Hebrew, t_for(Lang::Hebrew, "Opened {path}"), &[("path", ".env")]), ".env :נפתח");
        assert_eq!(visual_rtl("פרויקט ProjectCraft (100%)"), "ProjectCraft (100%) פרויקט");
        assert_eq!(visual_rtl("שמור (Ctrl+O)"), "(Ctrl+O) שמור");
        assert_eq!(visual_rtl("הכול הושלם (100%)"), "(100%) הושלם הכול");
        assert_eq!(t_for(Lang::Hebrew, ""), "");
    }

    #[test]
    fn hebrew_wrapping_reorders_after_line_breaks() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, egui::ThemePreference::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let logical = "המרת משכים בין שעות, ימים, שבועות וחודשים; המטבע של כל סכום; אופן הצגת היחידות";
        let font = egui::FontId::proportional(14.0);
        let wrapped = wrapped_visual_lines(&ctx, Lang::Hebrew, logical, font, 160.0);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert!(lines.len() >= 2, "expected wrapped lines, got {wrapped:?}");
        assert!(lines[0].contains("המרת"), "first wrapped row should begin with opening words: {lines:?}");
        assert!(!lines[0].contains("תרמה"), "characters inside Hebrew words must remain logical: {lines:?}");
        assert!(!lines[0].contains("היחידות"), "first wrapped row should not start with paragraph end: {lines:?}");
        assert!(lines.last().unwrap().contains("היחידות"), "last wrapped row should contain closing words: {lines:?}");
        assert!(!lines.last().unwrap().contains("תודיחיה"), "closing word letters must remain logical: {lines:?}");

        let english_long = "This is a long English-only fallback diagnostic message that should still wrap.";
        let mut english_resp_width = 0.0f32;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.allocate_ui(egui::vec2(140.0, 200.0), |ui| {
                    let resp = wrapped_label_for(ui, Lang::Hebrew, english_long, |r| r);
                    english_resp_width = resp.rect.width();
                });
            });
        })
        .textures_delta
        .clear();
        assert!(english_resp_width <= 150.0, "English fallback in Hebrew mode must still wrap, got width {english_resp_width}");
    }
}
