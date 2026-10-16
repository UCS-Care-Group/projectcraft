//! Interface text. Every string the interface shows goes through [`t`] (the `tl!` macro), keyed by
//! its English text, so language catalogs can be added without touching the widgets. English is
//! the only language so far; `Pseudo` (a test language) shows which strings skip the lookup.
//! As MailCraft and WordCraft.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    /// Accented look-alikes of the English text, for finding strings that skip translation.
    Pseudo,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::Pseudo as u8 { Lang::Pseudo } else { Lang::English }
}

/// `s` in the interface language.
pub fn t(s: &str) -> &str {
    match current() {
        Lang::English => s,
        Lang::Pseudo => pseudo(s),
    }
}

/// The pseudo-language form of `s`, interned so it can be handed out as `&'static str`.
fn pseudo(s: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
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

/// Fill `{name}` placeholders in a (translated) template.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_pseudo() {
        assert_eq!(fmt("Start {start} · Finish {finish}", &[("start", "Mon"), ("finish", "Fri")]), "Start Mon · Finish Fri");
        assert_eq!(pseudo("Task {name}"), "Tásk {name}");
        assert_eq!(t("Undo"), "Undo");
    }
}
