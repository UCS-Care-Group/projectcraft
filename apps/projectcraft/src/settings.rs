//! The app's settings folder and what's kept in it: the recent-files list (`recent-files.txt`, one
//! path a line), the autosave switch (`autosave.txt`, `off` when turned off) and an untitled
//! project's recovery copy (`recovery/`).

use std::path::PathBuf;

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
