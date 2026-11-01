//! Capture file names: a pattern of date and time codes, turned into a name Windows accepts.
//!
//! **The pattern syntax.** Text is kept as it is, and a code in braces is replaced by part of
//! the capture's local time, zero-padded:
//!
//! | Code | Is | Example |
//! |---|---|---|
//! | `{year}` | four-digit year | `2026` |
//! | `{yy}` | two-digit year | `26` |
//! | `{month}` | month, 01–12 | `10` |
//! | `{day}` | day of the month, 01–31 | `21` |
//! | `{hour}` | hour, 00–23 | `09` |
//! | `{minute}` | minute, 00–59 | `42` |
//! | `{second}` | second, 00–59 | `17` |
//! | `{millisecond}` | millisecond, 000–999 | `250` |
//!
//! `{{` and `}}` stand for a brace itself. The extension (`.png`, `.jpg`) is added after the
//! pattern, so it isn't part of it. The default, [`DEFAULT_PATTERN`], gives
//! `2026-10-21 09.42.17.png`.
//!
//! **Safe for Windows.** Characters Windows doesn't allow in a name (`< > : " / \ | ? *` and
//! control characters) become `_`, so a pattern can't make folders; trailing dots and spaces
//! are removed; a device name such as `CON` or `LPT1` gets a `_` after it; a name is at most
//! [`MAX_STEM_UTF16`] UTF-16 units (the unit Windows limits a name's length in), cut between
//! characters; and an empty one becomes `Overmark`.
//!
//! **Clashes.** If the name is taken, ` (2)`, ` (3)` and so on go before the extension, as
//! Explorer does.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::tr;

/// The default pattern: `2026-10-21 09.42.17`.
pub const DEFAULT_PATTERN: &str = "{year}-{month}-{day} {hour}.{minute}.{second}";

/// The longest a name may be before its clash number and extension, in UTF-16 code units: the
/// unit Windows limits a name to 255 of, where an emoji takes two. It leaves room for a clash
/// number up to ` (99999999)` and an extension of up to 40 units.
pub const MAX_STEM_UTF16: usize = 200;

/// The name used when a pattern gives nothing usable.
const FALLBACK_STEM: &str = crate::APP_NAME;

/// A capture's local time, as the platform reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTime {
    pub year: i32,
    /// 1–12.
    pub month: u8,
    /// 1–31.
    pub day: u8,
    /// 0–23.
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}

/// What's wrong with a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// A code that isn't in the table, such as `{hours}`.
    UnknownCode(String),
    /// A `{` with no `}` after it.
    Unclosed,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            PatternError::UnknownCode(code) => {
                tr!("pattern-unknown-code", code = format!("{{{code}}}"))
            }
            PatternError::Unclosed => tr!("pattern-unclosed"),
        })
    }
}

impl std::error::Error for PatternError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Code {
    Year,
    Yy,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

impl Code {
    fn parse(name: &str) -> Option<Code> {
        Some(match name {
            "year" => Code::Year,
            "yy" => Code::Yy,
            "month" => Code::Month,
            "day" => Code::Day,
            "hour" => Code::Hour,
            "minute" => Code::Minute,
            "second" => Code::Second,
            "millisecond" => Code::Millisecond,
            _ => return None,
        })
    }

    fn write(self, time: &CaptureTime, out: &mut String) {
        use std::fmt::Write as _;
        let _ = match self {
            Code::Year => write!(out, "{:04}", time.year),
            Code::Yy => write!(out, "{:02}", time.year.rem_euclid(100)),
            Code::Month => write!(out, "{:02}", time.month),
            Code::Day => write!(out, "{:02}", time.day),
            Code::Hour => write!(out, "{:02}", time.hour),
            Code::Minute => write!(out, "{:02}", time.minute),
            Code::Second => write!(out, "{:02}", time.second),
            Code::Millisecond => write!(out, "{:03}", time.millisecond),
        };
    }
}

/// Checks a pattern without using it.
pub fn check_pattern(pattern: &str) -> Result<(), PatternError> {
    expand(pattern, &CaptureTime::EXAMPLE).map(drop)
}

impl CaptureTime {
    /// 21 Oct 2026, 09:42:17.250: the time the docs' examples use.
    pub const EXAMPLE: CaptureTime = CaptureTime {
        year: 2026,
        month: 10,
        day: 21,
        hour: 9,
        minute: 42,
        second: 17,
        millisecond: 250,
    };
}

/// The pattern with its codes replaced, before it's made safe.
pub fn expand(pattern: &str, time: &CaptureTime) -> Result<String, PatternError> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let brace = rest.as_bytes()[i];
        rest = &rest[i + 1..];
        if brace == b'}' {
            // `}}` is a brace; a lone `}` is kept too, as nothing else could be meant.
            out.push('}');
            rest = rest.strip_prefix('}').unwrap_or(rest);
            continue;
        }
        if let Some(after) = rest.strip_prefix('{') {
            out.push('{');
            rest = after;
            continue;
        }
        let end = rest.find('}').ok_or(PatternError::Unclosed)?;
        let name = &rest[..end];
        let code = Code::parse(name).ok_or_else(|| PatternError::UnknownCode(name.to_owned()))?;
        code.write(time, &mut out);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `name` made safe as a Windows file name (without its extension): see the module's docs.
pub fn safe_stem(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .scan(0, |units, c| {
            *units += c.len_utf16();
            (*units <= MAX_STEM_UTF16).then_some(c)
        })
        .collect();
    let mut stem = replaced
        .trim_start()
        .trim_end_matches(['.', ' '])
        .to_owned();
    if stem.is_empty() {
        return FALLBACK_STEM.to_owned();
    }
    if is_device_name(&stem) {
        // Windows reads `AUX.backup` as the device too, so the mark goes after the base name.
        let base = stem.find('.').unwrap_or(stem.len());
        stem.insert(base, '_');
    }
    stem
}

/// Whether Windows reserves `stem` (or the part before its first dot) for a device.
fn is_device_name(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).trim_end();
    let upper = base.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// The capture's file name: `pattern` at `time`, made safe, with `extension` (`"png"`), and a
/// ` (2)`, ` (3)`, … if `taken` says the name is in use.
pub fn file_name(
    pattern: &str,
    time: &CaptureTime,
    extension: &str,
    taken: impl Fn(&str) -> bool,
) -> Result<String, PatternError> {
    let stem = safe_stem(&expand(pattern, time)?);
    let name = |n: u32| match n {
        1 => format!("{stem}.{extension}"),
        n => format!("{stem} ({n}).{extension}"),
    };
    let mut n = 1;
    while taken(&name(n)) {
        n += 1;
    }
    Ok(name(n))
}

/// The capture's path in `folder`, its name from [`file_name`], with clashes checked against
/// the files there. Another program could take the name before it's written, so create the file
/// with `create_new` and ask again if that fails.
pub fn capture_path(
    folder: &Path,
    pattern: &str,
    time: &CaptureTime,
    extension: &str,
) -> Result<PathBuf, PatternError> {
    let name = file_name(pattern, time, extension, |name| {
        folder.join(name).symlink_metadata().is_ok()
    })?;
    Ok(folder.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const T: CaptureTime = CaptureTime::EXAMPLE;

    fn free(_: &str) -> bool {
        false
    }

    #[test]
    fn the_default_name() {
        assert_eq!(
            file_name(DEFAULT_PATTERN, &T, "png", free).unwrap(),
            "2026-10-21 09.42.17.png"
        );
    }

    #[test]
    fn every_code_expands_zero_padded() {
        let time = CaptureTime {
            year: 2007,
            month: 3,
            day: 4,
            hour: 5,
            minute: 6,
            second: 7,
            millisecond: 8,
        };
        assert_eq!(
            expand(
                "{year}|{yy}|{month}|{day}|{hour}|{minute}|{second}|{millisecond}",
                &time
            )
            .unwrap(),
            "2007|07|03|04|05|06|07|008"
        );
    }

    #[test]
    fn braces_escape_and_bad_codes_fail() {
        assert_eq!(expand("{{year}} {year}", &T).unwrap(), "{year} 2026");
        assert_eq!(expand("a}b", &T).unwrap(), "a}b");
        assert_eq!(
            expand("{hours}", &T),
            Err(PatternError::UnknownCode("hours".into()))
        );
        assert_eq!(
            expand("{Year}", &T),
            Err(PatternError::UnknownCode("Year".into()))
        );
        assert_eq!(expand("shot {year", &T), Err(PatternError::Unclosed));
        assert!(check_pattern(DEFAULT_PATTERN).is_ok());
        assert!(check_pattern("{nope}").is_err());
    }

    #[test]
    fn errors_read_as_text() {
        let text = PatternError::UnknownCode("hours".into()).to_string();
        assert!(text.contains("{hours}"), "{text}");
        assert!(!PatternError::Unclosed.to_string().starts_with("pattern-"));
    }

    #[test]
    fn invalid_characters_become_underscores() {
        assert_eq!(
            file_name("a<b>c:d\"e/f\\g|h?i*j\u{7}k", &T, "png", free).unwrap(),
            "a_b_c_d_e_f_g_h_i_j_k.png"
        );
        // A pattern with a colon in the time, as people often write.
        assert_eq!(
            file_name("{hour}:{minute}", &T, "jpg", free).unwrap(),
            "09_42.jpg"
        );
    }

    #[test]
    fn trailing_dots_and_spaces_go_and_empty_falls_back() {
        assert_eq!(safe_stem("  shot. . "), "shot");
        assert_eq!(safe_stem(""), "Overmark");
        assert_eq!(safe_stem(" ..."), "Overmark");
    }

    #[test]
    fn device_names_are_changed() {
        for name in ["CON", "con", "Nul", "COM1", "lpt9", "AUX.backup", "COM²"] {
            let safe = safe_stem(name);
            assert!(!is_device_name(&safe), "{name} -> {safe}");
        }
        assert_eq!(safe_stem("COM10"), "COM10");
        assert_eq!(safe_stem("Console"), "Console");
    }

    #[test]
    fn long_names_are_cut() {
        let long = "x".repeat(500);
        assert_eq!(safe_stem(&long).chars().count(), MAX_STEM_UTF16);
    }

    fn utf16_len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    /// Windows counts a name's length in UTF-16 units, where an emoji takes two.
    #[test]
    fn long_names_are_cut_by_utf16_units() {
        let emoji = "\u{1F600}".repeat(200);
        let stem = safe_stem(&emoji);
        assert_eq!(utf16_len(&stem), MAX_STEM_UTF16);
        assert_eq!(stem.chars().count(), MAX_STEM_UTF16 / 2);

        // An emoji that would straddle the limit is left out whole.
        let mixed = format!("{}{}", "a".repeat(199), "\u{1F600}".repeat(5));
        assert_eq!(safe_stem(&mixed), "a".repeat(199));

        // The longest stem with the largest clash number and extension allowed fits in 255.
        let worst = format!("{stem} (99999999).{}", "x".repeat(40));
        assert!(utf16_len(&worst) <= 255, "{}", utf16_len(&worst));
    }

    /// A pattern of 200 emoji, saved for real, twice. Windows only: the name
    /// fits Windows' 255 UTF-16 units, but Linux file systems count 255 bytes, which it passes.
    #[cfg(windows)]
    #[test]
    fn an_emoji_pattern_can_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let pattern = "\u{1F600}".repeat(200);
        let first = capture_path(dir.path(), &pattern, &T, "png").unwrap();
        std::fs::write(&first, b"").unwrap();
        let second = capture_path(dir.path(), &pattern, &T, "png").unwrap();
        assert!(second.to_string_lossy().ends_with(" (2).png"), "{second:?}");
        std::fs::write(&second, b"").unwrap();
    }

    #[test]
    fn clashes_get_a_number_before_the_extension() {
        let taken: HashSet<&str> = ["2026-10-21 09.42.17.png", "2026-10-21 09.42.17 (2).png"]
            .into_iter()
            .collect();
        assert_eq!(
            file_name(DEFAULT_PATTERN, &T, "png", |n| taken.contains(n)).unwrap(),
            "2026-10-21 09.42.17 (3).png"
        );
        // A JPG doesn't clash with a PNG of the same time.
        assert_eq!(
            file_name(DEFAULT_PATTERN, &T, "jpg", |n| taken.contains(n)).unwrap(),
            "2026-10-21 09.42.17.jpg"
        );
    }

    #[test]
    fn capture_path_checks_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let first = capture_path(dir.path(), DEFAULT_PATTERN, &T, "png").unwrap();
        assert_eq!(first, dir.path().join("2026-10-21 09.42.17.png"));
        std::fs::write(&first, b"").unwrap();
        let second = capture_path(dir.path(), DEFAULT_PATTERN, &T, "png").unwrap();
        assert_eq!(second, dir.path().join("2026-10-21 09.42.17 (2).png"));
    }
}
