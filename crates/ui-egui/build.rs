//! Optional build-time discovery of `NotoSansHebrew-Regular.ttf` in `craft-fonts`.
//! Builds without `CRAFT_FONTS_DIR` succeed cleanly and use desktop system font fallbacks.

use std::path::{Path, PathBuf};

fn find_fonts_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors().skip(1) {
        let sibling = ancestor.join("craft-fonts");
        if sibling.is_dir() {
            return Some(sibling);
        }
        if ancestor.file_name().is_some_and(|n| n == "projectcraft") {
            if let Some(parent) = ancestor.parent() {
                let near = parent.join("craft-fonts");
                if near.is_dir() {
                    return Some(near);
                }
            }
            break;
        }
    }
    None
}

fn candidate_hebrew_font(dir: &Path) -> Option<PathBuf> {
    let p = dir.join("fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf");
    p.is_file().then_some(p)
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_hebrew_craft_font)");
    println!("cargo:rerun-if-env-changed=CRAFT_FONTS_DIR");
    if let Some(dir) = find_fonts_dir()
        && let Some(font) = candidate_hebrew_font(&dir)
    {
        println!("cargo:rerun-if-changed={}", font.display());
        println!("cargo:rustc-cfg=has_hebrew_craft_font");
        println!("cargo:rustc-env=HEBREW_CRAFT_FONT_PATH={}", font.display());
    }
}
