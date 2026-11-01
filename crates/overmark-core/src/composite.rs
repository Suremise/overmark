//! Compositing ink layers onto a clean screen image, as a save does: the backdrop (the clean
//! grab, the frozen frame, or a whiteboard or blackboard's colour), then the guides over it if
//! saves include them, then the highlighter's layer by darkening, then the ink's layer over it
//! (docs/ARCHITECTURE.md, Saving; [`compose_save`]). Every image is rows of 4-byte pixels with
//! the colour channels first, in the same order (BGRA or RGBA); layers are premultiplied, the
//! screen opaque.

use crate::colour::Colour;

/// The order of a pixel's colour bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOrder {
    /// Red, green, blue, alpha: tiny-skia's.
    Rgba,
    /// Blue, green, red, alpha: Windows' bitmaps.
    Bgra,
}

/// What a save starts from.
#[derive(Debug, Clone, Copy)]
pub enum SaveBase<'a> {
    /// An opaque image: the clean grab (Live) or the frozen frame (Frozen).
    Image(&'a [u8]),
    /// A whiteboard or blackboard: `pixels` pixels of `colour`, always opaque (any alpha is
    /// dropped, as the screen shows it).
    Solid {
        colour: Colour,
        order: ChannelOrder,
        pixels: usize,
    },
}

/// The layers drawn over a save's base, each the base's size, premultiplied, in the base's
/// channel order. Leave out what isn't wanted: the guides when Settings' `include_in_saves`
/// is off, the ink for "save without the drawing".
#[derive(Debug, Clone, Copy, Default)]
pub struct SaveLayers<'a> {
    pub guides: Option<&'a [u8]>,
    pub highlight: Option<&'a [u8]>,
    pub ink: Option<&'a [u8]>,
}

/// A save's pixels: the base, then the guides (over), the highlighter (darken) and the ink
/// (over), in that order.
pub fn compose_save(base: SaveBase, layers: &SaveLayers) -> Vec<u8> {
    let mut screen = match base {
        SaveBase::Image(image) => image.to_vec(),
        SaveBase::Solid {
            colour,
            order,
            pixels,
        } => {
            let px = match order {
                ChannelOrder::Rgba => [colour.r, colour.g, colour.b, 255],
                ChannelOrder::Bgra => [colour.b, colour.g, colour.r, 255],
            };
            px.repeat(pixels)
        }
    };
    if let Some(guides) = layers.guides {
        over(&mut screen, guides);
    }
    if let Some(highlight) = layers.highlight {
        darken(&mut screen, highlight);
    }
    if let Some(ink) = layers.ink {
        over(&mut screen, ink);
    }
    screen
}

/// Draws the premultiplied `layer` over `screen` (source-over), in place.
pub fn over(screen: &mut [u8], layer: &[u8]) {
    assert_eq!(
        screen.len(),
        layer.len(),
        "the screen and the layer differ in size"
    );
    for (d, s) in screen
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(layer.as_chunks::<4>().0)
    {
        let a = u32::from(s[3]);
        if a == 0 {
            continue;
        }
        for n in 0..3 {
            d[n] = (u32::from(s[n]) + (u32::from(d[n]) * (255 - a) + 127) / 255).min(255) as u8;
        }
        d[3] = 255;
    }
}

/// Darkens `screen` by the premultiplied `layer`, in place: each channel becomes the smaller of
/// the two where the layer is opaque, blended by its alpha where it isn't. This is what
/// DirectComposition's `MIN_BLEND` shows for the live highlighter, so a save matches the screen.
pub fn darken(screen: &mut [u8], layer: &[u8]) {
    assert_eq!(
        screen.len(),
        layer.len(),
        "the screen and the layer differ in size"
    );
    for (d, s) in screen
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(layer.as_chunks::<4>().0)
    {
        let a = u32::from(s[3]);
        if a == 0 {
            continue;
        }
        for n in 0..3 {
            let dst = u32::from(d[n]);
            // Un-premultiply the layer's colour, take the minimum, blend by alpha.
            let src = (u32::from(s[n]) * 255 + a / 2) / a;
            let min = src.min(dst);
            d[n] = ((min * a + dst * (255 - a) + 127) / 255) as u8;
        }
        d[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_draws_ink_on_the_screen() {
        let mut screen = vec![50, 100, 200, 255, 50, 100, 200, 255, 50, 100, 200, 255];
        // Clear, opaque red (BGRA), half-transparent red premultiplied.
        let layer = vec![0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 128, 128];
        over(&mut screen, &layer);
        assert_eq!(&screen[0..4], &[50, 100, 200, 255]);
        assert_eq!(&screen[4..8], &[0, 0, 255, 255]);
        // 128 + 200 * 127 / 255 = 228; blue and green halve.
        assert_eq!(&screen[8..12], &[25, 50, 228, 255]);
    }

    #[test]
    fn darken_keeps_dark_text_and_tints_paper() {
        // White paper, black text, a mid colour; an opaque yellow highlighter (BGRA 0, 255, 255).
        let mut screen = vec![255, 255, 255, 255, 0, 0, 0, 255, 50, 100, 200, 255];
        let layer = vec![0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255];
        darken(&mut screen, &layer);
        assert_eq!(&screen[0..4], &[0, 255, 255, 255]);
        assert_eq!(&screen[4..8], &[0, 0, 0, 255]);
        // What MIN_BLEND showed live over (200, 100, 50) in the ink-layer spike.
        assert_eq!(&screen[8..12], &[0, 100, 200, 255]);
    }

    #[test]
    fn a_save_composes_base_guides_highlight_then_ink() {
        // Two pixels, BGRA. A solid white base; a guide on pixel 0; yellow highlight on both;
        // red ink on pixel 1.
        let guides = [0, 0, 0, 128, 0, 0, 0, 0];
        let highlight = [0, 255, 255, 255, 0, 255, 255, 255];
        let ink = [0, 0, 0, 0, 0, 0, 255, 255];
        let base = SaveBase::Solid {
            colour: Colour::rgb(255, 255, 255),
            order: ChannelOrder::Bgra,
            pixels: 2,
        };
        let with = compose_save(
            base,
            &SaveLayers {
                guides: Some(&guides),
                highlight: Some(&highlight),
                ink: Some(&ink),
            },
        );
        // Pixel 0: white, a half-black guide (127), then yellow darkens blue to 0.
        assert_eq!(&with[0..4], &[0, 127, 127, 255]);
        assert_eq!(&with[4..8], &[0, 0, 255, 255]);
        let without = compose_save(
            base,
            &SaveLayers {
                guides: None,
                highlight: Some(&highlight),
                ink: Some(&ink),
            },
        );
        assert_eq!(&without[0..4], &[0, 255, 255, 255]);
    }

    #[test]
    fn a_solid_base_follows_the_channel_order() {
        let c = Colour::rgb(10, 20, 30);
        let rgba = compose_save(
            SaveBase::Solid {
                colour: c,
                order: ChannelOrder::Rgba,
                pixels: 1,
            },
            &SaveLayers::default(),
        );
        let bgra = compose_save(
            SaveBase::Solid {
                colour: c,
                order: ChannelOrder::Bgra,
                pixels: 1,
            },
            &SaveLayers::default(),
        );
        assert_eq!((rgba, bgra), (vec![10, 20, 30, 255], vec![30, 20, 10, 255]));
        let image = [1, 2, 3, 255];
        assert_eq!(
            compose_save(SaveBase::Image(&image), &SaveLayers::default()),
            image
        );
    }

    #[test]
    fn a_clear_layer_changes_nothing() {
        let mut screen = vec![1, 2, 3, 255];
        over(&mut screen, &[0, 0, 0, 0]);
        darken(&mut screen, &[0, 0, 0, 0]);
        assert_eq!(screen, vec![1, 2, 3, 255]);
    }
}
