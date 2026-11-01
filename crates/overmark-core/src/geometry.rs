//! Each tool's shape: what an item fills, for drawing, hit-testing and its bounds.
//!
//! - **Pen:** the perfect-freehand outline, pressure or speed setting the width ([`crate::pen`]).
//! - **Highlighter:** the same outline at a constant width with flat ends (a chisel tip); how
//!   it blends is the renderer's ([`crate::render::Highlight`]).
//! - **Line, rectangle, ellipse:** outlines of the item's width, from its first point to its
//!   last ([`constrain`] and [`snap`] give squares, circles and 45° lines).
//! - **Arrow:** a shaft and a head that grows with the width ([`arrow`]).
//! - **Text:** the glyphs of its text ([`crate::text`]), its first point the top-left.
//! - **Numbered stamp:** a disc the width across, its number in white or black, whichever
//!   reads better on its colour.
//! - **Blur:** a rectangle, pixelated by the renderer over a background image only.
//! - **Eraser, laser:** nothing (they never become items).

use kurbo::{Affine, BezPath, Cap, Circle, Ellipse, Join, Line, Point, Rect, Shape, Stroke, Vec2};

use crate::colour::Colour;
use crate::drawing::Item;
use crate::pen;
use crate::text;
use crate::tool::ToolKind;

/// How a fill is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The item's colour.
    Ink,
    /// White or black, whichever reads on the item's colour (a stamp's number).
    Contrast,
}

/// One filled path of an item.
#[derive(Debug, Clone)]
pub(crate) struct Fill {
    /// For tiny-skia: straight edges no longer than `pen::MAX_EDGE`.
    pub path: tiny_skia::Path,
    pub role: Role,
}

/// An item's shape, kept with it in the drawing.
#[derive(Debug, Clone, Default)]
pub(crate) struct Geometry {
    /// A pen or highlighter stroke's outline polygon, to find what changed as it grows; empty
    /// for other tools.
    pub outline: Vec<Point>,
    /// What's filled, in order.
    pub fills: Vec<Fill>,
    /// Closed polygons the eraser tests against (non-zero winding).
    pub contours: Vec<Vec<Point>>,
    /// A blur item's rectangle.
    pub blur: Option<Rect>,
    /// Everything the item paints, anti-aliasing included.
    pub bounds: Option<Rect>,
}

/// How far anti-aliasing reaches beyond a shape, in pixels, with a margin for rounding.
pub(crate) const AA_MARGIN: f64 = 2.0;

/// The geometry of an item.
pub(crate) fn of(item: &Item) -> Geometry {
    let colour_fills = |paths: Vec<(BezPath, Role)>, outline: Vec<Point>| {
        let mut geometry = Geometry {
            outline,
            ..Geometry::default()
        };
        for (path, role) in paths {
            if let Some(skia) = pen::to_skia(&path) {
                geometry.contours.extend(contours(&skia));
                geometry.fills.push(Fill { path: skia, role });
            }
        }
        geometry.bounds = bounds_of(geometry.contours.iter().flatten());
        geometry
    };
    let first = item.points.first().map(|p| p.position());
    let last = item.points.last().map(|p| p.position());
    let width = f64::from(item.width.max(0.5));
    match (item.tool, first, last) {
        (ToolKind::Pen | ToolKind::Highlighter, Some(_), _) => {
            let outline = pen::outline(item);
            let path = pen::smooth_path(&outline);
            colour_fills(vec![(path, Role::Ink)], outline)
        }
        (ToolKind::Line, Some(a), Some(b)) => {
            let style = Stroke::new(width).with_caps(Cap::Round);
            colour_fills(
                vec![(stroke(Line::new(a, b), &style), Role::Ink)],
                Vec::new(),
            )
        }
        (ToolKind::Rectangle, Some(a), Some(b)) => {
            let style = Stroke::new(width).with_join(Join::Miter);
            let rect = Rect::from_points(a, b);
            colour_fills(vec![(stroke(rect, &style), Role::Ink)], Vec::new())
        }
        (ToolKind::Ellipse, Some(a), Some(b)) => {
            let style = Stroke::new(width);
            let ellipse = Ellipse::from_rect(Rect::from_points(a, b));
            colour_fills(vec![(stroke(ellipse, &style), Role::Ink)], Vec::new())
        }
        (ToolKind::Arrow, Some(a), Some(b)) => {
            let polygon = arrow(a, b, width);
            colour_fills(vec![(polygon_path(&polygon), Role::Ink)], Vec::new())
        }
        (ToolKind::Text, Some(at), _) => {
            let content = item.text.as_ref().map_or("", |t| t.text.as_str());
            let shape = text::shape(content, item.width);
            let glyphs = Affine::translate(at.to_vec2()) * shape.glyphs;
            colour_fills(vec![(glyphs, Role::Ink)], Vec::new())
        }
        (ToolKind::Stamp, Some(at), _) => {
            let radius = width / 2.0;
            let disc = Circle::new(at, radius).to_path(0.01);
            let mut fills = vec![(disc, Role::Ink)];
            if let Some(n) = item.number {
                let label = n.to_string();
                let shape = text::shape(&label, (width * 0.6) as f32);
                // Centre the number's ink, not its line box, in the disc.
                if let Some(ink) = bounds_of_path(&shape.glyphs) {
                    let offset = at.to_vec2() - ink.center().to_vec2();
                    fills.push((Affine::translate(offset) * shape.glyphs, Role::Contrast));
                }
            }
            colour_fills(fills, Vec::new())
        }
        (ToolKind::Blur, Some(a), Some(b)) => {
            let rect = Rect::from_points(a, b);
            Geometry {
                contours: vec![vec![
                    Point::new(rect.x0, rect.y0),
                    Point::new(rect.x1, rect.y0),
                    Point::new(rect.x1, rect.y1),
                    Point::new(rect.x0, rect.y1),
                ]],
                blur: (rect.area() > 0.0).then_some(rect),
                bounds: (rect.area() > 0.0).then_some(rect),
                ..Geometry::default()
            }
        }
        _ => Geometry::default(),
    }
}

/// `path` stroked with `style`, as a path to fill.
fn stroke(shape: impl Shape, style: &Stroke) -> BezPath {
    kurbo::stroke(
        shape.path_elements(0.05),
        style,
        &kurbo::StrokeOpts::default(),
        0.05,
    )
}

fn polygon_path(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    if let Some((first, rest)) = points.split_first() {
        path.move_to(*first);
        for p in rest {
            path.line_to(*p);
        }
        path.close_path();
    }
    path
}

/// The closed polygons of a flattened tiny-skia path.
fn contours(path: &tiny_skia::Path) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let pt = |p: tiny_skia::Point| Point::new(f64::from(p.x), f64::from(p.y));
    for segment in path.segments() {
        match segment {
            tiny_skia::PathSegment::MoveTo(p) => out.push(vec![pt(p)]),
            tiny_skia::PathSegment::LineTo(p) => {
                if let Some(c) = out.last_mut() {
                    c.push(pt(p));
                }
            }
            // `pen::to_skia` gives straight edges only.
            tiny_skia::PathSegment::QuadTo(_, p) | tiny_skia::PathSegment::CubicTo(_, _, p) => {
                if let Some(c) = out.last_mut() {
                    c.push(pt(p));
                }
            }
            tiny_skia::PathSegment::Close => {}
        }
    }
    out.retain(|c| c.len() > 1);
    out
}

fn bounds_of<'a>(points: impl Iterator<Item = &'a Point>) -> Option<Rect> {
    let mut points = points;
    let first = *points.next()?;
    let rect = points.fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
    Some(rect.inflate(AA_MARGIN, AA_MARGIN))
}

fn bounds_of_path(path: &BezPath) -> Option<Rect> {
    (!path.elements().is_empty()).then(|| path.bounding_box())
}

/// The arrow from `tail` to `tip`, `width` thick: a polygon (shaft and head). The head is
/// [`ARROW_HEAD_LENGTH`] widths long and [`ARROW_HEAD_WIDTH`] widths across, at least 10 px
/// long, and never longer than the arrow (a very short arrow is all head).
pub fn arrow(tail: Point, tip: Point, width: f64) -> Vec<Point> {
    let along = tip - tail;
    let length = along.hypot();
    let width = width.max(0.5);
    if length < 1e-6 {
        // A click: a small square, so it can be seen and erased.
        let h = width / 2.0;
        return vec![
            tip + Vec2::new(-h, -h),
            tip + Vec2::new(h, -h),
            tip + Vec2::new(h, h),
            tip + Vec2::new(-h, h),
        ];
    }
    let dir = along / length;
    let normal = Vec2::new(-dir.y, dir.x);
    let head_length = (width * ARROW_HEAD_LENGTH).max(10.0).min(length);
    let head_half = (width * ARROW_HEAD_WIDTH / 2.0).max(head_length * 0.4);
    let shaft_half = width / 2.0;
    let base = tip - dir * head_length;
    vec![
        tail + normal * shaft_half,
        base + normal * shaft_half,
        base + normal * head_half,
        tip,
        base - normal * head_half,
        base - normal * shaft_half,
        tail - normal * shaft_half,
    ]
}

/// An arrow head's length, in widths.
pub const ARROW_HEAD_LENGTH: f64 = 4.0;
/// An arrow head's width across, in widths.
pub const ARROW_HEAD_WIDTH: f64 = 3.5;

/// A rectangle or ellipse drag made square (Shift): `to` moved so the box from `from` is as
/// wide as it is tall, the larger side kept, in the direction dragged.
pub fn constrain(from: Point, to: Point) -> Point {
    let d = to - from;
    let side = d.x.abs().max(d.y.abs());
    let sign = |v: f64| if v < 0.0 { -1.0 } else { 1.0 };
    Point::new(from.x + side * sign(d.x), from.y + side * sign(d.y))
}

/// A line or arrow drag snapped to the nearest 45° (Shift): `to` moved onto that direction
/// from `from`, at the same length.
pub fn snap(from: Point, to: Point) -> Point {
    let d = to - from;
    let length = d.hypot();
    if length == 0.0 {
        return to;
    }
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (d.y.atan2(d.x) / step).round() * step;
    let snapped = Point::new(from.x + length * angle.cos(), from.y + length * angle.sin());
    // Exactly on the axes, not 1e-15 off them.
    Point::new(clean(snapped.x, from.x), clean(snapped.y, from.y))
}

fn clean(v: f64, origin: f64) -> f64 {
    if (v - origin).abs() < 1e-9 { origin } else { v }
}

/// Whether a stamp's number should be dark (on a light colour) rather than white.
pub(crate) fn dark_on(colour: Colour) -> bool {
    let lin = |c: u8| f64::from(c) / 255.0;
    let luminance = 0.2126 * lin(colour.r) + 0.7152 * lin(colour.g) + 0.0722 * lin(colour.b);
    luminance > 0.6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn constrain_makes_squares_in_the_dragged_direction() {
        assert_eq!(constrain(p(0.0, 0.0), p(30.0, 10.0)), p(30.0, 30.0));
        assert_eq!(constrain(p(0.0, 0.0), p(-10.0, 25.0)), p(-25.0, 25.0));
        assert_eq!(constrain(p(5.0, 5.0), p(-5.0, -2.0)), p(-5.0, -5.0));
        assert_eq!(constrain(p(1.0, 1.0), p(1.0, 1.0)), p(1.0, 1.0));
    }

    #[test]
    fn snap_rounds_to_45_degrees_keeping_length() {
        let flat = snap(p(0.0, 0.0), p(100.0, 10.0));
        assert_eq!(flat.y, 0.0);
        assert!((flat.x - 100.0f64.hypot(10.0)).abs() < 1e-9, "{flat:?}");
        assert_eq!(snap(p(0.0, 0.0), p(5.0, -90.0)).x, 0.0);
        let d = snap(p(10.0, 10.0), p(60.0, 52.0));
        assert!(((d.x - 10.0) - (d.y - 10.0)).abs() < 1e-9, "{d:?}");
        let len = (d - p(10.0, 10.0)).hypot();
        assert!((len - (p(60.0, 52.0) - p(10.0, 10.0)).hypot()).abs() < 1e-9);
        assert_eq!(snap(p(3.0, 3.0), p(3.0, 3.0)), p(3.0, 3.0));
    }

    #[test]
    fn the_arrow_head_grows_with_the_width() {
        let head = |w: f64| {
            let a = arrow(p(0.0, 0.0), p(200.0, 0.0), w);
            // Points 2 and 4 are the head's corners; the tip is point 3.
            assert_eq!(a[3], p(200.0, 0.0));
            (200.0 - a[2].x, a[2].y - a[4].y)
        };
        let (thin_len, thin_across) = head(2.0);
        let (thick_len, thick_across) = head(8.0);
        assert_eq!(thin_len, 10.0, "at least 10 px long");
        assert_eq!(thick_len, 8.0 * ARROW_HEAD_LENGTH);
        assert!(thick_across > thin_across);
        assert_eq!(thick_across, 8.0 * ARROW_HEAD_WIDTH);
        // The shaft is the width across.
        let a = arrow(p(0.0, 0.0), p(200.0, 0.0), 8.0);
        assert_eq!(a[0].y - a[6].y, 8.0);
    }

    #[test]
    fn a_short_arrow_is_all_head() {
        let a = arrow(p(0.0, 0.0), p(6.0, 0.0), 4.0);
        assert_eq!(a[1].x, 0.0, "the head starts at the tail");
        assert!(arrow(p(5.0, 5.0), p(5.0, 5.0), 4.0).len() == 4);
    }

    #[test]
    fn stamp_numbers_contrast_with_their_colour() {
        assert!(dark_on(Colour::rgb(0xFD, 0xD8, 0x35)), "dark on yellow");
        assert!(dark_on(Colour::rgb(255, 255, 255)));
        assert!(!dark_on(Colour::rgb(0xE5, 0x39, 0x35)), "white on red");
        assert!(!dark_on(Colour::rgb(0x21, 0x21, 0x21)));
    }
}
