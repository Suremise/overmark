//! Text: laying out a text item and turning its glyphs into paths.
//!
//! `parley` lays the text out (shaping, kerning, lines at each newline) and `swash` reads each
//! glyph's outline, which is filled like any other ink (docs/ARCHITECTURE.md, Text: parley and
//! swash).
//!
//! **Fonts come from the platform at run time; none ships**. `overmark-win` reads
//! the system's Segoe UI Variable (or Segoe UI) from `%WINDIR%\Fonts` and calls [`set_fonts`]
//! once at start-up, with fallbacks after it. Until then, text has no glyphs: a text item draws
//! nothing but its caret, and a stamp shows its circle without the number. The tests use Tuffy
//! (public domain, `tests/fonts/`), which is never shipped.

use std::sync::{Arc, Mutex, OnceLock};

use kurbo::{BezPath, Point, Rect};
use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontFamily, FontFamilyName, Layout,
    LayoutContext, PositionedLayoutItem, StyleProperty,
};
use swash::scale::ScaleContext;
use swash::zeno::{Command, PathData};

/// A problem with font data handed to [`set_fonts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// None of the data was a font parley could read.
    NoFont,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A diagnostic for the log; the app words what people see.
        f.write_str("none of the font files could be read")
    }
}

impl std::error::Error for FontError {}

struct Engine {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    scaler: ScaleContext,
    /// The family names to use, first choice first.
    families: Vec<String>,
}

fn engine() -> &'static Mutex<Option<Engine>> {
    static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();
    ENGINE.get_or_init(|| Mutex::new(None))
}

/// Sets the fonts text is drawn in: each a font file's bytes, the first the main font and the
/// rest fallbacks for characters it lacks. Replaces any set before. Items already in a
/// drawing keep their glyphs until they change.
pub fn set_fonts(files: Vec<Vec<u8>>) -> Result<(), FontError> {
    let mut fonts = FontContext::new();
    let mut families = Vec::new();
    for data in files {
        let blob = parley::fontique::Blob::new(Arc::new(data));
        for (family, _) in fonts.collection.register_fonts(blob, None) {
            if let Some(name) = fonts.collection.family_name(family)
                && !families.iter().any(|f| f == name)
            {
                families.push(name.to_owned());
            }
        }
    }
    if families.is_empty() {
        return Err(FontError::NoFont);
    }
    *engine().lock().unwrap_or_else(|e| e.into_inner()) = Some(Engine {
        fonts,
        layouts: LayoutContext::new(),
        scaler: ScaleContext::new(),
        families,
    });
    Ok(())
}

/// Whether [`set_fonts`] has been given a font.
pub fn has_fonts() -> bool {
    engine().lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Text laid out at a size, its top-left at (0, 0).
#[derive(Debug, Clone, Default)]
pub struct TextShape {
    /// The glyphs' outlines, to fill (non-zero).
    pub glyphs: BezPath,
    /// The laid-out size: each line's advance, and the lines' height.
    pub width: f64,
    pub height: f64,
    /// Where the caret goes at each byte index that starts a character (and at the end): a
    /// thin rectangle the line's height.
    carets: Vec<(usize, Rect)>,
    /// One line's height, for a caret in empty text.
    pub line_height: f64,
}

impl TextShape {
    /// The caret's rectangle at byte index `index` of the text (clamped to a character
    /// boundary before it).
    pub fn caret(&self, index: usize) -> Rect {
        self.carets
            .iter()
            .rev()
            .find(|(i, _)| *i <= index)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| Rect::new(0.0, 0.0, 1.0, self.line_height))
    }
}

/// Lays out `text` (lines split at `\n`) at `size` pixels. Without fonts it has no glyphs, and
/// its caret is a line `size` tall.
pub fn shape(text: &str, size: f32) -> TextShape {
    let size = if size.is_finite() {
        size.max(1.0)
    } else {
        16.0
    };
    let mut guard = engine().lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = guard.as_mut() else {
        let line = f64::from(size) * 1.2;
        return TextShape {
            line_height: line,
            carets: vec![(0, Rect::new(0.0, 0.0, 1.0, line))],
            ..TextShape::default()
        };
    };
    let Engine {
        fonts,
        layouts,
        scaler,
        families,
    } = engine;

    let names: Vec<FontFamilyName<'_>> = families
        .iter()
        .map(|f| FontFamilyName::Named(f.as_str().into()))
        .collect();
    let mut builder = layouts.ranged_builder(fonts, text, 1.0, false);
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(&names[..])));
    let mut layout: Layout<()> = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    let mut glyphs = BezPath::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let font = run.run().font();
            let Some(font_ref) = swash::FontRef::from_index(font.data.data(), font.index as usize)
            else {
                continue;
            };
            let coords: Vec<i16> = run
                .run()
                .normalized_coords()
                .iter()
                .map(|c| c.to_bits())
                .collect();
            let mut glyph_scaler = scaler
                .builder(font_ref)
                .size(run.run().font_size())
                .hint(false)
                .normalized_coords(coords.iter())
                .build();
            for glyph in run.positioned_glyphs() {
                let Ok(id) = u16::try_from(glyph.id) else {
                    continue;
                };
                let Some(outline) = glyph_scaler.scale_outline(id) else {
                    continue;
                };
                let (gx, gy) = (f64::from(glyph.x), f64::from(glyph.y));
                // Font outlines point y up; the layout's y points down.
                let at =
                    |p: swash::zeno::Vector| Point::new(gx + f64::from(p.x), gy - f64::from(p.y));
                for command in outline.path().commands() {
                    match command {
                        Command::MoveTo(p) => glyphs.move_to(at(p)),
                        Command::LineTo(p) => glyphs.line_to(at(p)),
                        Command::QuadTo(c, p) => glyphs.quad_to(at(c), at(p)),
                        Command::CurveTo(a, b, p) => glyphs.curve_to(at(a), at(b), at(p)),
                        Command::Close => glyphs.close_path(),
                    }
                }
            }
        }
    }

    let line_height = layout.lines().next().map_or(f64::from(size) * 1.2, |l| {
        f64::from(l.metrics().line_height)
    });
    let mut carets = Vec::new();
    let boundaries = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()));
    for index in boundaries {
        let affinity = if index > 0 && text[..index].ends_with('\n') {
            Affinity::Downstream
        } else if index == text.len() {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        let g = Cursor::from_byte_index(&layout, index, affinity).geometry(&layout, 1.0);
        carets.push((index, Rect::new(g.x0, g.y0, g.x1, g.y1)));
    }

    TextShape {
        glyphs,
        width: f64::from(layout.width()),
        height: f64::from(layout.height()),
        carets,
        line_height,
    }
}

#[cfg(test)]
pub(crate) fn use_test_font() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let tuffy = include_bytes!("../tests/fonts/Tuffy.ttf").to_vec();
        set_fonts(vec![tuffy]).expect("Tuffy loads");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_has_glyphs_and_grows_with_its_content() {
        use_test_font();
        let one = shape("Hi", 20.0);
        let two = shape("Hi there", 20.0);
        assert!(!one.glyphs.elements().is_empty());
        assert!(two.width > one.width);
        assert!(one.height >= 20.0 && one.height < 40.0, "{}", one.height);
        let lines = shape("Hi\nthere", 20.0);
        assert!(
            lines.height > one.height * 1.8,
            "{} vs {}",
            lines.height,
            one.height
        );
    }

    #[test]
    fn the_caret_moves_with_the_index_and_to_new_lines() {
        use_test_font();
        let s = shape("ab\ncd", 20.0);
        let start = s.caret(0);
        let after_b = s.caret(2);
        let line_two = s.caret(3);
        let end = s.caret(5);
        assert!(after_b.x0 > start.x0);
        assert!(line_two.y0 > start.y0);
        assert!((line_two.x0 - start.x0).abs() < 1.0);
        assert!(end.x0 > line_two.x0);
        // Empty text still has a caret, a line tall.
        let empty = shape("", 20.0);
        assert!(empty.caret(0).height() > 10.0);
    }

    #[test]
    fn set_fonts_refuses_data_that_isnt_a_font() {
        assert_eq!(
            set_fonts(vec![b"not a font".to_vec()]),
            Err(FontError::NoFont)
        );
        // A failed call leaves the fonts as they were.
        use_test_font();
        assert!(has_fonts());
    }
}
