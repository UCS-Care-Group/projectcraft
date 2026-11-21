//! The app's settings folder and what's kept in it: the recent-files list (`recent-files.txt`, one
//! path a line), the autosave switch (`autosave.txt`, `off` when turned off), the interface
//! language (`language.txt`, BCP 47 tag), and an untitled project's recovery copy (`recovery/`).

use std::path::PathBuf;

use projectcraft_ui_egui::i18n::Lang;

/// The settings folder: macOS `~/Library/Application Support/ProjectCraft`, Windows
/// `%APPDATA%\ProjectCraft`, otherwise `$XDG_CONFIG_HOME/projectcraft` or `~/.config/projectcraft`.
/// `PROJECTCRAFT_PROFILE_DIR` overrides the whole folder for isolated runs.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PROJECTCRAFT_PROFILE_DIR").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/ProjectCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("ProjectCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("projectcraft"))
    }
}

fn recent_file() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("recent-files.txt"))
}

/// The recent-files list, with files that no longer exist pruned. Empty when there is none or it can't be read.
pub fn load_recent() -> Vec<PathBuf> {
    // Capped read: the file is ours, but a damaged one shouldn't cost memory.
    let mut bytes = Vec::new();
    if let Some(Ok(f)) = recent_file().map(std::fs::File::open) {
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(f, 64 * 1024), &mut bytes);
    }
    let text = String::from_utf8_lossy(&bytes);
    projectcraft_ui_egui::parse_recent(&text).into_iter().filter(|p| p.exists()).collect()
}

/// Writes the recent-files list. A failure only loses the list, so it's reported and otherwise ignored.
pub fn save_recent(list: &[PathBuf]) {
    let Some(file) = recent_file() else {
        return;
    };
    let result = file
        .parent()
        .map_or(Ok(()), projectcraft_engine::create_private_dir_all)
        .and_then(|()| projectcraft_engine::write_private(&file, projectcraft_ui_egui::recent_text(list).as_bytes()));
    if let Err(e) = result {
        eprintln!("couldn't save the recent-files list to {}: {e}", file.display());
    }
}

fn autosave_file() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("autosave.txt"))
}

/// The folder for an untitled project's recovery copy.
pub fn recovery_dir() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("recovery"))
}

/// Whether autosave is on: on unless `autosave.txt` says `off`.
pub fn load_autosave() -> bool {
    let mut bytes = Vec::new();
    if let Some(Ok(f)) = autosave_file().map(std::fs::File::open) {
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(f, 64), &mut bytes);
    }
    String::from_utf8_lossy(&bytes).trim() != "off"
}

/// Writes the autosave switch. A failure only loses the setting, so it's reported and otherwise ignored.
pub fn save_autosave(on: bool) {
    let Some(file) = autosave_file() else {
        return;
    };
    let result = file
        .parent()
        .map_or(Ok(()), projectcraft_engine::create_private_dir_all)
        .and_then(|()| projectcraft_engine::write_private(&file, if on { "on\n" } else { "off\n" }.as_bytes()));
    if let Err(e) = result {
        eprintln!("couldn't save the autosave setting to {}: {e}", file.display());
    }
}

fn language_file() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("language.txt"))
}

fn resolve_language_from_str(saved: Option<&str>, env_tags: &[Option<String>]) -> Lang {
    if let Some(tag) = saved.and_then(Lang::from_tag) {
        return tag;
    }
    for env_val in env_tags.iter().flatten() {
        if let Some(lang) = Lang::from_tag(env_val) {
            return lang;
        }
    }
    Lang::English
}

/// The interface language: `language.txt` if valid, otherwise `PROJECTCRAFT_LANG` / locale env vars.
pub fn load_language() -> Lang {
    let env_tags = ["PROJECTCRAFT_LANG", "LC_ALL", "LC_MESSAGES", "LANG"].map(|k| std::env::var(k).ok());
    load_language_from(language_file().as_deref(), &env_tags)
}

fn load_language_from(file: Option<&std::path::Path>, env_tags: &[Option<String>]) -> Lang {
    let mut bytes = Vec::new();
    let saved = if let Some(Ok(f)) = file.map(std::fs::File::open) {
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(f, 64), &mut bytes);
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        None
    };
    resolve_language_from_str(saved.as_deref(), env_tags)
}

/// Writes the interface language tag. A failure only loses the setting, so it's reported and otherwise ignored.
pub fn save_language(lang: Lang) {
    let Some(file) = language_file() else {
        return;
    };
    save_language_to(&file, lang);
}

fn save_language_to(file: &std::path::Path, lang: Lang) {
    let body = format!("{}\n", lang.code());
    let result = file
        .parent()
        .map_or(Ok(()), projectcraft_engine::create_private_dir_all)
        .and_then(|()| projectcraft_engine::write_private(file, body.as_bytes()));
    if let Err(e) = result {
        eprintln!("couldn't save the language setting to {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_resolution_normalizes_hebrew_and_legacy_iw_tags() {
        assert_eq!(resolve_language_from_str(Some("he\n"), &[]), Lang::Hebrew);
        assert_eq!(resolve_language_from_str(Some("iw_IL.UTF-8"), &[]), Lang::Hebrew);
        assert_eq!(resolve_language_from_str(None, &[None, Some("he_IL.UTF-8".to_owned())]), Lang::Hebrew);
        assert_eq!(resolve_language_from_str(Some("xx"), &[Some("iw".to_owned())]), Lang::Hebrew);
        assert_eq!(resolve_language_from_str(None, &[None, Some("fr_FR.UTF-8".to_owned())]), Lang::English);
    }

    #[test]
    fn language_file_round_trips_hebrew_and_back_to_english() {
        let dir = std::env::temp_dir().join(format!("projectcraft-lang-test-{}", std::process::id()));
        let file = dir.join("language.txt");
        let _ = std::fs::remove_dir_all(&dir);

        save_language_to(&file, Lang::Hebrew);
        assert_eq!(load_language_from(Some(&file), &[]), Lang::Hebrew);

        save_language_to(&file, Lang::English);
        assert_eq!(load_language_from(Some(&file), &[]), Lang::English);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
