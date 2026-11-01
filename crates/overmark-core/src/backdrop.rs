//! What's under the ink: a backdrop, and guides over it.
//!
//! - **Backdrop**, one at a time, cycled by W (Shift+W back): [`Backdrop::Live`] (the screen,
//!   moving), [`Backdrop::Frozen`] (the frozen frame), [`Backdrop::Whiteboard`] and
//!   [`Backdrop::Blackboard`] (solid colours from Settings). F still toggles Live and Frozen.
//! - **Guides**, over any backdrop, Live included, cycled by G (Shift+G back): [`Guides::Off`],
//!   [`Guides::Grid`], [`Guides::Dots`], [`Guides::Lines`] (ruled).
//!
//! Each is a list: a new entry is added to its enum and its `ALL`, and a match arm or two
//! here; nothing elsewhere changes. The overlay (overmark-win) wires the keys and the windows;
//! core gives the models, their names, how each is filled, and the guides' pixels
//! ([`render_guides`], [`GuideLayer`]). Blur stays Frozen-only: [`crate::session::Session`]
//! refuses it on any other backdrop.

use tiny_skia::{IntRect, Pixmap, PixmapMut};

use crate::colour::Colour;
use crate::render::View;
use crate::settings::{BackdropSettings, GuideSettings};
use crate::tr;

/// What's under the ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Backdrop {
    /// The screen, still moving underneath.
    #[default]
    Live,
    /// The screen as it was when frozen.
    Frozen,
    /// A solid white (Settings' `backdrops.whiteboard_colour`).
    Whiteboard,
    /// A solid dark grey (Settings' `backdrops.blackboard_colour`).
    Blackboard,
}

/// How a backdrop is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdropFill {
    /// Nothing: the live screen shows through (in a save, the clean grab).
    Screen,
    /// The frozen frame.
    Frame,
    /// A solid colour.
    Solid(Colour),
}

impl Backdrop {
    /// Every backdrop, in the order W cycles them.
    pub const ALL: [Backdrop; 4] = [
        Backdrop::Live,
        Backdrop::Frozen,
        Backdrop::Whiteboard,
        Backdrop::Blackboard,
    ];

    /// The next backdrop (W), wrapping round.
    pub fn next(self) -> Self {
        cycle(&Self::ALL, self, 1)
    }

    /// The previous backdrop (Shift+W), wrapping round.
    pub fn prev(self) -> Self {
        cycle(&Self::ALL, self, -1)
    }

    /// Its name, in the current language.
    pub fn name(self) -> String {
        match self {
            Backdrop::Live => tr!("backdrop-live"),
            Backdrop::Frozen => tr!("backdrop-frozen"),
            Backdrop::Whiteboard => tr!("backdrop-whiteboard"),
            Backdrop::Blackboard => tr!("backdrop-blackboard"),
        }
    }

    /// The id of its icon, for the toolbar.
    pub fn icon(self) -> &'static str {
        match self {
            Backdrop::Live => "backdrop-live",
            Backdrop::Frozen => "backdrop-frozen",
            Backdrop::Whiteboard => "backdrop-whiteboard",
            Backdrop::Blackboard => "backdrop-blackboard",
        }
    }

    /// How it's filled, with the colours from Settings. A solid backdrop is always opaque (it
    /// hides the screen, on screen and in a save alike): any alpha in its colour is dropped.
    pub fn fill(self, settings: &BackdropSettings) -> BackdropFill {
        let opaque = |c: Colour| Colour { a: 255, ..c };
        match self {
            Backdrop::Live => BackdropFill::Screen,
            Backdrop::Frozen => BackdropFill::Frame,
            Backdrop::Whiteboard => BackdropFill::Solid(opaque(settings.whiteboard_colour)),
            Backdrop::Blackboard => BackdropFill::Solid(opaque(settings.blackboard_colour)),
        }
    }

    /// Whether there's a still image under the ink, so Blur has something to pixelate: only the
    /// frozen frame.
    pub fn allows_blur(self) -> bool {
        self == Backdrop::Frozen
    }

    /// The colour guides are drawn in over this backdrop: dark on light, light on dark, and a
    /// neutral translucent grey over the screen, which could be anything.
    pub fn guide_colour(self, settings: &BackdropSettings) -> Colour {
        match self.fill(settings) {
            BackdropFill::Solid(c) if luminance(c) < 0.5 => Colour {
                a: 70,
                ..Colour::rgb(255, 255, 255)
            },
            BackdropFill::Solid(_) => Colour {
                a: 60,
                ..Colour::rgb(0x1E, 0x3A, 0x5F)
            },
            BackdropFill::Screen | BackdropFill::Frame => Colour {
                a: 90,
                ..Colour::rgb(128, 128, 128)
            },
        }
    }
}

/// Guides over the backdrop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Guides {
    #[default]
    Off,
    /// Lines both ways, `spacing` apart.
    Grid,
    /// A dot where the grid's lines would cross.
    Dots,
    /// Horizontal lines, like ruled paper.
    Lines,
}

impl Guides {
    /// Every kind, in the order G cycles them.
    pub const ALL: [Guides; 4] = [Guides::Off, Guides::Grid, Guides::Dots, Guides::Lines];

    /// The next kind (G), wrapping round.
    pub fn next(self) -> Self {
        cycle(&Self::ALL, self, 1)
    }

    /// The previous kind (Shift+G), wrapping round.
    pub fn prev(self) -> Self {
        cycle(&Self::ALL, self, -1)
    }

    /// Its name, in the current language.
    pub fn name(self) -> String {
        match self {
            Guides::Off => tr!("guides-off"),
            Guides::Grid => tr!("guides-grid"),
            Guides::Dots => tr!("guides-dots"),
            Guides::Lines => tr!("guides-lines"),
        }
    }

    /// The id of its icon, for the toolbar.
    pub fn icon(self) -> &'static str {
        match self {
            Guides::Off => "guides-off",
            Guides::Grid => "guides-grid",
            Guides::Dots => "guides-dots",
            Guides::Lines => "guides-lines",
        }
    }
}

fn cycle<T: Copy + PartialEq>(all: &[T], current: T, step: isize) -> T {
    let n = all.len() as isize;
    let i = all.iter().position(|x| *x == current).unwrap_or(0) as isize;
    all[(i + step).rem_euclid(n) as usize]
}

fn luminance(c: Colour) -> f64 {
    let lin = |v: u8| f64::from(v) / 255.0;
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// Fills `region` of `target` (all of it if `None`) with a solid backdrop colour, replacing
/// what's there.
pub fn render_solid(target: &mut PixmapMut, colour: Colour, region: Option<IntRect>) {
    let Some(region) = whole_or(target, region) else {
        return;
    };
    let px = premultiplied(colour);
    let width = target.width() as usize;
    let data = target.data_mut();
    for y in region.top()..region.bottom() {
        let row = (y as usize * width + region.left() as usize) * 4;
        for p in data[row..row + region.width() as usize * 4]
            .as_chunks_mut::<4>()
            .0
        {
            p.copy_from_slice(&px);
        }
    }
}

/// How guides are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideStyle {
    /// The distance between lines or dots, in drawing pixels.
    pub spacing: u32,
    pub colour: Colour,
}

impl GuideStyle {
    /// The style from Settings, over `backdrop`.
    pub fn new(guides: &GuideSettings, backdrops: &BackdropSettings, backdrop: Backdrop) -> Self {
        GuideStyle {
            spacing: guides.spacing,
            colour: backdrop.guide_colour(backdrops),
        }
    }
}

/// Draws `guides` over what's in `target` (source-over), in `region` only (all of it if
/// `None`): lines 1 pixel wide (more at higher scales) and square dots, on whole pixels, so they
/// are crisp at 100%, 150% and any scale, and a region draws exactly what a full render does.
/// The pattern starts at the target's top-left.
pub fn render_guides(
    target: &mut PixmapMut,
    guides: Guides,
    style: &GuideStyle,
    view: View,
    region: Option<IntRect>,
) {
    let Some(region) = whole_or(target, region) else {
        return;
    };
    if guides == Guides::Off || style.colour.a == 0 {
        return;
    }
    let scale = f64::from(view.scale.max(0.01));
    let step = f64::from(style.spacing.max(1)) * scale;
    let thick = scale.round().max(1.0) as i32;
    let dot = (2.0 * scale).round().max(2.0) as i32;
    let colour = premultiplied(style.colour);
    // The guide positions (in target pixels) from 0 up to `limit`.
    let positions = |limit: i32| -> Vec<i32> {
        let mut out = Vec::new();
        let mut k = 0u32;
        loop {
            let at = (f64::from(k) * step).round() as i32;
            if at >= limit {
                return out;
            }
            out.push(at);
            k += 1;
        }
    };
    let (w, h) = (target.width() as i32, target.height() as i32);
    let mut paint = |x0: i32, y0: i32, x1: i32, y1: i32| {
        let (x0, y0) = (x0.max(region.left()), y0.max(region.top()));
        let (x1, y1) = (x1.min(region.right()), y1.min(region.bottom()));
        if x1 > x0 && y1 > y0 {
            blend_rect(target, x0, y0, x1, y1, colour);
        }
    };
    match guides {
        Guides::Off => {}
        Guides::Grid => {
            // Vertical lines, then horizontal ones that skip where they cross, so a crossing
            // isn't darker than the lines.
            let xs = positions(w);
            for &x in &xs {
                paint(x, 0, x + thick, h);
            }
            for y in positions(h) {
                let mut from = 0;
                for &x in &xs {
                    paint(from, y, x, y + thick);
                    from = x + thick;
                }
                paint(from, y, w, y + thick);
            }
        }
        Guides::Dots => {
            let half = dot / 2;
            for y in positions(h) {
                for x in positions(w) {
                    paint(x - half, y - half, x - half + dot, y - half + dot);
                }
            }
        }
        Guides::Lines => {
            for y in positions(h) {
                paint(0, y, w, y + thick);
            }
        }
    }
}

/// The guides as a layer of their own, kept until something about them changes: render it
/// once, then reuse it each frame (the overlay shows it as a layer between the backdrop and the
/// ink, or draws it under the ink).
#[derive(Debug, Default)]
pub struct GuideLayer {
    key: Option<(u32, u32, Guides, GuideStyle, u32)>,
    pixmap: Option<Pixmap>,
}

impl GuideLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The guides on a transparent `width` × `height` layer at `view`'s scale, drawn only when
    /// this differs from last time. `None` for [`Guides::Off`] (or an empty size).
    pub fn layer(
        &mut self,
        width: u32,
        height: u32,
        guides: Guides,
        style: &GuideStyle,
        view: View,
    ) -> Option<&Pixmap> {
        if guides == Guides::Off {
            return None;
        }
        let key = (width, height, guides, *style, view.scale.to_bits());
        if self.key != Some(key) || self.pixmap.is_none() {
            let mut pixmap = Pixmap::new(width, height)?;
            render_guides(&mut pixmap.as_mut(), guides, style, view, None);
            self.pixmap = Some(pixmap);
            self.key = Some(key);
        }
        self.pixmap.as_ref()
    }

    /// Whether the last [`GuideLayer::layer`] call drew anew (for tests and timing).
    pub fn is_for(
        &self,
        width: u32,
        height: u32,
        guides: Guides,
        style: &GuideStyle,
        view: View,
    ) -> bool {
        self.key == Some((width, height, guides, *style, view.scale.to_bits()))
    }
}

fn whole_or(target: &PixmapMut, region: Option<IntRect>) -> Option<IntRect> {
    let all = IntRect::from_xywh(0, 0, target.width(), target.height())?;
    match region {
        Some(r) => r.intersect(&all),
        None => Some(all),
    }
}

/// A straight-alpha colour as premultiplied RGBA bytes.
fn premultiplied(c: Colour) -> [u8; 4] {
    let a = u32::from(c.a);
    let m = |v: u8| ((u32::from(v) * a + 127) / 255) as u8;
    [m(c.r), m(c.g), m(c.b), c.a]
}

/// Draws a premultiplied colour over the rectangle (source-over).
fn blend_rect(target: &mut PixmapMut, x0: i32, y0: i32, x1: i32, y1: i32, colour: [u8; 4]) {
    let width = target.width() as usize;
    let data = target.data_mut();
    let inv = 255 - u32::from(colour[3]);
    for y in y0..y1 {
        let row = (y as usize * width + x0 as usize) * 4;
        for p in data[row..row + (x1 - x0) as usize * 4]
            .as_chunks_mut::<4>()
            .0
        {
            for c in 0..4 {
                p[c] = (u32::from(colour[c]) + (u32::from(p[c]) * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrops_cycle_both_ways_in_the_decided_order() {
        let mut b = Backdrop::Live;
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(b);
            b = b.next();
        }
        assert_eq!(
            seen,
            [
                Backdrop::Live,
                Backdrop::Frozen,
                Backdrop::Whiteboard,
                Backdrop::Blackboard,
                Backdrop::Live
            ]
        );
        assert_eq!(Backdrop::Live.prev(), Backdrop::Blackboard);
        for b in Backdrop::ALL {
            assert_eq!(b.next().prev(), b);
        }
    }

    #[test]
    fn guides_cycle_both_ways_in_the_decided_order() {
        assert_eq!(Guides::Off.next(), Guides::Grid);
        assert_eq!(Guides::Grid.next(), Guides::Dots);
        assert_eq!(Guides::Dots.next(), Guides::Lines);
        assert_eq!(Guides::Lines.next(), Guides::Off);
        assert_eq!(Guides::Off.prev(), Guides::Lines);
        for g in Guides::ALL {
            assert_eq!(g.prev().next(), g);
        }
    }

    #[test]
    fn every_entry_has_a_name_and_an_icon() {
        for b in Backdrop::ALL {
            assert!(!b.name().starts_with("backdrop-"), "{b:?}");
            assert_eq!(
                b.icon(),
                format!("backdrop-{}", format!("{b:?}").to_lowercase())
            );
        }
        for g in Guides::ALL {
            assert!(!g.name().starts_with("guides-"), "{g:?}");
            assert!(g.icon().starts_with("guides-"));
        }
    }

    #[test]
    fn fills_come_from_settings_and_only_frozen_allows_blur() {
        let s = BackdropSettings {
            whiteboard_colour: Colour::rgb(1, 2, 3),
            ..BackdropSettings::default()
        };
        assert_eq!(
            Backdrop::Whiteboard.fill(&s),
            BackdropFill::Solid(Colour::rgb(1, 2, 3))
        );
        assert_eq!(Backdrop::Live.fill(&s), BackdropFill::Screen);
        assert_eq!(Backdrop::Frozen.fill(&s), BackdropFill::Frame);
        let blur: Vec<_> = Backdrop::ALL.iter().map(|b| b.allows_blur()).collect();
        assert_eq!(blur, [false, true, false, false]);
    }

    #[test]
    fn guide_colours_contrast_with_the_backdrop() {
        let s = BackdropSettings::default();
        let on_white = Backdrop::Whiteboard.guide_colour(&s);
        let on_black = Backdrop::Blackboard.guide_colour(&s);
        assert!(luminance(on_white) < 0.5 && luminance(on_black) > 0.5);
        assert!(on_white.a < 255 && on_black.a < 255, "guides stay quiet");
    }

    fn style() -> GuideStyle {
        GuideStyle {
            spacing: 10,
            colour: Colour::rgb(0, 0, 0),
        }
    }

    fn alpha_at(p: &Pixmap, x: u32, y: u32) -> u8 {
        p.pixel(x, y).unwrap().alpha()
    }

    #[test]
    fn grid_lines_are_crisp_whole_pixels() {
        let mut p = Pixmap::new(50, 30).unwrap();
        render_guides(
            &mut p.as_mut(),
            Guides::Grid,
            &style(),
            View::default(),
            None,
        );
        // Lines at 0, 10, 20…, one pixel wide, fully opaque, nothing between.
        assert_eq!(alpha_at(&p, 10, 5), 255);
        assert_eq!(alpha_at(&p, 11, 5), 0);
        assert_eq!(alpha_at(&p, 5, 20), 255);
        assert_eq!(alpha_at(&p, 5, 21), 0);
        assert_eq!(alpha_at(&p, 5, 5), 0);
        // At 150% the step is 15 px and lines are 2 px (1.5 rounded).
        let mut p = Pixmap::new(50, 30).unwrap();
        render_guides(
            &mut p.as_mut(),
            Guides::Grid,
            &style(),
            View { scale: 1.5 },
            None,
        );
        assert_eq!(
            (
                alpha_at(&p, 15, 5),
                alpha_at(&p, 16, 5),
                alpha_at(&p, 17, 5)
            ),
            (255, 255, 0)
        );
    }

    #[test]
    fn a_translucent_grid_is_no_darker_where_lines_cross() {
        let mut p = Pixmap::new(30, 30).unwrap();
        let faint = GuideStyle {
            spacing: 10,
            colour: Colour {
                a: 100,
                ..Colour::rgb(0, 0, 0)
            },
        };
        render_guides(&mut p.as_mut(), Guides::Grid, &faint, View::default(), None);
        assert_eq!(alpha_at(&p, 10, 10), alpha_at(&p, 10, 5));
    }

    #[test]
    fn dots_sit_where_grid_lines_would_cross_and_lines_are_horizontal() {
        let mut p = Pixmap::new(40, 40).unwrap();
        render_guides(
            &mut p.as_mut(),
            Guides::Dots,
            &style(),
            View::default(),
            None,
        );
        assert_eq!(alpha_at(&p, 20, 20), 255);
        assert_eq!(alpha_at(&p, 20, 25), 0);
        assert_eq!(alpha_at(&p, 15, 20), 0);
        let mut p = Pixmap::new(40, 40).unwrap();
        render_guides(
            &mut p.as_mut(),
            Guides::Lines,
            &style(),
            View::default(),
            None,
        );
        assert_eq!(alpha_at(&p, 13, 20), 255);
        assert_eq!(alpha_at(&p, 20, 13), 0, "no vertical lines");
    }

    #[test]
    fn a_region_draws_exactly_what_a_full_render_does_there() {
        for guides in [Guides::Grid, Guides::Dots, Guides::Lines] {
            let view = View { scale: 1.5 };
            let mut full = Pixmap::new(80, 60).unwrap();
            render_guides(&mut full.as_mut(), guides, &style(), view, None);
            let mut parts = Pixmap::new(80, 60).unwrap();
            for (x, y) in [(0, 0), (37, 0), (0, 23), (37, 23)] {
                let r = IntRect::from_xywh(x, y, 43, 37).unwrap();
                let r = r
                    .intersect(&IntRect::from_xywh(0, 0, 80, 60).unwrap())
                    .unwrap();
                // Each region once: clear it, then draw it (as a redraw over a backdrop would).
                render_solid(
                    &mut parts.as_mut(),
                    Colour {
                        a: 0,
                        ..Colour::rgb(0, 0, 0)
                    },
                    Some(r),
                );
                render_guides(&mut parts.as_mut(), guides, &style(), view, Some(r));
            }
            assert!(full.data() == parts.data(), "{guides:?}");
        }
    }

    #[test]
    fn the_guide_layer_is_drawn_once_until_something_changes() {
        let mut cache = GuideLayer::new();
        let view = View::default();
        assert!(cache.layer(40, 40, Guides::Off, &style(), view).is_none());
        let first = cache
            .layer(40, 40, Guides::Grid, &style(), view)
            .unwrap()
            .data()
            .as_ptr();
        let again = cache
            .layer(40, 40, Guides::Grid, &style(), view)
            .unwrap()
            .data()
            .as_ptr();
        assert_eq!(first, again, "reused, not redrawn");
        assert!(cache.is_for(40, 40, Guides::Grid, &style(), view));
        cache.layer(40, 40, Guides::Dots, &style(), view).unwrap();
        assert!(!cache.is_for(40, 40, Guides::Grid, &style(), view));
    }

    #[test]
    fn a_solid_backdrop_fills_its_region() {
        let mut p = Pixmap::new(10, 10).unwrap();
        render_solid(
            &mut p.as_mut(),
            Colour::rgb(10, 20, 30),
            Some(IntRect::from_xywh(0, 0, 5, 10).unwrap()),
        );
        let c = p.pixel(2, 2).unwrap();
        assert_eq!((c.red(), c.green(), c.blue(), c.alpha()), (10, 20, 30, 255));
        assert_eq!(p.pixel(7, 2).unwrap().alpha(), 0);
    }
}
