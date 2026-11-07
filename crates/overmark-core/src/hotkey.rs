//! Hotkeys as the settings file writes them ("Win+Shift+D"): parsed, checked against the
//! hotkey rules and shown with the key names from the strings. Platform-free; the Windows
//! layer maps a [`Hotkey`] onto `RegisterHotKey`.

use std::fmt;

/// A key a hotkey can end with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A to Z (stored upper case).
    Letter(char),
    /// 0 to 9 on the main row.
    Digit(u8),
    /// F1 to F24.
    F(u8),
    PrintScreen,
    Space,
    Tab,
    Enter,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
}

/// A hotkey: modifiers and one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

/// Why a hotkey's text can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// Nothing but spaces.
    Empty,
    /// A part that isn't a modifier or a key this module knows.
    UnknownKey(String),
    /// More than one key, or none after the modifiers.
    NotOneKey,
    /// A key without Ctrl, Alt, Shift or Win (Print Screen alone is allowed).
    NeedsModifier,
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotkeyError::Empty => write!(f, "no hotkey"),
            HotkeyError::UnknownKey(k) => write!(f, "{k:?} isn't a key"),
            HotkeyError::NotOneKey => write!(f, "a hotkey is modifiers and one key"),
            HotkeyError::NeedsModifier => write!(f, "a key needs Ctrl, Alt, Shift or Win"),
        }
    }
}

impl std::error::Error for HotkeyError {}

fn key_named(name: &str) -> Option<Key> {
    let upper = name.to_ascii_uppercase();
    let mut chars = upper.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_uppercase() {
            return Some(Key::Letter(c));
        }
        if c.is_ascii_digit() {
            return Some(Key::Digit(c as u8 - b'0'));
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&n)
    {
        return Some(Key::F(n));
    }
    Some(match upper.as_str() {
        "PRINTSCREEN" | "PRINT SCREEN" | "PRTSC" | "PRTSCN" => Key::PrintScreen,
        "SPACE" => Key::Space,
        "TAB" => Key::Tab,
        "ENTER" | "RETURN" => Key::Enter,
        "DELETE" | "DEL" => Key::Delete,
        "INSERT" | "INS" => Key::Insert,
        "HOME" => Key::Home,
        "END" => Key::End,
        "PAGEUP" | "PGUP" => Key::PageUp,
        "PAGEDOWN" | "PGDN" => Key::PageDown,
        "LEFT" => Key::Left,
        "RIGHT" => Key::Right,
        "UP" => Key::Up,
        "DOWN" => Key::Down,
        _ => return None,
    })
}

impl Hotkey {
    /// Parses "Win+Shift+D" (any case, spaces around the parts ignored). `Ok(None)` for an
    /// empty text: no hotkey.
    pub fn parse(text: &str) -> Result<Option<Hotkey>, HotkeyError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "" => return Err(HotkeyError::NotOneKey),
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "win" | "windows" | "super" => win = true,
                _ => {
                    let k =
                        key_named(part).ok_or_else(|| HotkeyError::UnknownKey(part.to_owned()))?;
                    if key.replace(k).is_some() {
                        return Err(HotkeyError::NotOneKey);
                    }
                }
            }
        }
        let key = key.ok_or(HotkeyError::NotOneKey)?;
        if !(ctrl || alt || shift || win) && key != Key::PrintScreen {
            return Err(HotkeyError::NeedsModifier);
        }
        Ok(Some(Hotkey {
            ctrl,
            alt,
            shift,
            win,
            key,
        }))
    }

    /// Ctrl+Alt is AltGr on many keyboards, which types characters: worth a warning.
    pub fn is_ctrl_alt(&self) -> bool {
        self.ctrl && self.alt
    }

    /// The key's name for showing, from `strings`: a letter, digit or F key as itself (they're
    /// the keys' own identities), every other key a `key-*` message.
    fn key_text(&self, strings: &crate::i18n::Localizer) -> String {
        let named = match self.key {
            Key::Letter(c) => return c.to_string(),
            Key::Digit(d) => return d.to_string(),
            Key::F(n) => return format!("F{n}"),
            Key::PrintScreen => "key-print-screen",
            Key::Space => "key-space",
            Key::Tab => "key-tab",
            Key::Enter => "key-enter",
            Key::Delete => "key-delete",
            Key::Insert => "key-insert",
            Key::Home => "key-home",
            Key::End => "key-end",
            Key::PageUp => "key-page-up",
            Key::PageDown => "key-page-down",
            Key::Left => "key-left",
            Key::Right => "key-right",
            Key::Up => "key-up",
            Key::Down => "key-down",
        };
        strings.tr(named)
    }

    /// The hotkey as people see it, with the strings' key names joined by "+", in Windows'
    /// order: Win, Ctrl, Alt, Shift ("Win+Shift+S", "Ctrl+Shift+Esc").
    pub fn display(&self) -> String {
        crate::i18n::with_current(|strings| self.display_in(strings))
    }

    /// [`Hotkey::display`] in the language of `strings`.
    pub fn display_in(&self, strings: &crate::i18n::Localizer) -> String {
        let mut parts = Vec::new();
        for (on, key) in [
            (self.win, "key-win"),
            (self.ctrl, "key-ctrl"),
            (self.alt, "key-alt"),
            (self.shift, "key-shift"),
        ] {
            if on {
                parts.push(strings.tr(key));
            }
        }
        parts.push(self.key_text(strings));
        parts.join("+")
    }
}

impl Key {
    /// Every key a hotkey can end with, in the order Settings lists them: letters, digits,
    /// F keys, then the named keys.
    pub fn all() -> Vec<Key> {
        let mut keys: Vec<Key> = ('A'..='Z').map(Key::Letter).collect();
        keys.extend((0..=9).map(Key::Digit));
        keys.extend((1..=24).map(Key::F));
        keys.extend([
            Key::PrintScreen,
            Key::Space,
            Key::Tab,
            Key::Enter,
            Key::Delete,
            Key::Insert,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
        ]);
        keys
    }

    /// The key's name in the settings file: what [`Hotkey::parse`] reads, in English whatever
    /// the language.
    pub fn setting_name(self) -> String {
        match self {
            Key::Letter(c) => c.to_string(),
            Key::Digit(d) => d.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::PrintScreen => "Print Screen".to_owned(),
            Key::Space => "Space".to_owned(),
            Key::Tab => "Tab".to_owned(),
            Key::Enter => "Enter".to_owned(),
            Key::Delete => "Delete".to_owned(),
            Key::Insert => "Insert".to_owned(),
            Key::Home => "Home".to_owned(),
            Key::End => "End".to_owned(),
            Key::PageUp => "PageUp".to_owned(),
            Key::PageDown => "PageDown".to_owned(),
            Key::Left => "Left".to_owned(),
            Key::Right => "Right".to_owned(),
            Key::Up => "Up".to_owned(),
            Key::Down => "Down".to_owned(),
        }
    }

    /// The key's name for showing alone (in Settings' list of keys), from the strings.
    pub fn display(self) -> String {
        Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: self,
        }
        .display()
    }
}

/// The modifier sets [`Hotkey::alternatives`] tries, nearest first, as (ctrl, alt, shift,
/// win). Never Ctrl+Alt, which is AltGr on many keyboards.
const MODIFIER_SETS: [(bool, bool, bool, bool); 6] = [
    (false, false, true, true),
    (false, true, false, true),
    (true, false, false, true),
    (true, false, true, false),
    (false, true, true, false),
    (true, false, true, true),
];

impl Hotkey {
    /// The hotkey as the settings file writes it ("Win+Shift+D"): English, in Windows' order,
    /// whatever the language. [`Hotkey::parse`] reads it back.
    pub fn to_setting(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (on, name) in [
            (self.win, "Win"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if on {
                parts.push(name.to_owned());
            }
        }
        parts.push(self.key.setting_name());
        parts.join("+")
    }

    /// Hotkeys near this one, nearest first, to suggest when it's taken: the same
    /// key with the other usual modifiers, then the keys beside it (a letter's neighbours in
    /// the alphabet, a digit's or an F key's numbers) with this one's modifiers, then with the
    /// others. Never Ctrl+Alt, never this hotkey itself, and none twice.
    pub fn alternatives(&self) -> Vec<Hotkey> {
        let with = |key: Key, (ctrl, alt, shift, win): (bool, bool, bool, bool)| Hotkey {
            ctrl,
            alt,
            shift,
            win,
            key,
        };
        let own = (self.ctrl, self.alt, self.shift, self.win);
        let mut sets = vec![own];
        sets.extend(MODIFIER_SETS.iter().copied().filter(|m| *m != own));
        let neighbours: Vec<Key> = match self.key {
            Key::Letter(c) => (1..=25u8)
                .flat_map(|d| [(c as u8).checked_add(d), (c as u8).checked_sub(d)])
                .flatten()
                .filter(u8::is_ascii_uppercase)
                .map(|b| Key::Letter(b as char))
                .collect(),
            Key::Digit(n) => (1..=9u8)
                .flat_map(|d| [n.checked_add(d), n.checked_sub(d)])
                .flatten()
                .filter(|v| *v <= 9)
                .map(Key::Digit)
                .collect(),
            Key::F(n) => (1..=23u8)
                .flat_map(|d| [n.checked_add(d), n.checked_sub(d)])
                .flatten()
                .filter(|v| (1..=24).contains(v))
                .map(Key::F)
                .collect(),
            _ => Vec::new(),
        };
        let mut out: Vec<Hotkey> = Vec::new();
        let mut push = |h: Hotkey| {
            let alone = !(h.ctrl || h.alt || h.shift || h.win);
            let allowed = !alone || h.key == Key::PrintScreen;
            if h != *self && allowed && !h.is_ctrl_alt() && !out.contains(&h) {
                out.push(h);
            }
        };
        for m in &sets {
            push(with(self.key, *m));
        }
        for key in &neighbours {
            push(with(*key, own));
        }
        for key in &neighbours {
            for m in &sets {
                push(with(*key, *m));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_text_reads_back_in_english() {
        for text in [
            "Win+Shift+D",
            "Ctrl+Shift+F12",
            "Print Screen",
            "Win+Alt+PageDown",
            "Win+Ctrl+Alt+Shift+7",
        ] {
            let h = Hotkey::parse(text).unwrap().unwrap();
            assert_eq!(h.to_setting(), text);
        }
        assert_eq!(
            Hotkey::parse("shift+win+d").unwrap().unwrap().to_setting(),
            "Win+Shift+D"
        );
        for key in Key::all() {
            let h = Hotkey {
                ctrl: false,
                alt: false,
                shift: true,
                win: true,
                key,
            };
            assert_eq!(Hotkey::parse(&h.to_setting()), Ok(Some(h)), "{key:?}");
        }
    }

    #[test]
    fn alternatives_start_near_and_never_use_ctrl_alt() {
        let d = Hotkey::parse("Win+Shift+D").unwrap().unwrap();
        let alts = d.alternatives();
        let first: Vec<String> = alts.iter().take(5).map(Hotkey::to_setting).collect();
        assert_eq!(
            first,
            [
                "Win+Alt+D",
                "Win+Ctrl+D",
                "Ctrl+Shift+D",
                "Alt+Shift+D",
                "Win+Ctrl+Shift+D"
            ]
        );
        // Then the letters beside it, with its own modifiers.
        let next: Vec<String> = alts[5..9].iter().map(Hotkey::to_setting).collect();
        assert_eq!(
            next,
            ["Win+Shift+E", "Win+Shift+C", "Win+Shift+F", "Win+Shift+B"]
        );
        assert!(!alts.contains(&d));
        assert!(alts.iter().all(|h| !h.is_ctrl_alt()));
        for (i, h) in alts.iter().enumerate() {
            assert!(!alts[..i].contains(h), "{} twice", h.to_setting());
            assert_eq!(Hotkey::parse(&h.to_setting()), Ok(Some(*h)));
        }
        // A key at the end of its range still gets neighbours.
        let f24 = Hotkey::parse("Ctrl+Shift+F24").unwrap().unwrap();
        assert!(f24.alternatives().iter().any(|h| h.key == Key::F(23)));
        let z = Hotkey::parse("Win+Shift+Z").unwrap().unwrap();
        assert!(z.alternatives().iter().any(|h| h.key == Key::Letter('Y')));
        // Print Screen alone: the others are it with modifiers.
        let prt = Hotkey::parse("Print Screen").unwrap().unwrap();
        assert_eq!(prt.alternatives()[0].to_setting(), "Win+Shift+Print Screen");
    }

    fn hk(ctrl: bool, alt: bool, shift: bool, win: bool, key: Key) -> Option<Hotkey> {
        Some(Hotkey {
            ctrl,
            alt,
            shift,
            win,
            key,
        })
    }

    #[test]
    fn parses_the_defaults_in_any_case_and_spacing() {
        assert_eq!(
            Hotkey::parse("Win+Shift+D"),
            Ok(hk(false, false, true, true, Key::Letter('D')))
        );
        assert_eq!(
            Hotkey::parse(" win + shift + f "),
            Ok(hk(false, false, true, true, Key::Letter('F')))
        );
        assert_eq!(
            Hotkey::parse("Ctrl+Alt+F12"),
            Ok(hk(true, true, false, false, Key::F(12)))
        );
        assert_eq!(
            Hotkey::parse("Shift+7"),
            Ok(hk(false, false, true, false, Key::Digit(7)))
        );
    }

    #[test]
    fn empty_means_none_and_print_screen_may_stand_alone() {
        assert_eq!(Hotkey::parse(""), Ok(None));
        assert_eq!(Hotkey::parse("   "), Ok(None));
        assert_eq!(
            Hotkey::parse("Print Screen"),
            Ok(hk(false, false, false, false, Key::PrintScreen))
        );
    }

    #[test]
    fn rejects_what_cant_be_a_hotkey() {
        assert_eq!(Hotkey::parse("D"), Err(HotkeyError::NeedsModifier));
        assert_eq!(Hotkey::parse("Win+Shift"), Err(HotkeyError::NotOneKey));
        assert_eq!(Hotkey::parse("Win+D+F"), Err(HotkeyError::NotOneKey));
        assert_eq!(Hotkey::parse("Win++D"), Err(HotkeyError::NotOneKey));
        assert_eq!(
            Hotkey::parse("Win+Banana"),
            Err(HotkeyError::UnknownKey("Banana".to_owned()))
        );
        assert_eq!(
            Hotkey::parse("Win+F25"),
            Err(HotkeyError::UnknownKey("F25".to_owned()))
        );
    }

    #[test]
    fn shows_with_the_key_names_and_flags_ctrl_alt() {
        let draw = Hotkey::parse("shift+win+d").unwrap().unwrap();
        assert_eq!(draw.display(), "Win+Shift+D");
        assert!(!draw.is_ctrl_alt());
        let altgr = Hotkey::parse("Alt+Ctrl+P").unwrap().unwrap();
        assert_eq!(altgr.display(), "Ctrl+Alt+P");
        assert!(altgr.is_ctrl_alt());
    }

    /// Every named key is shown from the strings: a language's own names come through.
    #[test]
    fn named_keys_come_from_the_strings() {
        use crate::i18n::{Locale, Localizer};
        const NAMES: [(&str, &str); 18] = [
            ("key-win", "Gagner"),
            ("key-ctrl", "Contrôle"),
            ("key-alt", "Alterne"),
            ("key-shift", "Maj"),
            ("key-print-screen", "Impr écran"),
            ("key-space", "Espace"),
            ("key-tab", "Tabulation"),
            ("key-enter", "Entrée"),
            ("key-delete", "Suppr"),
            ("key-insert", "Inser"),
            ("key-home", "Début"),
            ("key-end", "Fin"),
            ("key-page-up", "Page préc"),
            ("key-page-down", "Page suiv"),
            ("key-left", "Gauche"),
            ("key-right", "Droite"),
            ("key-up", "Haut"),
            ("key-down", "Bas"),
        ];
        let ftl: &'static str = NAMES
            .iter()
            .map(|(k, v)| format!("{k} = {v}\n"))
            .collect::<String>()
            .leak();
        let files: &'static [(&str, &str)] = vec![("common.ftl", ftl)].leak();
        let locales = [Locale {
            tag: "fr-FR",
            files,
        }];
        let fr = Localizer::new(&locales, "fr-FR", "fr-FR");
        let shown = |text: &str| Hotkey::parse(text).unwrap().unwrap().display_in(&fr);
        assert_eq!(shown("Ctrl+Home"), "Contrôle+Début");
        assert_eq!(shown("Win+Shift+PageDown"), "Gagner+Maj+Page suiv");
        for (text, want) in [
            ("Win+Insert", "Inser"),
            ("Win+End", "Fin"),
            ("Win+PageUp", "Page préc"),
            ("Win+Left", "Gauche"),
            ("Win+Right", "Droite"),
            ("Win+Up", "Haut"),
            ("Win+Down", "Bas"),
            ("Win+Space", "Espace"),
            ("Win+Tab", "Tabulation"),
            ("Win+Enter", "Entrée"),
            ("Win+Delete", "Suppr"),
            ("Print Screen", "Impr écran"),
        ] {
            assert!(shown(text).ends_with(want), "{text}: {}", shown(text));
        }
        // Letters, digits and F keys are the keys themselves.
        assert_eq!(shown("Alt+Q"), "Alterne+Q");
        assert_eq!(shown("Shift+F7"), "Maj+F7");
    }

    /// The English strings have every key name the display uses.
    #[test]
    fn english_has_every_key_name() {
        for name in [
            "key-insert",
            "key-home",
            "key-end",
            "key-page-up",
            "key-page-down",
            "key-left",
            "key-right",
            "key-up",
            "key-down",
            "key-print-screen",
            "key-space",
            "key-tab",
            "key-enter",
            "key-delete",
        ] {
            assert!(crate::i18n::has(name), "{name}");
        }
    }

    #[test]
    fn every_default_hotkey_parses() {
        let s = crate::settings::Settings::default();
        for text in [
            &s.general.hotkey_draw,
            &s.general.hotkey_draw_frozen,
            &s.general.hotkey_capture_region,
            &s.general.hotkey_record_region,
        ] {
            assert!(matches!(Hotkey::parse(text), Ok(Some(_))), "{text}");
        }
    }
}
