//! A colour, written as `#RRGGBB` or `#RRGGBBAA` in `settings.toml`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Colour {
    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Colour { r, g, b, a: 255 }
    }
}

/// Text that isn't `#RRGGBB` or `#RRGGBBAA`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadColour(pub String);

impl fmt::Display for BadColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Shown inside a settings error: the detail.
        f.write_str(&crate::tr!(
            "error-not-a-colour",
            text = format!("{:?}", self.0)
        ))
    }
}

impl std::error::Error for BadColour {}

impl FromStr for Colour {
    type Err = BadColour;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadColour(s.to_owned());
        let hex = s.trim().strip_prefix('#').ok_or_else(bad)?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Colour {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if hex.len() == 8 { byte(6)? } else { 255 },
        })
    }
}

impl fmt::Display for Colour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02X}", self.a)?;
        }
        Ok(())
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_hex() {
        let c: Colour = "#e53935".parse().unwrap();
        assert_eq!(c, Colour::rgb(0xE5, 0x39, 0x35));
        assert_eq!(c.to_string(), "#E53935");
        let c: Colour = "#11223344".parse().unwrap();
        assert_eq!(c.a, 0x44);
        assert_eq!(c.to_string(), "#11223344");
    }

    #[test]
    fn rejects_what_isnt_hex() {
        for s in [
            "", "#", "e53935", "#e5393", "#e53935f", "#zz3935", "#+1+2+3",
        ] {
            assert!(s.parse::<Colour>().is_err(), "{s}");
        }
    }
}
