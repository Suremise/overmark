//! The pen: a stroke's outline from its points, and the curves that fill it.
//!
//! The width comes from [`crate::freehand`] (the perfect-freehand port). A pen's pressure sets
//! it; with a mouse there is none, and the distance between samples (the speed, at the
//! mouse's steady sampling rate) sets it instead. The outline's polygon is smoothed with
//! `kurbo` into quadratic curves through the midpoints of its edges, as perfect-freehand
//! recommends, so the edge looks drawn rather than faceted.

use kurbo::{BezPath, PathEl, Point};

use crate::drawing::Item;
use crate::freehand::{self, InputPoint, StrokeOptions};

/// How much pressure narrows a pen stroke (perfect-freehand's `thinning`).
pub const PEN_THINNING: f64 = 0.6;
/// How much a mouse's speed narrows a stroke: less than a pen's pressure, so a quick line isn't
/// a hairline.
pub const MOUSE_THINNING: f64 = 0.5;
/// How much the outline's edge is softened.
pub const SMOOTHING: f64 = 0.5;
/// How much the input is smoothed: higher is smoother but lags the pointer more.
pub const STREAMLINE: f64 = 0.45;

/// The options a stroke is drawn with: the pen's, or the highlighter's (a constant width with
/// flat ends, like a chisel tip).
pub fn options(item: &Item) -> StrokeOptions {
    if item.tool == crate::tool::ToolKind::Highlighter {
        return StrokeOptions {
            size: f64::from(item.width),
            thinning: 0.0,
            smoothing: SMOOTHING,
            streamline: STREAMLINE,
            simulate_pressure: false,
            start: freehand::StrokeEnd {
                cap: false,
                ..freehand::StrokeEnd::START
            },
            end: freehand::StrokeEnd {
                cap: false,
                ..freehand::StrokeEnd::END
            },
            last: item.finished,
            ..StrokeOptions::default()
        };
    }
    let has_pressure = item.points.iter().any(|p| p.pressure.is_some());
    StrokeOptions {
        size: f64::from(item.width),
        thinning: if has_pressure {
            PEN_THINNING
        } else {
            MOUSE_THINNING
        },
        smoothing: SMOOTHING,
        streamline: STREAMLINE,
        simulate_pressure: !has_pressure,
        last: item.finished,
        ..StrokeOptions::default()
    }
}

/// A stroke's outline: a closed polygon in drawing coordinates.
pub fn outline(item: &Item) -> Vec<Point> {
    let input: Vec<InputPoint> = item
        .points
        .iter()
        .map(|p| InputPoint {
            x: p.x,
            y: p.y,
            pressure: p.pressure.map(f64::from),
        })
        .collect();
    freehand::stroke_outline(&input, &options(item))
}

/// A closed polygon smoothed into quadratic curves: each edge's midpoint to the next, bending
/// at the corner between them. Empty for fewer than three points.
pub fn smooth_path(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let n = points.len();
    if n < 3 {
        return path;
    }
    let mid = |a: Point, b: Point| a.midpoint(b);
    path.move_to(mid(points[n - 1], points[0]));
    for i in 0..n {
        let next = points[(i + 1) % n];
        path.quad_to(points[i], mid(points[i], next));
    }
    path.close_path();
    path
}

/// How closely the straight edges filled follow the curves, in drawing pixels.
pub const FLATTEN_TOLERANCE: f64 = 0.05;

/// The longest straight edge handed to tiny-skia, in drawing pixels.
///
/// Why it matters: where a path leaves a pixmap, tiny-skia clips it, and the clipped edge's
/// anti-aliasing can differ slightly from the unclipped edge's. That disturbs only the pixels
/// along that one edge. With every edge this short, the disturbance stays within
/// `MAX_EDGE` (times the view's scale) of the pixmap's border, which
/// [`crate::render::render_region`] keeps outside the pixels it copies: so a region redraw is
/// exactly what a full render gives. (Curves would be split by tiny-skia itself, and a split
/// curve moves along its whole length.)
pub const MAX_EDGE: f64 = 4.0;

/// A path as tiny-skia's, in straight edges no longer than [`MAX_EDGE`], or `None` if it's
/// empty or not finite. The same edges serve every render, full or partial.
pub(crate) fn to_skia(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    let line_to = |builder: &mut tiny_skia::PathBuilder, from: Point, to: Point| {
        let pieces = ((from.distance(to) / MAX_EDGE).ceil() as usize).max(1);
        for k in 1..=pieces {
            let p = from.lerp(to, k as f64 / pieces as f64);
            builder.line_to(p.x as f32, p.y as f32);
        }
    };
    kurbo::flatten(
        path.elements().iter().copied(),
        FLATTEN_TOLERANCE,
        |el| match el {
            PathEl::MoveTo(p) => {
                builder.move_to(p.x as f32, p.y as f32);
                last = p;
                start = p;
            }
            PathEl::LineTo(p) => {
                line_to(&mut builder, last, p);
                last = p;
            }
            PathEl::ClosePath => {
                line_to(&mut builder, last, start);
                builder.close();
                last = start;
            }
            // flatten only gives moves, lines and closes.
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        },
    );
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::Colour;
    use crate::drawing::{InkPoint, ItemId};
    use crate::tool::ToolKind;

    fn item(points: Vec<InkPoint>, finished: bool) -> Item {
        Item {
            id: ItemId(0),
            tool: ToolKind::Pen,
            colour: Colour::rgb(0, 0, 0),
            width: 10.0,
            points,
            finished,
            number: None,
            text: None,
        }
    }

    fn line(n: u32, f: impl Fn(f64) -> InkPoint) -> Vec<InkPoint> {
        (0..n).map(|i| f(f64::from(i))).collect()
    }

    fn widest(outline: &[Point], y: f64) -> f64 {
        outline.iter().map(|p| (p.y - y).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn a_mouse_simulates_pressure_and_a_pen_uses_its_own() {
        let mouse = item(line(5, |i| InkPoint::mouse(i * 10.0, 0.0, 0)), false);
        assert!(options(&mouse).simulate_pressure);
        let pen = item(line(5, |i| InkPoint::pen(i * 10.0, 0.0, 0.5, 0)), true);
        let o = options(&pen);
        assert!(!o.simulate_pressure && o.last);
        assert_eq!(o.size, 10.0);
    }

    #[test]
    fn harder_pressure_draws_wider() {
        let at = |p: f32| {
            let it = item(line(40, |i| InkPoint::pen(i * 5.0, 50.0, p, 0)), true);
            widest(&outline(&it), 50.0)
        };
        let (light, firm) = (at(0.1), at(1.0));
        assert!(firm > light * 2.0, "{light} vs {firm}");
        assert!(firm <= 10.0, "{firm}");
    }

    #[test]
    fn a_faster_mouse_draws_thinner() {
        // The same line, sampled densely (slow) and sparsely (fast).
        let at = |step: f64| {
            let n = (400.0 / step) as u32;
            let it = item(line(n, |i| InkPoint::mouse(i * step, 50.0, 0)), true);
            widest(&outline(&it)[..], 50.0)
        };
        let (slow, fast) = (at(2.0), at(40.0));
        assert!(slow > fast, "{slow} vs {fast}");
    }

    #[test]
    fn the_smoothed_path_is_closed_and_passes_the_midpoints() {
        let square = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let path = smooth_path(&square);
        let els = path.elements();
        assert_eq!(els.len(), 6);
        assert_eq!(els[0], PathEl::MoveTo(Point::new(0.0, 5.0)));
        assert_eq!(
            els[1],
            PathEl::QuadTo(Point::new(0.0, 0.0), Point::new(5.0, 0.0))
        );
        assert_eq!(els[5], PathEl::ClosePath);
        assert!(smooth_path(&square[..2]).elements().is_empty());
        assert!(to_skia(&path).is_some());
        assert!(to_skia(&BezPath::new()).is_none());
    }

    #[test]
    fn filled_edges_are_short_lines() {
        use kurbo::Shape as _;
        let circle = kurbo::Circle::new((50.0, 50.0), 40.0).to_path(0.01);
        let mut bez = BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.line_to((100.0, 0.0)); // long: split
        bez.quad_to((150.0, 50.0), (100.0, 100.0));
        bez.close_path(); // back to (0, 0): long, split
        for path in [circle, bez] {
            let skia = to_skia(&path).unwrap();
            let mut last = None;
            for seg in skia.segments() {
                match seg {
                    tiny_skia::PathSegment::MoveTo(p) => last = Some(p),
                    tiny_skia::PathSegment::LineTo(p) => {
                        let a = last.unwrap();
                        let d = ((a.x - p.x).powi(2) + (a.y - p.y).powi(2)).sqrt();
                        assert!(f64::from(d) <= MAX_EDGE + 1e-3, "{d}");
                        last = Some(p);
                    }
                    tiny_skia::PathSegment::Close => {}
                    other => panic!("not a line: {other:?}"),
                }
            }
        }
    }
}
