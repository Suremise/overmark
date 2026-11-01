//! The drawing tools: which ones there are, their keys and their names.

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// A drawing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    #[default]
    Pen,
    Highlighter,
    Arrow,
    Rectangle,
    Ellipse,
    Line,
    Text,
    Stamp,
    Blur,
    Eraser,
    Laser,
}

impl ToolKind {
    /// Every tool, in the toolbar's order.
    pub const ALL: [ToolKind; 11] = [
        ToolKind::Pen,
        ToolKind::Highlighter,
        ToolKind::Arrow,
        ToolKind::Rectangle,
        ToolKind::Ellipse,
        ToolKind::Line,
        ToolKind::Text,
        ToolKind::Stamp,
        ToolKind::Blur,
        ToolKind::Eraser,
        ToolKind::Laser,
    ];

    /// The key that picks it, as a capital letter.
    pub fn key(self) -> char {
        match self {
            ToolKind::Pen => 'P',
            ToolKind::Highlighter => 'H',
            ToolKind::Arrow => 'A',
            ToolKind::Rectangle => 'R',
            ToolKind::Ellipse => 'E',
            ToolKind::Line => 'L',
            ToolKind::Text => 'T',
            ToolKind::Stamp => 'N',
            ToolKind::Blur => 'B',
            ToolKind::Eraser => 'X',
            ToolKind::Laser => 'Z',
        }
    }

    /// Its name in `settings.toml` (`"pen"`, `"highlighter"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            ToolKind::Pen => "pen",
            ToolKind::Highlighter => "highlighter",
            ToolKind::Arrow => "arrow",
            ToolKind::Rectangle => "rectangle",
            ToolKind::Ellipse => "ellipse",
            ToolKind::Line => "line",
            ToolKind::Text => "text",
            ToolKind::Stamp => "stamp",
            ToolKind::Blur => "blur",
            ToolKind::Eraser => "eraser",
            ToolKind::Laser => "laser",
        }
    }

    /// Its name as people see it, in the current language.
    pub fn name(self) -> String {
        match self {
            ToolKind::Pen => tr("tool-pen"),
            ToolKind::Highlighter => tr("tool-highlighter"),
            ToolKind::Arrow => tr("tool-arrow"),
            ToolKind::Rectangle => tr("tool-rectangle"),
            ToolKind::Ellipse => tr("tool-ellipse"),
            ToolKind::Line => tr("tool-line"),
            ToolKind::Text => tr("tool-text"),
            ToolKind::Stamp => tr("tool-stamp"),
            ToolKind::Blur => tr("tool-blur"),
            ToolKind::Eraser => tr("tool-eraser"),
            ToolKind::Laser => tr("tool-laser"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn keys_are_unique() {
        let keys: String = ToolKind::ALL.iter().map(|t| t.key()).collect();
        assert_eq!(keys, "PHARELTNBXZ");
        assert_eq!(
            keys.chars().collect::<HashSet<_>>().len(),
            ToolKind::ALL.len()
        );
    }

    #[test]
    fn as_str_matches_serde() {
        for tool in ToolKind::ALL {
            let value = toml::Value::try_from(tool).unwrap();
            assert_eq!(value.as_str(), Some(tool.as_str()));
        }
    }

    #[test]
    fn every_tool_has_a_name() {
        for tool in ToolKind::ALL {
            assert_ne!(tool.name(), format!("tool-{}", tool.as_str()));
        }
    }
}
