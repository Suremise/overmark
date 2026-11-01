//! Hit-testing for the eraser: whether a point or a short drag touches a shape.
//!
//! A drag is the segment from where the pointer was to where it is, so a quick swipe can't
//! jump over a thin stroke between two samples.

use kurbo::Point;

/// Whether the segment `from`–`to` passes over the filled polygon `outline` (non-zero winding)
/// or within `tolerance` of its edge.
pub fn touches_polygon(outline: &[Point], from: Point, to: Point, tolerance: f64) -> bool {
    match outline.len() {
        0 => false,
        1 => segment_distance(from, to, outline[0], outline[0]) <= tolerance,
        _ => {
            winding(outline, from) != 0
                || winding(outline, to) != 0
                || edges(outline).any(|(a, b)| segment_distance(from, to, a, b) <= tolerance)
        }
    }
}

/// Whether the segment `from`–`to` passes over the filled polygons (non-zero winding, all
/// together) or within `tolerance` of any of their edges.
pub fn touches_polygons(polygons: &[Vec<Point>], from: Point, to: Point, tolerance: f64) -> bool {
    let winding_all = |p: Point| polygons.iter().map(|c| winding(c, p)).sum::<i32>();
    if winding_all(from) != 0 || winding_all(to) != 0 {
        return true;
    }
    polygons.iter().any(|c| match c.len() {
        0 => false,
        1 => segment_distance(from, to, c[0], c[0]) <= tolerance,
        _ => edges(c).any(|(a, b)| segment_distance(from, to, a, b) <= tolerance),
    })
}

/// Whether the segment `from`–`to` comes within `reach` of the polyline `points`.
pub fn touches_polyline(points: &[Point], from: Point, to: Point, reach: f64) -> bool {
    match points.len() {
        0 => false,
        1 => segment_distance(from, to, points[0], points[0]) <= reach,
        _ => points
            .windows(2)
            .any(|w| segment_distance(from, to, w[0], w[1]) <= reach),
    }
}

/// A closed polygon's edges, the last one back to the start.
fn edges(polygon: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    let n = polygon.len();
    (0..n).map(move |i| (polygon[i], polygon[(i + 1) % n]))
}

/// The polygon's winding number around `p`: 0 outside, otherwise inside.
pub fn winding(polygon: &[Point], p: Point) -> i32 {
    let mut w = 0;
    for (a, b) in edges(polygon) {
        let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
        if a.y <= p.y {
            if b.y > p.y && cross > 0.0 {
                w += 1;
            }
        } else if b.y <= p.y && cross < 0.0 {
            w -= 1;
        }
    }
    w
}

/// The shortest distance between segments `a`–`b` and `c`–`d` (0 if they cross).
pub fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if segments_cross(a, b, c, d) {
        return 0.0;
    }
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let side = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn square() -> Vec<Point> {
        vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)]
    }

    #[test]
    fn inside_on_and_near_a_polygon() {
        let sq = square();
        assert!(touches_polygon(&sq, p(5.0, 5.0), p(5.0, 5.0), 0.0));
        assert!(touches_polygon(&sq, p(12.0, 5.0), p(12.0, 5.0), 2.0));
        assert!(!touches_polygon(&sq, p(12.5, 5.0), p(12.5, 5.0), 2.0));
        // Reversed winding is still inside.
        let rev: Vec<Point> = sq.iter().rev().copied().collect();
        assert!(touches_polygon(&rev, p(5.0, 5.0), p(5.0, 5.0), 0.0));
    }

    #[test]
    fn a_drag_across_a_thin_shape_touches_it() {
        // A 1 px tall sliver, crossed by a drag whose ends are both well away from it.
        let sliver = vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 1.0), p(0.0, 1.0)];
        assert!(touches_polygon(&sliver, p(50.0, -30.0), p(50.0, 30.0), 0.0));
        assert!(!touches_polygon(
            &sliver,
            p(150.0, -30.0),
            p(150.0, 30.0),
            0.0
        ));
    }

    #[test]
    fn polylines_use_their_reach() {
        let line = vec![p(0.0, 0.0), p(100.0, 0.0)];
        assert!(touches_polyline(&line, p(50.0, 4.0), p(50.0, 4.0), 5.0));
        assert!(!touches_polyline(&line, p(50.0, 6.0), p(50.0, 6.0), 5.0));
        assert!(touches_polyline(
            &[p(3.0, 4.0)],
            p(0.0, 0.0),
            p(0.0, 0.0),
            5.0
        ));
        assert!(!touches_polyline(&[], p(0.0, 0.0), p(0.0, 0.0), 5.0));
    }

    #[test]
    fn segment_distances() {
        assert_eq!(
            segment_distance(p(0.0, 0.0), p(10.0, 10.0), p(0.0, 10.0), p(10.0, 0.0)),
            0.0
        );
        assert_eq!(
            segment_distance(p(0.0, 0.0), p(10.0, 0.0), p(0.0, 3.0), p(10.0, 3.0)),
            3.0
        );
        assert_eq!(
            segment_distance(p(0.0, 0.0), p(0.0, 0.0), p(3.0, 4.0), p(3.0, 4.0)),
            5.0
        );
    }
}
