//! The Text tool's and the stamps' fonts: Windows' own, read from `%WINDIR%\Fonts` at run time
//! and handed to core (`overmark_core::text::set_fonts`); nothing is bundled, so there's
//! nothing to license (docs/ARCHITECTURE.md, Text: the font). The files are
//! Replica's list (`replica/src/ui/fonts.rs`): Segoe UI Variable (Windows 11), else Segoe UI,
//! with Segoe UI Symbol after them for the symbols they lack.

use std::path::{Path, PathBuf};

/// The text fonts, in the order core falls back through them.
pub const FILES: &[&str] = &["SegUIVar.ttf", "segoeui.ttf", "seguisym.ttf"];

/// Windows' fonts folder (`%WINDIR%\Fonts`).
pub fn fonts_dir() -> PathBuf {
    let windows = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&windows).join("Fonts")
}

/// The bytes of each of [`FILES`] in `dir` that's there.
pub fn read(dir: &Path) -> Vec<Vec<u8>> {
    FILES
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok())
        .collect()
}

/// Gives core the system fonts, once per process (they stay loaded). Returns whether core has
/// fonts now; without them, text has no glyphs and a stamp shows its disc alone.
pub fn ensure_loaded() -> bool {
    use crate::core::text;
    if text::has_fonts() {
        return true;
    }
    let files = read(&fonts_dir());
    if files.is_empty() {
        tracing::warn!(dir = %fonts_dir().display(), "no text font found");
        return false;
    }
    let count = files.len();
    match text::set_fonts(files) {
        Ok(()) => {
            tracing::info!(count, "text fonts loaded");
            true
        }
        Err(error) => {
            tracing::warn!(%error, "the text fonts couldn't be loaded");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_has_a_text_font() {
        assert!(!super::read(&super::fonts_dir()).is_empty());
    }
}
