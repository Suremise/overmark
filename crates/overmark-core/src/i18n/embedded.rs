//! Overmark's strings, compiled into the exe so the portable copy needs no extra files.
//!
//! The only app-specific part of [`super`]. To add a file or a language, see "Strings and
//! languages" in `docs/ARCHITECTURE.md`.

use super::Locale;

/// The language used when a key or a language is missing.
pub const FALLBACK: &str = "en";

macro_rules! ftl {
    ($lang:literal, $file:literal) => {
        (
            $file,
            include_str!(concat!("../../../../locales/", $lang, "/", $file)),
        )
    };
}

pub static LOCALES: &[Locale] = &[Locale {
    tag: "en",
    files: &[
        ftl!("en", "common.ftl"),
        ftl!("en", "tray.ftl"),
        ftl!("en", "toolbar.ftl"),
        ftl!("en", "toasts.ftl"),
        ftl!("en", "settings.ftl"),
        ftl!("en", "hotkeys.ftl"),
        ftl!("en", "errors.ftl"),
        ftl!("en", "installer.ftl"),
    ],
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Localizer;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn english() -> Localizer {
        Localizer::new(LOCALES, FALLBACK, FALLBACK)
    }

    /// The message keys a locale defines (terms, starting with `-`, and comments left out).
    fn keys(locale: &Locale) -> BTreeSet<String> {
        locale
            .files
            .iter()
            .flat_map(|(_, source)| source.lines())
            .filter(|line| line.starts_with(|c: char| c.is_ascii_alphabetic()))
            .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim().to_owned()))
            .collect()
    }

    #[test]
    fn every_file_parses_without_duplicates() {
        for locale in LOCALES {
            let (_, errors) = Localizer::build(std::slice::from_ref(locale), locale.tag, FALLBACK);
            let report: String = errors.iter().map(|e| format!("{e}\n")).collect();
            assert!(errors.is_empty(), "{report}");
        }
    }

    #[test]
    fn every_key_has_a_value() {
        for locale in LOCALES {
            let localizer = Localizer::new(std::slice::from_ref(locale), locale.tag, locale.tag);
            for key in keys(locale) {
                assert!(localizer.has(&key), "{}: {key} has no value", locale.tag);
            }
        }
    }

    /// Git may check the files out with CRLF line ends; multi-line strings mustn't keep the CR.
    #[test]
    fn no_string_carries_a_carriage_return() {
        for locale in LOCALES {
            let localizer = Localizer::new(std::slice::from_ref(locale), locale.tag, locale.tag);
            for key in keys(locale) {
                assert!(!localizer.tr(&key).contains('\r'), "{}: {key}", locale.tag);
            }
        }
    }

    #[test]
    fn every_ftl_file_on_disk_is_embedded() {
        for dir in std::fs::read_dir(workspace().join("locales")).unwrap() {
            let dir = dir.unwrap().path();
            let tag = dir.file_name().unwrap().to_string_lossy().into_owned();
            let locale = LOCALES.iter().find(|l| l.tag == tag);
            assert!(locale.is_some(), "locales/{tag} isn't in LOCALES");
            for file in std::fs::read_dir(&dir).unwrap() {
                let name = file.unwrap().file_name().to_string_lossy().into_owned();
                if name.ends_with(".ftl") {
                    let listed = locale.unwrap().files.iter().any(|(f, _)| *f == name);
                    assert!(listed, "locales/{tag}/{name} isn't in LOCALES");
                }
            }
        }
    }

    #[test]
    fn other_languages_only_translate_keys_english_has() {
        let english = keys(LOCALES.iter().find(|l| l.tag == FALLBACK).unwrap());
        for locale in LOCALES.iter().filter(|l| l.tag != FALLBACK) {
            let extra: Vec<_> = keys(locale).difference(&english).cloned().collect();
            assert!(
                extra.is_empty(),
                "{} has keys English doesn't: {extra:?}",
                locale.tag
            );
        }
    }

    /// Every key the code asks for, as `tr("…")`, `tr_args("…"` or `tr!("…"`, exists in English.
    #[test]
    fn every_key_used_in_the_code_exists() {
        let english = english();
        let mut missing = Vec::new();
        let mut stack = vec![workspace().join("crates")];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if path.extension().is_some_and(|e| e == "rs")
                // This module's own tests use made-up keys.
                && !path.parent().is_some_and(|p| p.ends_with("i18n"))
            {
                let source = std::fs::read_to_string(&path).unwrap();
                for key in used_keys(&source) {
                    if !english.has(&key) {
                        missing.push(format!("{}: {key}", path.display()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing keys:\n{}", missing.join("\n"));
    }

    /// The calls that put text in front of the user: egui's labels, buttons, tooltips and
    /// accessible names, the components' own, window titles, the tray's balloons and the
    /// capture notice, and the console. A word written into one of them is text that bypasses
    /// Fluent; it goes in `locales/en/` instead.
    const UI_CALLS: &[&str] = &[
        ".label(",
        ".button(",
        ".small_button(",
        ".heading(",
        ".link(",
        ".hyperlink_to(",
        ".selectable_label(",
        ".checkbox(",
        ".radio_value(",
        ".toggle_value(",
        "on_hover_text(",
        "on_disabled_hover_text(",
        "accessible_name(",
        "RichText::new(",
        "Label::new(",
        "Button::new(",
        "with_title(",
        "title: Some(",
        "set_title(",
        ".text(",
        "balloon(",
        "Toast::show(",
        "info_bar(",
        "message_bar(",
        "error_text(",
        "quiet_text(",
        "page_title(",
        "section_card(",
        "plain_card(",
        "setting_row(",
        "info_row(",
        "path_row(",
        "primary_button(",
        "secondary_button(",
        "danger_button(",
        "danger_button_filled(",
        "icon_button(",
        "status_chip(",
        "tab_strip(",
        "dropdown(",
        "eprintln!(",
        "println!(",
    ];

    /// The literals with words in them (two letters in a row) that `source` passes straight to a
    /// [`UI_CALLS`] call, before its tests, with the line each is on. A line with
    /// `// not ui text` is skipped (an id, say, that only looks like words).
    fn ui_literals(source: &str) -> Vec<(usize, String)> {
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        let mut found = Vec::new();
        for (n, line) in code.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || line.contains("// not ui text") {
                continue;
            }
            for call in UI_CALLS {
                let mut rest = line;
                while let Some(at) = rest.find(call) {
                    rest = &rest[at + call.len()..];
                    // The call's own arguments on this line: a string literal given straight
                    // to it (not to a `tr(…)` inside it).
                    let args = rest.split(')').next().unwrap_or(rest);
                    let mut at = 0;
                    while let Some(open) = args[at..].find('"') {
                        let open = at + open;
                        let Some(len) = args[open + 1..].find('"') else {
                            break;
                        };
                        let literal = &args[open + 1..open + 1 + len];
                        at = open + len + 2;
                        let before = args[..open].trim_end().trim_end_matches('&').trim_end();
                        let direct = before.is_empty() || before.ends_with(',');
                        let words = literal
                            .as_bytes()
                            .windows(2)
                            .any(|w| w[0].is_ascii_alphabetic() && w[1].is_ascii_alphabetic());
                        // An id (`"tray-click"`): lower case, hyphens, no spaces.
                        let id = literal
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_');
                        if direct && words && !id {
                            found.push((n + 1, literal.to_owned()));
                        }
                    }
                }
            }
        }
        // `println!(` is inside `eprintln!(`: each literal once.
        found.sort();
        found.dedup();
        found
    }

    /// No text is written into the calls that show it: each
    /// crate's code, outside its tests, examples and build scripts, gives them Fluent's words.
    #[test]
    fn no_text_is_written_into_ui_calls() {
        let mut found = Vec::new();
        let mut stack = vec![workspace().join("crates")];
        while let Some(path) = stack.pop() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                // Tests, examples and benches aren't the app.
                if !matches!(name, "tests" | "examples" | "benches" | "target") {
                    stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
                }
            } else if path.extension().is_some_and(|e| e == "rs")
                && !name.ends_with("tests.rs")
                // A build script's `println!`s are instructions to Cargo, not text anyone sees.
                && name != "build.rs"
                && !path.parent().is_some_and(|p| p.ends_with("i18n"))
            {
                let source = std::fs::read_to_string(&path).unwrap();
                for (line, literal) in ui_literals(&source) {
                    found.push(format!("{}:{line}: {literal:?}", path.display()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "text written into UI calls (put it in locales/en/):\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn finds_text_written_into_ui_calls() {
        let source = r#"
ui.label("Hello there");
ui.label(tr("key"));
ui.label("+");
secondary_button(ui, &format!("{}", x));
eprintln!("{}", tr("cli-x"));
eprintln!("Overmark: oops");
tray.balloon(&title, "Body text", kind);
ui.label("Id thing"); // not ui text
Options { title: Some("Overmark notice"), ..Default::default() }
Options { title: Some(window::title("window-notice-name")) }
// ui.label("a comment")
#[cfg(test)]
ui.label("In a test");
"#;
        let found: Vec<String> = ui_literals(source).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            found,
            [
                "Hello there",
                "Overmark: oops",
                "Body text",
                "Overmark notice"
            ]
        );
    }

    /// The literal keys passed to `tr(`, `tr_args(` and `tr!(`. Keys built at run time aren't
    /// seen, so prefer literals.
    fn used_keys(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for call in ["tr(\"", "tr_args(\"", "tr!(\""] {
            let mut rest = source;
            while let Some(start) = rest.find(call) {
                let after_ident = rest[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                rest = &rest[start + call.len()..];
                if let Some(end) = rest.find('"')
                    && !after_ident
                    && !rest[..end].contains(char::is_whitespace)
                {
                    keys.push(rest[..end].to_owned());
                }
            }
        }
        keys
    }

    #[test]
    fn finds_used_keys() {
        let source =
            r#"tr("a-b"); i18n::tr_args("c", &x); tr!("d", job = j); attr("no"); xtr("no")"#;
        let mut found = used_keys(source);
        found.sort();
        assert_eq!(found, ["a-b", "c", "d"]);
    }

    /// The keys the toolbar, tray and settings need.
    #[test]
    fn the_core_strings_exist() {
        let english = english();
        for key in [
            // The tray.
            "tray-tooltip-shortcut",
            "tray-draw",
            "tray-draw-frozen",
            "tray-capture-region",
            "tray-open-captures",
            "tray-settings",
            "tray-start-with-windows",
            "tray-check-updates",
            "tray-about",
            "tray-quit",
            // Tools and their tooltips (§4.2).
            "tool-pen",
            "tool-highlighter",
            "tool-arrow",
            "tool-rectangle",
            "tool-ellipse",
            "tool-line",
            "tool-text",
            "tool-stamp",
            "tool-blur",
            "tool-eraser",
            "tool-laser",
            // The toolbar's buttons (§4.4).
            "toolbar-save",
            "toolbar-save-region",
            "toolbar-copy",
            "toolbar-save-without-drawing",
            "toolbar-freeze",
            "toolbar-unfreeze",
            "toolbar-undo",
            "toolbar-redo",
            "toolbar-clear",
            "toolbar-close",
            "toolbar-collapse",
            // The mode badges (§4.1).
            "mode-live",
            "mode-frozen",
            // Toasts (§4.3).
            "toast-copied-and-saved",
            "toast-saved",
            "toast-copied",
            "toast-save-failed",
            // Settings (§8).
            "settings-general",
            "settings-drawing",
            "settings-capture",
            "settings-about",
            "settings-dim",
            "settings-save-location",
            "settings-third-party",
            "settings-mode-portable",
            "settings-reset",
            // Hotkey conflicts (§6).
            "hotkey-taken",
            "hotkey-taken-replaced",
            "hotkey-notice-title",
            "hotkey-notice-body",
            // The portable folder not writable (§9).
            "error-folder-not-writable-title",
            "error-portable-not-writable",
        ] {
            assert!(english.has(key), "{key}");
        }
        for tool in [
            "pen",
            "highlighter",
            "arrow",
            "rectangle",
            "ellipse",
            "line",
            "text",
            "stamp",
            "blur",
            "eraser",
            "laser",
        ] {
            let key = format!("tool-{tool}-tooltip");
            assert!(english.has(&key), "{key}");
        }
    }

    #[test]
    fn tooltips_carry_their_key() {
        let english = english();
        let tip = english.tr_args(
            "tool-pen-tooltip",
            &crate::i18n::args([("key", "P".into())]),
        );
        assert!(tip.starts_with("Pen (P)"), "{tip}");
        let tip = english.tr_args(
            "tool-rectangle-tooltip",
            &crate::i18n::args([("key", "R".into())]),
        );
        assert_eq!(tip, "Rectangle (R): hold Shift for a square");
        let tip = english.tr_args(
            "tray-tooltip-shortcut",
            &crate::i18n::args([("action", "Draw".into()), ("hotkey", "Win+Shift+D".into())]),
        );
        assert_eq!(tip, "Draw: Win+Shift+D");
    }
}
