//! Drawing a [`Drawing`] into pixels, with `tiny-skia`.
//!
//! One function serves both uses:
//! - **the live layer:** [`Background::Transparent`], so only the ink is opaque, then shown in
//!   a per-pixel-alpha window;
//! - **Frozen mode and a saved capture:** [`Background::Image`], the frozen frame or the screen
//!   image at full resolution, with the ink drawn over it.
//!
//! The pixels are tiny-skia's: RGBA, 8 bits each, **premultiplied** alpha. (Windows' layered
//! windows want premultiplied BGRA: swap red and blue.)
//!
//! While a stroke is drawn, redraw only what changed: each change to the [`Drawing`] returns the
//! area it touched, [`dirty_rect`] turns that into pixels, and [`render_region`] redraws just
//! that rectangle, giving exactly the pixels a full [`render`] would. On a 4K layer a pen sample
//! takes about half a millisecond, while a full redraw takes over a 144 Hz frame
//! (`docs/ARCHITECTURE.md`, Drawing and rendering).
//!
//! **Options** ([`RenderOptions`], with [`render_with`] and [`render_region_with`]):
//! - how the **highlighter** shows ([`Highlight`]), and [`render_highlights`] for a darken
//!   layer of its own under the ink;
//! - **disappearing ink**: items fade out a delay after they were drawn ([`opacity`]);
//! - the **laser pointer**, drawn over everything at the given time.
//!
//! **Blur** items pixelate what's under them, so they draw only over a [`Background::Image`]:
//! on a transparent layer there's nothing to pixelate.

use kurbo::{Circle, Point, Rect, Shape};
use tiny_skia::{
    BlendMode, Color, FillRule, IntRect, Paint, Pixmap, PixmapMut, PixmapRef, Shader, Transform,
};

use crate::colour::Colour;
use crate::drawing::{Drawing, Item};
use crate::geometry::{Geometry, Role, dark_on};
use crate::laser::{self, Laser};
use crate::tool::ToolKind;

/// What's under the ink.
#[derive(Clone, Copy)]
pub enum Background<'a> {
    /// Nothing: transparent pixels, for the live layer.
    Transparent,
    /// An image (the frozen frame or the captured screen), drawn unscaled from the target's
    /// top-left corner. Make it the target's size; anything beyond it is transparent.
    Image(PixmapRef<'a>),
}

/// How the drawing maps to the target's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Target pixels per drawing pixel: 1 for the layer and a capture of the same monitor;
    /// another value for, say, a thumbnail.
    pub scale: f32,
}

impl Default for View {
    fn default() -> Self {
        View { scale: 1.0 }
    }
}

impl View {
    fn transform(self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }
}

/// How highlighter strokes are drawn (docs/ARCHITECTURE.md, The highlighter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Highlight {
    /// Multiply over a background image, Tint on a transparent layer.
    #[default]
    Auto,
    /// A true multiply with what's under it: Frozen mode and captures.
    Multiply,
    /// Each channel's minimum with what's under it: the same as a DirectComposition MIN_BLEND
    /// layer shows live, for a capture that should match it exactly.
    Darken,
    /// A translucent tint at [`TINT_ALPHA`] of the colour's alpha: a live layer without a
    /// darken layer behind it.
    Tint,
    /// Not drawn here: they're on their own layer ([`render_highlights`]).
    Skip,
}

/// The highlighter's alpha as a tint on the live layer, as a share of its colour's alpha.
pub const TINT_ALPHA: f32 = 0.38;

/// How long an item takes to fade out once its delay is up, with disappearing ink.
pub const FADE_MS: u64 = 600;

/// The laser to draw, and how.
#[derive(Debug, Clone, Copy)]
pub struct LaserLook<'a> {
    pub laser: &'a Laser,
    pub colour: Colour,
    /// How long its trail lasts (Settings' `laser_trail_ms`).
    pub trail_ms: u32,
}

/// Everything about a render beyond the drawing, the target and the background.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions<'a> {
    pub view: View,
    pub highlight: Highlight,
    /// The time now, on the drawing's clock (as [`crate::drawing::InkPoint::time_ms`]): for
    /// disappearing ink and the laser.
    pub now_ms: u64,
    /// Disappearing ink: when set, items fade out this many milliseconds after they were drawn.
    pub disappearing_ink_ms: Option<u64>,
    /// The laser pointer, drawn over everything.
    pub laser: Option<LaserLook<'a>>,
}

impl RenderOptions<'_> {
    fn of(view: View) -> Self {
        RenderOptions {
            view,
            ..Default::default()
        }
    }
}

/// How visible an item is at `now_ms` with disappearing ink after `delay_ms`: 1 until the delay
/// is up, then falling to 0 over [`FADE_MS`]. Without disappearing ink, always 1. A stroke still
/// being drawn and a blur don't fade (a blur hides something).
pub fn opacity(item: &Item, now_ms: u64, disappearing_ink_ms: Option<u64>) -> f32 {
    let Some(delay) = disappearing_ink_ms else {
        return 1.0;
    };
    if !item.finished || item.tool == ToolKind::Blur {
        return 1.0;
    }
    let fade_start = item.drawn_at().saturating_add(delay);
    if now_ms <= fade_start {
        return 1.0;
    }
    let into = now_ms - fade_start;
    if into >= FADE_MS {
        0.0
    } else {
        1.0 - into as f32 / FADE_MS as f32
    }
}

/// Draws the whole drawing into `target`: the background, then each item in order.
pub fn render(drawing: &Drawing, target: &mut PixmapMut, view: View, background: Background) {
    render_with(drawing, target, &RenderOptions::of(view), background);
}

/// [`render`] with options.
pub fn render_with(
    drawing: &Drawing,
    target: &mut PixmapMut,
    options: &RenderOptions,
    background: Background,
) {
    let full =
        IntRect::from_xywh(0, 0, target.width(), target.height()).expect("a pixmap is never empty");
    paint_background(target, background, full);
    let has_image = matches!(background, Background::Image(_));
    paint(drawing, target, options, full, (0, 0), has_image, Pass::Ink);
}

/// Redraws one rectangle of `target`, leaving the rest as it was: exactly the pixels [`render`]
/// gives there. `region` is clipped to the target.
///
/// Returns the rectangle actually redrawn, **which is the one to show**: `region` clipped to
/// the target, and grown to cover whole any blur over a background image that it touches (each
/// of a blur's pixels depends on what's under it). `None` if `region` is outside the target.
pub fn render_region(
    drawing: &Drawing,
    target: &mut PixmapMut,
    view: View,
    background: Background,
    region: IntRect,
) -> Option<IntRect> {
    render_region_with(
        drawing,
        target,
        &RenderOptions::of(view),
        background,
        region,
    )
}

/// [`render_region`] with options. Returns the rectangle actually redrawn, as it does.
pub fn render_region_with(
    drawing: &Drawing,
    target: &mut PixmapMut,
    options: &RenderOptions,
    background: Background,
    region: IntRect,
) -> Option<IntRect> {
    region_pass(drawing, target, options, background, region, Pass::Ink)
}

/// Draws only the highlighter strokes, opaque in their colours (times disappearing ink's
/// fade), on transparent pixels: the content of a darken (MIN_BLEND) layer under the ink, so
/// the live highlighter darkens the screen as a real one would. Render the ink layer with
/// [`Highlight::Skip`] beside it.
pub fn render_highlights(drawing: &Drawing, target: &mut PixmapMut, options: &RenderOptions) {
    let full =
        IntRect::from_xywh(0, 0, target.width(), target.height()).expect("a pixmap is never empty");
    clear_rect(target, full);
    paint(
        drawing,
        target,
        options,
        full,
        (0, 0),
        false,
        Pass::Highlights,
    );
}

/// [`render_highlights`] for one rectangle, as [`render_region`]; returns the rectangle redrawn.
pub fn render_highlights_region(
    drawing: &Drawing,
    target: &mut PixmapMut,
    options: &RenderOptions,
    region: IntRect,
) -> Option<IntRect> {
    region_pass(
        drawing,
        target,
        options,
        Background::Transparent,
        region,
        Pass::Highlights,
    )
}

/// Which items a render draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Everything (the highlighter as [`RenderOptions::highlight`] says), then the laser.
    Ink,
    /// Only the highlighter, opaque.
    Highlights,
}

fn region_pass(
    drawing: &Drawing,
    target: &mut PixmapMut,
    options: &RenderOptions,
    background: Background,
    region: IntRect,
    pass: Pass,
) -> Option<IntRect> {
    let region = clip(region, target.width(), target.height())?;
    // A blur averages blocks of what's under it, so a change anywhere under a blur changes
    // pixels all over it: the redrawn region grows to cover every blur it touches, whole, and
    // every blur those touch in turn.
    let region = if pass == Pass::Ink && matches!(background, Background::Image(_)) {
        let grown = with_whole_blurs(drawing, options.view, region);
        clip(grown, target.width(), target.height()).unwrap_or(region)
    } else {
        region
    };
    // Draw into a scratch pixmap, offset so its pixels line up with the target's, then copy the
    // region in. Where a path leaves a pixmap, tiny-skia clips its edges, which can change the
    // anti-aliasing along each clipped edge. Paths are filled as straight edges no longer than
    // pen::MAX_EDGE, so that stays within MAX_EDGE (scaled) of the scratch's border: the scratch
    // reaches scratch_margin() past the region (but not past the target, whose own borders a
    // full render clips at too), so the copied pixels are exactly a full render's.
    let margin = scratch_margin(options.view);
    let wanted = IntRect::from_ltrb(
        region.left() - margin,
        region.top() - margin,
        region.right() + margin,
        region.bottom() + margin,
    )
    .unwrap_or(region);
    let grown = clip(wanted, target.width(), target.height()).unwrap_or(region);
    let mut scratch = Pixmap::new(grown.width(), grown.height())?;
    if let Background::Image(image) = background {
        copy_rect(image, grown, &mut scratch.as_mut(), 0, 0);
    }
    let has_image = matches!(background, Background::Image(_));
    paint(
        drawing,
        &mut scratch.as_mut(),
        options,
        grown,
        (grown.x(), grown.y()),
        has_image,
        pass,
    );
    let inner = IntRect::from_xywh(
        region.x() - grown.x(),
        region.y() - grown.y(),
        region.width(),
        region.height(),
    )
    .expect("the region is inside the scratch");
    copy_rect(scratch.as_ref(), inner, target, region.x(), region.y());
    Some(region)
}

/// `region` grown to cover, whole, every blur it overlaps, and every blur those overlap.
fn with_whole_blurs(drawing: &Drawing, view: View, region: IntRect) -> IntRect {
    let s = f64::from(view.scale);
    let blurs: Vec<IntRect> = drawing
        .drawn()
        .filter_map(|(_, g)| g.blur)
        .filter_map(|b| {
            let b = scale_rect(b, s);
            IntRect::from_ltrb(
                b.x0.floor() as i32,
                b.y0.floor() as i32,
                b.x1.ceil() as i32,
                b.y1.ceil() as i32,
            )
        })
        .collect();
    let mut grown = region;
    loop {
        let before = grown;
        for blur in &blurs {
            if overlaps(to_rect(*blur), to_rect(grown)) {
                grown = union_int(grown, *blur);
            }
        }
        if grown == before {
            return grown;
        }
    }
}

/// How far past a redrawn region the scratch pixmap reaches (see [`render_region`]): the longest
/// edge at this scale, plus room for anti-aliasing.
fn scratch_margin(view: View) -> i32 {
    let edge = crate::pen::MAX_EDGE * f64::from(view.scale.abs());
    if edge.is_finite() {
        (edge.ceil() as i32).saturating_add(4).min(4096)
    } else {
        4096
    }
}

/// The target pixels a drawing area covers, at `view`, grown to whole pixels and clipped to a
/// `width` × `height` target. `None` if it's empty or entirely outside.
pub fn dirty_rect(area: Rect, view: View, width: u32, height: u32) -> Option<IntRect> {
    let s = f64::from(view.scale);
    let x0 = (area.x0 * s).floor();
    let y0 = (area.y0 * s).floor();
    let x1 = (area.x1 * s).ceil();
    let y1 = (area.y1 * s).ceil();
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    let x0 = x0.max(0.0);
    let y0 = y0.max(0.0);
    let x1 = x1.min(f64::from(width));
    let y1 = y1.min(f64::from(height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    IntRect::from_ltrb(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
}

/// The area that changes on its own over time at `now_ms`, in drawing coordinates: items
/// fading out with disappearing ink, and the laser. Redraw it each frame while it's `Some`.
///
/// It includes what has finished fading but is still there to clear: an item faded to nothing
/// that [`crate::session::Session::prune_faded`] hasn't removed yet, and laser positions past
/// their trail that [`crate::session::Session::laser_prune`] hasn't dropped. So each frame:
/// take this area, redraw it, **then** prune; the last frame of a fade or a trail is then drawn
/// (clearing it) before it's forgotten.
pub fn animated_area(drawing: &Drawing, options: &RenderOptions) -> Option<Rect> {
    let mut area: Option<Rect> = None;
    let mut add = |r: Option<Rect>| {
        area = match (area, r) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        }
    };
    if let Some(delay) = options.disappearing_ink_ms {
        for (item, geometry) in drawing.drawn() {
            // Fading, faded and not yet removed, or starting within a frame or so.
            let soon = opacity(item, options.now_ms + 50, Some(delay));
            if soon < 1.0 {
                add(geometry.bounds);
            }
        }
    }
    if let Some(look) = options.laser {
        add(look.laser.painted_bounds());
    }
    area
}

fn clip(rect: IntRect, width: u32, height: u32) -> Option<IntRect> {
    let bounds = IntRect::from_xywh(0, 0, width, height)?;
    rect.intersect(&bounds)
}

fn union_int(a: IntRect, b: IntRect) -> IntRect {
    IntRect::from_ltrb(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
    .unwrap_or(a)
}

fn to_rect(r: IntRect) -> Rect {
    Rect::new(
        f64::from(r.left()),
        f64::from(r.top()),
        f64::from(r.right()),
        f64::from(r.bottom()),
    )
}

fn scale_rect(r: Rect, s: f64) -> Rect {
    Rect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

fn paint_background(target: &mut PixmapMut, background: Background, region: IntRect) {
    match background {
        Background::Transparent => clear_rect(target, region),
        Background::Image(image) => {
            clear_rect(target, region);
            copy_rect(image, region, target, region.x(), region.y());
        }
    }
}

/// Paints the items (and in the ink pass, the laser) into `target`, which holds `region` of
/// the full target with its top-left at `origin`.
fn paint(
    drawing: &Drawing,
    target: &mut PixmapMut,
    options: &RenderOptions,
    region: IntRect,
    origin: (i32, i32),
    has_image: bool,
    pass: Pass,
) {
    let view = options.view;
    let s = f64::from(view.scale);
    let transform = view
        .transform()
        .post_translate(-origin.0 as f32, -origin.1 as f32);
    let region_rect = to_rect(region);
    let highlight = match options.highlight {
        Highlight::Auto if has_image => Highlight::Multiply,
        Highlight::Auto => Highlight::Tint,
        other => other,
    };
    for (item, geometry) in drawing.drawn() {
        let Some(bounds) = geometry.bounds else {
            continue;
        };
        if !overlaps(scale_rect(bounds, s), region_rect) {
            continue;
        }
        let opacity = opacity(item, options.now_ms, options.disappearing_ink_ms);
        if opacity <= 0.0 {
            continue;
        }
        let is_highlighter = item.tool == ToolKind::Highlighter;
        match pass {
            Pass::Highlights if !is_highlighter => continue,
            Pass::Highlights => {
                fill(
                    target,
                    geometry,
                    item.colour,
                    opacity,
                    BlendMode::SourceOver,
                    transform,
                );
            }
            Pass::Ink if item.tool == ToolKind::Blur => {
                if has_image && let Some(blur) = geometry.blur {
                    let block = (f64::from(item.width) * s).max(2.0);
                    pixelate(target, scale_rect(blur, s), block, origin);
                }
            }
            Pass::Ink if is_highlighter => {
                let (alpha, blend) = match highlight {
                    Highlight::Skip => continue,
                    Highlight::Tint => (opacity * TINT_ALPHA, BlendMode::SourceOver),
                    Highlight::Darken => (opacity, BlendMode::Darken),
                    Highlight::Multiply | Highlight::Auto => (opacity, BlendMode::Multiply),
                };
                fill(target, geometry, item.colour, alpha, blend, transform);
            }
            Pass::Ink => fill(
                target,
                geometry,
                item.colour,
                opacity,
                BlendMode::SourceOver,
                transform,
            ),
        }
    }
    if pass == Pass::Ink
        && let Some(look) = options.laser
    {
        paint_laser(target, look, options.now_ms, transform);
    }
}

/// Fills an item's paths: its own colour, or the contrast colour for a stamp's number.
fn fill(
    target: &mut PixmapMut,
    geometry: &Geometry,
    colour: Colour,
    opacity: f32,
    blend_mode: BlendMode,
    transform: Transform,
) {
    for f in &geometry.fills {
        let c = match f.role {
            Role::Ink => colour,
            Role::Contrast if dark_on(colour) => Colour::rgb(0x21, 0x21, 0x21),
            Role::Contrast => Colour::rgb(255, 255, 255),
        };
        let mut paint = Paint {
            shader: Shader::SolidColor(skia_colour(c)),
            anti_alias: true,
            blend_mode,
            ..Paint::default()
        };
        paint.shader.apply_opacity(opacity.clamp(0.0, 1.0));
        target.fill_path(&f.path, &paint, FillRule::Winding, transform, None);
    }
}

/// Pixelates `rect` (target pixels) of `target`, whose top-left is at `origin` of the full
/// target: each block of the full target's grid becomes the average of its pixels inside `rect`.
fn pixelate(target: &mut PixmapMut, rect: Rect, block: f64, origin: (i32, i32)) {
    let block = block.round().max(2.0) as i32;
    let (w, h) = (target.width() as i32, target.height() as i32);
    // The rect in full-target pixels (whole pixels it covers), then the blocks over it.
    let (x0, y0) = (rect.x0.round() as i32, rect.y0.round() as i32);
    let (x1, y1) = (rect.x1.round() as i32, rect.y1.round() as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let width = target.width() as usize;
    let data = target.data_mut();
    let mut by = y0.div_euclid(block) * block;
    while by < y1 {
        let mut bx = x0.div_euclid(block) * block;
        while bx < x1 {
            // This block's pixels inside the rect, in scratch coordinates.
            let (cx0, cy0) = (bx.max(x0) - origin.0, by.max(y0) - origin.1);
            let (cx1, cy1) = (
                (bx + block).min(x1) - origin.0,
                (by + block).min(y1) - origin.1,
            );
            let (cx0, cy0, cx1, cy1) = (cx0.max(0), cy0.max(0), cx1.min(w), cy1.min(h));
            if cx1 > cx0 && cy1 > cy0 {
                let mut sum = [0u64; 4];
                for y in cy0..cy1 {
                    for x in cx0..cx1 {
                        let i = (y as usize * width + x as usize) * 4;
                        for c in 0..4 {
                            sum[c] += u64::from(data[i + c]);
                        }
                    }
                }
                let n = ((cx1 - cx0) * (cy1 - cy0)) as u64;
                let avg = sum.map(|v| ((v + n / 2) / n) as u8);
                for y in cy0..cy1 {
                    for x in cx0..cx1 {
                        let i = (y as usize * width + x as usize) * 4;
                        data[i..i + 4].copy_from_slice(&avg);
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
}

/// The laser: its trail as fading, shrinking dots, then a soft glow and the dot.
fn paint_laser(target: &mut PixmapMut, look: LaserLook, now_ms: u64, transform: Transform) {
    let disc = |target: &mut PixmapMut, at: Point, radius: f64, alpha: f32| {
        let Some(path) = crate::pen::to_skia(&Circle::new(at, radius).to_path(0.05)) else {
            return;
        };
        let mut paint = Paint {
            shader: Shader::SolidColor(skia_colour(look.colour)),
            anti_alias: true,
            ..Paint::default()
        };
        paint.shader.apply_opacity(alpha.clamp(0.0, 1.0));
        target.fill_path(&path, &paint, FillRule::Winding, transform, None);
    };
    for (at, age) in look.laser.visible_trail(now_ms, look.trail_ms) {
        let left = 1.0 - age;
        disc(
            target,
            at,
            laser::DOT_RADIUS * (0.4 + 0.6 * left),
            (0.6 * left) as f32,
        );
    }
    if let Some(at) = look.laser.dot() {
        disc(target, at, laser::GLOW_RADIUS, 0.25);
        disc(target, at, laser::DOT_RADIUS, 1.0);
    }
}

/// A straight-alpha colour as tiny-skia's.
pub fn skia_colour(c: Colour) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn clear_rect(target: &mut PixmapMut, region: IntRect) {
    let width = target.width() as usize;
    let data = target.data_mut();
    for y in region.top()..region.bottom() {
        let row = (y as usize * width + region.left() as usize) * 4;
        data[row..row + region.width() as usize * 4].fill(0);
    }
}

/// Copies `rect` of `src` to `dst` with its top-left at (`x`, `y`), replacing what's there.
/// Whatever falls outside either pixmap is skipped; pixels of `rect` outside `src` are
/// transparent.
fn copy_rect(src: PixmapRef, rect: IntRect, dst: &mut PixmapMut, x: i32, y: i32) {
    let (src_w, src_h) = (src.width() as i32, src.height() as i32);
    let (dst_w, dst_h) = (dst.width() as i32, dst.height() as i32);
    let src_data = src.data();
    let dst_width = dst.width() as usize;
    let dst_data = dst.data_mut();
    for row in 0..rect.height() as i32 {
        let (sy, dy) = (rect.y() + row, y + row);
        if dy < 0 || dy >= dst_h {
            continue;
        }
        for (sx0, dx0, len) in spans(rect.x(), x, rect.width() as i32, src_w, dst_w) {
            let d = (dy as usize * dst_width + dx0 as usize) * 4;
            let out = &mut dst_data[d..d + len as usize * 4];
            if sy < 0 || sy >= src_h || sx0 < 0 {
                out.fill(0);
            } else {
                let s = (sy as usize * src_w as usize + sx0 as usize) * 4;
                out.copy_from_slice(&src_data[s..s + len as usize * 4]);
            }
        }
    }
}

/// A row's spans: (source x, destination x, length), clipped to the destination, with the part
/// outside the source split off (its source x is negative, meaning transparent).
fn spans(src_x: i32, dst_x: i32, len: i32, src_w: i32, dst_w: i32) -> Vec<(i32, i32, i32)> {
    // Clip to the destination.
    let start = 0.max(-dst_x);
    let end = len.min(dst_w - dst_x);
    if end <= start {
        return Vec::new();
    }
    // Split by the source's columns: before 0, inside, and at or past its width.
    let inside_start = start.max(-src_x).min(end);
    let inside_end = end.min(src_w - src_x).max(inside_start);
    let mut out = Vec::new();
    if inside_start > start {
        out.push((-1, dst_x + start, inside_start - start));
    }
    if inside_end > inside_start {
        out.push((
            src_x + inside_start,
            dst_x + inside_start,
            inside_end - inside_start,
        ));
    }
    if end > inside_end {
        out.push((-1, dst_x + inside_end, end - inside_end));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::InkPoint;
    use crate::tool::ToolKind;

    fn pixmap(w: u32, h: u32) -> Pixmap {
        Pixmap::new(w, h).unwrap()
    }

    fn wave(drawing: &mut Drawing, n: u32, colour: Colour) -> crate::drawing::ItemId {
        let at = |i: u32| {
            let t = f64::from(i);
            InkPoint::pen(
                20.0 + t * 4.0,
                60.0 + (t / 6.0).sin() * 30.0,
                (0.3 + (t / 9.0).sin().abs() * 0.7) as f32,
                u64::from(i) * 8,
            )
        };
        let (id, _) = drawing.begin(ToolKind::Pen, colour, 10.0, at(0));
        for i in 1..n {
            drawing.push_point(id, at(i));
        }
        id
    }

    #[test]
    fn the_layer_is_transparent_where_theres_no_ink() {
        let mut d = Drawing::new();
        wave(&mut d, 40, Colour::rgb(255, 0, 0));
        let mut px = pixmap(240, 120);
        px.fill(Color::from_rgba8(1, 2, 3, 4));
        render(
            &d,
            &mut px.as_mut(),
            View::default(),
            Background::Transparent,
        );
        assert_eq!(px.pixel(0, 0).unwrap().alpha(), 0);
        let painted = px.pixels().iter().filter(|p| p.alpha() == 255).count();
        assert!(painted > 500, "{painted}");
        // Ink pixels are the stroke's colour.
        let p = px.pixels().iter().find(|p| p.alpha() == 255).unwrap();
        assert_eq!((p.red(), p.green(), p.blue()), (255, 0, 0));
    }

    #[test]
    fn over_an_image_the_image_shows_around_the_ink() {
        let mut d = Drawing::new();
        wave(&mut d, 40, Colour::rgb(0, 0, 255));
        let mut screen = pixmap(240, 120);
        screen.fill(Color::from_rgba8(200, 200, 200, 255));
        let mut px = pixmap(240, 120);
        render(
            &d,
            &mut px.as_mut(),
            View::default(),
            Background::Image(screen.as_ref()),
        );
        let corner = px.pixel(0, 0).unwrap();
        assert_eq!((corner.red(), corner.alpha()), (200, 255));
        assert!(px.pixels().iter().all(|p| p.alpha() == 255));
        assert!(px.pixels().iter().any(|p| p.blue() == 255 && p.red() == 0));
    }

    #[test]
    fn a_region_redraw_matches_a_full_render() {
        let mut d = Drawing::new();
        wave(&mut d, 30, Colour::rgb(255, 0, 0));
        let id = wave(
            &mut d,
            30,
            Colour {
                r: 0,
                g: 128,
                b: 0,
                a: 160,
            },
        );
        let mut screen = pixmap(240, 130);
        for (i, p) in screen.data_mut().chunks_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, (i % 13 * 19) as u8, 90, 255]);
        }
        for background in [Background::Transparent, Background::Image(screen.as_ref())] {
            for scale in [1.0, 1.5] {
                let view = View { scale };
                let (w, h) = ((240.0 * scale) as u32, (130.0 * scale) as u32);
                let mut live = pixmap(w, h);
                render(&d, &mut live.as_mut(), view, background);
                // Grow the last stroke, redrawing only what changed.
                for i in 0..20 {
                    let p = InkPoint::pen(140.0 + f64::from(i) * 3.0, 100.0, 0.8, 1000);
                    let area = d.push_point(id, p).unwrap();
                    let rect = dirty_rect(area, view, w, h).unwrap();
                    render_region(&d, &mut live.as_mut(), view, background, rect);
                }
                let area = d.finish(id);
                if let Some(rect) = area.and_then(|a| dirty_rect(a, view, w, h)) {
                    render_region(&d, &mut live.as_mut(), view, background, rect);
                }
                let mut full = pixmap(w, h);
                render(&d, &mut full.as_mut(), view, background);
                let (differ, worst) = compare(&live, &full);
                assert!(
                    differ == 0,
                    "scale {scale}: {differ} pixels differ, by up to {worst}"
                );
                // Undo the growth for the next round.
                let (mut item, at, _) = d.remove(id).unwrap();
                item.points.truncate(30);
                item.finished = false;
                d.insert(at, item);
            }
        }
    }

    /// How many pixels differ, and the largest difference in any channel.
    fn compare(a: &Pixmap, b: &Pixmap) -> (usize, u8) {
        a.data()
            .chunks(4)
            .zip(b.data().chunks(4))
            .map(|(p, q)| {
                p.iter()
                    .zip(q)
                    .map(|(x, y)| x.abs_diff(*y))
                    .max()
                    .unwrap_or(0)
            })
            .filter(|d| *d > 0)
            .fold((0, 0), |(n, worst), d| (n + 1, worst.max(d)))
    }

    #[test]
    fn a_whole_region_redraw_is_exactly_a_render() {
        let mut d = Drawing::new();
        wave(&mut d, 40, Colour::rgb(255, 0, 0));
        let mut full = pixmap(240, 130);
        render(
            &d,
            &mut full.as_mut(),
            View::default(),
            Background::Transparent,
        );
        let mut region = pixmap(240, 130);
        let all = IntRect::from_xywh(0, 0, 240, 130).unwrap();
        render_region(
            &d,
            &mut region.as_mut(),
            View::default(),
            Background::Transparent,
            all,
        );
        assert_eq!(compare(&full, &region), (0, 0));
    }

    /// A long scribble changing direction at random, pressure too, each
    /// sample redrawn alone, then finished. The live pixels must be exactly a full render's,
    /// at any scale and width.
    #[test]
    fn a_long_random_scribble_redrawn_piecemeal_is_exact() {
        for (scale, width, samples) in [
            (1.0, 8.0, 600),
            (0.5, 8.0, 300),
            (1.5, 1.0, 300),
            (2.0, 80.0, 150),
            (1.0, 200.0, 100),
        ] {
            let view = View { scale };
            let (w, h) = ((512.0 * scale) as u32, (384.0 * scale) as u32);
            let mut d = Drawing::new();
            let (mut x, mut y) = (200.0f64, 200.0f64);
            let (id, _) = d.begin(
                ToolKind::Pen,
                Colour::rgb(255, 0, 0),
                width,
                InkPoint::pen(x, y, 0.5, 0),
            );
            let mut live = pixmap(w, h);
            render(&d, &mut live.as_mut(), view, Background::Transparent);
            let mut seed: u64 = 97;
            let mut next = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 32) as f64 / f64::from(u32::MAX)
            };
            let redraw = |d: &Drawing, live: &mut Pixmap, area: Option<Rect>| {
                if let Some(rect) = area.and_then(|a| dirty_rect(a, view, w, h)) {
                    render_region(d, &mut live.as_mut(), view, Background::Transparent, rect);
                }
            };
            for i in 1..samples {
                x = (x + (next() - 0.5) * 100.0).clamp(20.0, 450.0);
                y = (y + (next() - 0.5) * 100.0).clamp(20.0, 330.0);
                let p = InkPoint::pen(x, y, next() as f32, i);
                let area = d.push_point(id, p);
                redraw(&d, &mut live, area);
            }
            let area = d.finish(id);
            redraw(&d, &mut live, area);
            let mut full = pixmap(w, h);
            render(&d, &mut full.as_mut(), view, Background::Transparent);
            let (differ, worst) = compare(&live, &full);
            assert_eq!(
                (differ, worst),
                (0, 0),
                "scale {scale}, width {width}: {differ} pixels differ, by up to {worst}"
            );
        }
    }

    #[test]
    fn scaling_draws_the_same_shape_bigger() {
        let mut d = Drawing::new();
        wave(&mut d, 40, Colour::rgb(0, 0, 0));
        let mut one = pixmap(240, 120);
        let mut two = pixmap(480, 240);
        render(
            &d,
            &mut one.as_mut(),
            View::default(),
            Background::Transparent,
        );
        render(
            &d,
            &mut two.as_mut(),
            View { scale: 2.0 },
            Background::Transparent,
        );
        let ink = |p: &Pixmap| p.pixels().iter().filter(|p| p.alpha() > 127).count() as f64;
        let ratio = ink(&two) / ink(&one);
        assert!((3.5..4.5).contains(&ratio), "{ratio}");
    }

    #[test]
    fn dirty_rects_grow_to_pixels_and_clip() {
        let v = View::default();
        let r = dirty_rect(Rect::new(10.2, 20.7, 30.1, 40.0), v, 100, 100).unwrap();
        assert_eq!((r.left(), r.top(), r.right(), r.bottom()), (10, 20, 31, 40));
        let r = dirty_rect(Rect::new(-50.0, -5.0, 20.0, 500.0), v, 100, 100).unwrap();
        assert_eq!((r.left(), r.top(), r.right(), r.bottom()), (0, 0, 20, 100));
        assert!(dirty_rect(Rect::new(200.0, 0.0, 300.0, 10.0), v, 100, 100).is_none());
        let r = dirty_rect(
            Rect::new(10.0, 10.0, 20.0, 20.0),
            View { scale: 2.0 },
            100,
            100,
        );
        assert_eq!(r.unwrap().width(), 20);
    }

    #[test]
    fn copy_rect_clips_at_every_edge() {
        let mut src = pixmap(4, 4);
        src.fill(Color::from_rgba8(255, 255, 255, 255));
        let mut dst = pixmap(4, 4);
        dst.fill(Color::from_rgba8(0, 0, 0, 255));
        // Half the rectangle is outside the source (transparent) and it runs off the target.
        copy_rect(
            src.as_ref(),
            IntRect::from_xywh(2, 0, 4, 2).unwrap(),
            &mut dst.as_mut(),
            1,
            3,
        );
        let a = |x, y| dst.pixel(x, y).unwrap().alpha();
        let r = |x, y| dst.pixel(x, y).unwrap().red();
        assert_eq!((r(1, 3), r(2, 3)), (255, 255));
        assert_eq!((a(3, 3), a(0, 3)), (0, 255));
        assert_eq!(r(1, 2), 0);
    }
}
