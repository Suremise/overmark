//! The language Windows shows its own UI in, for the strings (`overmark_core::i18n::init`).

use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::core::PWSTR;

/// The user's first Windows UI language, as a tag ("en-GB"); "en-US" if Windows doesn't say.
pub fn ui_language() -> String {
    let (mut count, mut len) = (0u32, 0u32);
    // SAFETY: the first call asks for the buffer's length; the second fills a buffer of it.
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err()
            || len == 0
        {
            return "en-US".to_owned();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
        .is_err()
        {
            return "en-US".to_owned();
        }
        // A double-NUL-terminated list: the first entry.
        let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
        let tag = String::from_utf16_lossy(&buf[..end]);
        if tag.is_empty() {
            "en-US".to_owned()
        } else {
            tag
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gives_a_language_tag() {
        let tag = super::ui_language();
        assert!(
            tag.len() >= 2 && tag.chars().next().unwrap().is_ascii_alphabetic(),
            "{tag}"
        );
    }
}
