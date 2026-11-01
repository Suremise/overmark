use super::fixtures::{CASES, POINTS_CASES};
use super::*;

// The easings the fixtures name (generate-fixtures.mjs has the same).
pub(super) fn defaults() -> StrokeOptions {
    StrokeOptions::default()
}
pub(super) fn linear(t: f64) -> f64 {
    t
}
pub(super) fn quad_in(t: f64) -> f64 {
    t * t
}
pub(super) fn sine_out(t: f64) -> f64 {
    (t * std::f64::consts::PI / 2.0).sin()
}

/// How far the port may differ from the library: last-bit differences in `sin`, `cos` and
/// `hypot` between JavaScript engines and Rust's maths library.
const TOLERANCE: f64 = 1e-9;

fn input(points: &[(f64, f64, Option<f64>)]) -> Vec<InputPoint> {
    points
        .iter()
        .map(|&(x, y, pressure)| InputPoint { x, y, pressure })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= TOLERANCE * (1.0 + b.abs())
}

#[test]
fn there_are_fixtures() {
    assert!(
        CASES.len() > 50 && POINTS_CASES.len() >= 5,
        "fixtures.rs is empty: generate it with generate-fixtures.mjs"
    );
}

#[test]
fn outlines_match_perfect_freehand() {
    let mut failures = Vec::new();
    for case in CASES {
        let outline = stroke_outline(&input(case.input), &(case.options)());
        let expected: Vec<Point> = case
            .outline
            .chunks(2)
            .map(|xy| Point::new(xy[0], xy[1]))
            .collect();
        if outline.len() != expected.len() {
            failures.push(format!(
                "{}: {} points, the library has {}",
                case.name,
                outline.len(),
                expected.len()
            ));
            continue;
        }
        if let Some((i, (got, want))) = outline
            .iter()
            .zip(&expected)
            .enumerate()
            .find(|(_, (g, w))| !close(g.x, w.x) || !close(g.y, w.y))
        {
            failures.push(format!(
                "{}: point {i} is {got:?}, the library has {want:?}",
                case.name
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn stroke_points_match_perfect_freehand() {
    for case in POINTS_CASES {
        let points = stroke_points(&input(case.input), &(case.options)());
        let expected: Vec<&[f64]> = case.points.chunks(7).collect();
        assert_eq!(points.len(), expected.len(), "{}", case.name);
        for (i, (p, e)) in points.iter().zip(&expected).enumerate() {
            let got = [
                p.point.x,
                p.point.y,
                p.pressure,
                p.distance,
                p.vector.x,
                p.vector.y,
                p.running_length,
            ];
            for (field, (g, w)) in got.iter().zip(e.iter()).enumerate() {
                assert!(
                    close(*g, *w),
                    "{}: point {i}, field {field}: {g} vs {w}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn nothing_in_nothing_out() {
    assert!(stroke_outline(&[], &defaults()).is_empty());
    let line = [InputPoint::new(0.0, 0.0), InputPoint::new(50.0, 0.0)];
    let zero = StrokeOptions {
        size: 0.0,
        ..defaults()
    };
    assert!(stroke_outline(&line, &zero).is_empty());
}

#[test]
fn a_single_point_is_a_blob_around_it() {
    // A second point is added 1 px away, so a click draws a short capped stroke: a small round
    // blob, no wider than the stroke, around the point.
    let center = Point::new(50.0, 60.0);
    let outline = stroke_outline(&[InputPoint::new(center.x, center.y)], &defaults());
    assert!(outline.len() > 20, "{}", outline.len());
    for p in &outline {
        let r = p.distance(center);
        assert!(r > 0.0 && r <= defaults().size / 2.0 + 1.5, "{r}");
    }
}

#[test]
fn a_lone_measured_point_is_a_dot() {
    // The outline of a single measured point (stroke_points never gives one, but a caller
    // may) is the library's dot: 12 or 13 points round it, as its float loop counts.
    let point = StrokePoint {
        point: Point::new(10.0, 10.0),
        pressure: 0.5,
        distance: 0.0,
        vector: Vec2::new(1.0, 1.0),
        running_length: 0.0,
    };
    let dot = stroke_outline_points(&[point], &defaults());
    assert!((12..=13).contains(&dot.len()), "{}", dot.len());
    let radii: Vec<f64> = dot.iter().map(|p| p.distance(point.point)).collect();
    for r in &radii {
        assert!((r - radii[0]).abs() < 1e-9 && *r > 0.0, "{radii:?}");
    }
}

/// Every outline point lies within the stroke's radius of the line it follows.
#[test]
fn a_straight_stroke_stays_within_its_width() {
    let line: Vec<InputPoint> = (0..30)
        .map(|i| InputPoint::new(f64::from(i) * 10.0, 100.0))
        .collect();
    let options = StrokeOptions {
        thinning: 0.0,
        ..defaults()
    };
    let outline = stroke_outline(&line, &options);
    assert!(outline.len() > 20);
    for p in &outline {
        assert!((p.y - 100.0).abs() <= options.size / 2.0 + 1e-9, "{p:?}");
        assert!(
            p.x >= -options.size / 2.0 - 1e-9 && p.x <= 290.0 + options.size,
            "{p:?}"
        );
    }
}

#[test]
fn moving_the_input_moves_the_outline() {
    let wave: Vec<InputPoint> = (0..40)
        .map(|i| {
            let i = f64::from(i);
            InputPoint::with_pressure(i * 6.0, (i / 4.0).sin() * 40.0, 0.5)
        })
        .collect();
    let moved: Vec<InputPoint> = wave
        .iter()
        .map(|p| InputPoint {
            x: p.x + 1000.0,
            y: p.y - 500.0,
            ..*p
        })
        .collect();
    let a = stroke_outline(&wave, &defaults());
    let b = stroke_outline(&moved, &defaults());
    assert_eq!(a.len(), b.len());
    for (p, q) in a.iter().zip(&b) {
        assert!((q.x - p.x - 1000.0).abs() < 1e-6 && (q.y - p.y + 500.0).abs() < 1e-6);
    }
}

#[test]
fn pressure_widens_the_stroke() {
    let line = |pressure: f64| -> Vec<InputPoint> {
        (0..30)
            .map(|i| InputPoint::with_pressure(f64::from(i) * 10.0, 0.0, pressure))
            .collect()
    };
    let options = StrokeOptions {
        simulate_pressure: false,
        last: true,
        ..defaults()
    };
    let width = |outline: Vec<Point>| outline.iter().map(|p| p.y.abs()).fold(0.0, f64::max);
    let light = width(stroke_outline(&line(0.1), &options));
    let heavy = width(stroke_outline(&line(1.0), &options));
    assert!(heavy > light * 2.0, "{light} vs {heavy}");
}

#[test]
fn the_fixture_easings_are_the_generators() {
    assert_eq!(linear(0.3), 0.3);
    assert_eq!(quad_in(0.5), 0.25);
    assert!((sine_out(1.0) - 1.0).abs() < 1e-12);
}
