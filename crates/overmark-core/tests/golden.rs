//! Golden-image tests: known strokes rendered and compared with PNGs in `tests/golden/`.
//!
//! **To regenerate the PNGs** after a deliberate change to how strokes look, run
//! `UPDATE_GOLDEN=1 cargo test -p overmark-core --test golden` (PowerShell:
//! `$env:UPDATE_GOLDEN=1; cargo test -p overmark-core --test golden; Remove-Item Env:UPDATE_GOLDEN`),
//! look at every changed PNG, and commit them with the change.
//!
//! **Tolerance:** an image matches when no channel of any pixel differs by more than
//! [`MAX_CHANNEL_DIFF`], and at most [`MAX_DIFFERING_PIXELS`] parts in a thousand of its pixels
//! differ at all. That allows last-bit differences in the maths between platforms, never a
//! changed shape. On a mismatch the rendered image is written beside the target folder
//! (`target/golden-failures/`) to compare.

use std::path::{Path, PathBuf};

use overmark_core::colour::Colour;
use overmark_core::drawing::{Drawing, InkPoint};
use overmark_core::render::{Background, View, render};
use overmark_core::tool::ToolKind;
use tiny_skia::Pixmap;

const MAX_CHANNEL_DIFF: u8 = 24;
const MAX_DIFFERING_PIXELS: usize = 2; // per thousand

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn failures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-failures")
}

/// Compares `actual` with `tests/golden/<name>.png`, or writes it there with UPDATE_GOLDEN set.
fn check(name: &str, actual: &Pixmap) {
    let path = golden_dir().join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        actual.save_png(&path).unwrap();
        return;
    }
    let expected = Pixmap::load_png(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}. Generate it with UPDATE_GOLDEN=1 cargo test -p overmark-core --test golden",
            path.display()
        )
    });
    let fail = |why: String| {
        std::fs::create_dir_all(failures_dir()).unwrap();
        let out = failures_dir().join(format!("{name}.png"));
        actual.save_png(&out).unwrap();
        panic!("{name}: {why}; the render is at {}", out.display());
    };
    if (expected.width(), expected.height()) != (actual.width(), actual.height()) {
        fail(format!(
            "{}×{}, the golden image is {}×{}",
            actual.width(),
            actual.height(),
            expected.width(),
            expected.height()
        ));
    }
    let diffs: Vec<u8> = actual
        .data()
        .chunks(4)
        .zip(expected.data().chunks(4))
        .map(|(a, e)| {
            a.iter()
                .zip(e)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let worst = diffs.iter().copied().max().unwrap_or(0);
    let differing = diffs.iter().filter(|d| **d > 0).count();
    let allowed = diffs.len() * MAX_DIFFERING_PIXELS / 1000;
    if worst > MAX_CHANNEL_DIFF || differing > allowed {
        fail(format!(
            "{differing} pixels differ (at most {allowed} may), by up to {worst} (at most {MAX_CHANNEL_DIFF})"
        ));
    }
}

fn layer(width: u32, height: u32, drawing: &Drawing, view: View) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).unwrap();
    render(drawing, &mut pixmap.as_mut(), view, Background::Transparent);
    pixmap
}

/// Draws a stroke through `points` and finishes it.
fn stroke(drawing: &mut Drawing, colour: Colour, width: f32, points: &[InkPoint]) {
    let (id, _) = drawing.begin(ToolKind::Pen, colour, width, points[0]);
    for p in &points[1..] {
        drawing.push_point(id, *p);
    }
    drawing.finish(id);
}

const RED: Colour = Colour::rgb(0xE5, 0x39, 0x35);
const BLUE: Colour = Colour::rgb(0x1E, 0x88, 0xE5);

fn pen_wave() -> Vec<InkPoint> {
    (0..60)
        .map(|i| {
            let t = f64::from(i);
            InkPoint::pen(
                15.0 + t * 3.0,
                60.0 + (t / 7.0).sin() * 35.0,
                (0.15 + (t / 12.0).sin().abs() * 0.85) as f32,
                i as u64 * 8,
            )
        })
        .collect()
}

#[test]
fn pen_with_pressure() {
    let mut d = Drawing::new();
    stroke(&mut d, RED, 12.0, &pen_wave());
    check("pen_with_pressure", &layer(210, 120, &d, View::default()));
}

#[test]
fn mouse_zigzag_with_sharp_corners() {
    // A mouse moving faster on the long legs, so they're thinner.
    let mut points = Vec::new();
    let corners = [
        (15.0, 100.0),
        (60.0, 20.0),
        (105.0, 100.0),
        (150.0, 20.0),
        (195.0, 100.0),
    ];
    for (k, pair) in corners.windows(2).enumerate() {
        let steps = if k % 2 == 0 { 6 } else { 24 };
        for s in 0..steps {
            let t = f64::from(s) / f64::from(steps);
            let (a, b) = (pair[0], pair[1]);
            points.push(InkPoint::mouse(
                a.0 + (b.0 - a.0) * t,
                a.1 + (b.1 - a.1) * t,
                points.len() as u64 * 8,
            ));
        }
    }
    points.push(InkPoint::mouse(195.0, 100.0, 999));
    let mut d = Drawing::new();
    stroke(&mut d, BLUE, 10.0, &points);
    check("mouse_zigzag", &layer(210, 120, &d, View::default()));
}

#[test]
fn a_translucent_stroke_over_another() {
    let mut d = Drawing::new();
    stroke(&mut d, RED, 14.0, &pen_wave());
    let across: Vec<InkPoint> = (0..40)
        .map(|i| InkPoint::pen(10.0 + f64::from(i) * 5.0, 30.0 + f64::from(i) * 2.0, 0.7, 0))
        .collect();
    stroke(
        &mut d,
        Colour {
            a: 140,
            ..Colour::rgb(0x43, 0xA0, 0x47)
        },
        18.0,
        &across,
    );
    check("translucent_overlap", &layer(210, 120, &d, View::default()));
}

#[test]
fn at_twice_the_scale() {
    let mut d = Drawing::new();
    stroke(&mut d, RED, 12.0, &pen_wave());
    check("scaled_2x", &layer(420, 240, &d, View { scale: 2.0 }));
}

#[test]
fn over_a_screen_image() {
    // A capture: a stand-in screen (a gradient with a grid), the ink drawn over it.
    let (w, h) = (210, 120);
    let mut screen = Pixmap::new(w, h).unwrap();
    for (i, px) in screen.data_mut().chunks_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let grid = if x % 20 == 0 || y % 20 == 0 { 60 } else { 0 };
        px.copy_from_slice(&[
            (230 - x / 2 - grid) as u8,
            (220 - y - grid) as u8,
            (200 - grid) as u8,
            255,
        ]);
    }
    let mut d = Drawing::new();
    stroke(&mut d, RED, 12.0, &pen_wave());
    let mut capture = Pixmap::new(w, h).unwrap();
    render(
        &d,
        &mut capture.as_mut(),
        View::default(),
        Background::Image(screen.as_ref()),
    );
    check("over_screen", &capture);
}

#[test]
fn a_click_and_an_unfinished_stroke() {
    let mut d = Drawing::new();
    stroke(&mut d, BLUE, 16.0, &[InkPoint::mouse(40.0, 60.0, 0)]);
    // Still being drawn: it trails slightly behind its last point.
    let (id, _) = d.begin(ToolKind::Pen, RED, 8.0, InkPoint::mouse(80.0, 30.0, 0));
    for i in 1u32..25 {
        d.push_point(
            id,
            InkPoint::mouse(
                80.0 + f64::from(i) * 4.0,
                30.0 + f64::from(i) * 2.5,
                u64::from(i) * 8,
            ),
        );
    }
    check(
        "click_and_unfinished",
        &layer(210, 120, &d, View::default()),
    );
}

// ---- Every tool ----

use overmark_core::laser::Laser;
use overmark_core::render::{Highlight, LaserLook, RenderOptions, render_with};
use overmark_core::session::Session;

/// The test font: Tuffy (public domain), never shipped. The app uses the system font.
fn fonts() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let tuffy = include_bytes!("fonts/Tuffy.ttf").to_vec();
        overmark_core::text::set_fonts(vec![tuffy]).unwrap();
    });
}

const YELLOW: Colour = Colour::rgb(0xFD, 0xD8, 0x35);
const GREEN: Colour = Colour::rgb(0x43, 0xA0, 0x47);

fn pt(x: f64, y: f64, t: u64) -> InkPoint {
    InkPoint::mouse(x, y, t)
}

/// Draws with the session's current tool: down at the first point, through the rest, up at
/// the last.
fn gesture(s: &mut Session, points: &[(f64, f64)], shift: bool) {
    s.pointer_down(pt(points[0].0, points[0].1, 0), shift)
        .unwrap();
    for (x, y) in &points[1..] {
        s.pointer_move(pt(*x, *y, 0), shift);
    }
    let (x, y) = points[points.len() - 1];
    s.pointer_up(pt(x, y, 0), shift);
}

/// A stand-in screen: light, with dark "text" bars to highlight and fine detail to blur.
fn test_screen(w: u32, h: u32) -> Pixmap {
    let mut p = Pixmap::new(w, h).unwrap();
    for (i, px) in p.data_mut().chunks_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let text = y % 24 >= 8 && y % 24 < 16 && (x / 6) % 5 != 4;
        let detail = ((x / 3 + y / 3) % 2) as u8 * 18;
        let v = if text { 40 } else { 235 - detail };
        px.copy_from_slice(&[v, v, (v as u16 * 9 / 10) as u8, 255]);
    }
    p
}

fn highlighted_session() -> Session {
    let mut s = Session::new();
    s.tool = ToolKind::Highlighter;
    s.colour = YELLOW;
    s.width = 18.0;
    gesture(
        &mut s,
        &[(10.0, 12.0), (60.0, 12.5), (110.0, 12.0), (190.0, 13.0)],
        false,
    );
    s.width = 14.0;
    gesture(&mut s, &[(10.0, 60.0), (80.0, 58.0), (150.0, 62.0)], false);
    s
}

#[test]
fn highlighter_live_tint() {
    let s = highlighted_session();
    let mut out = Pixmap::new(210, 90).unwrap();
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &RenderOptions::default(),
        Background::Transparent,
    );
    check("highlighter_live_tint", &out);
}

#[test]
fn highlighter_multiply_over_text() {
    let s = highlighted_session();
    let screen = test_screen(210, 90);
    let mut out = Pixmap::new(210, 90).unwrap();
    let options = RenderOptions {
        highlight: Highlight::Multiply,
        ..RenderOptions::default()
    };
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &options,
        Background::Image(screen.as_ref()),
    );
    check("highlighter_multiply", &out);
}

#[test]
fn shapes_arrow_rectangle_ellipse_line() {
    let mut s = Session::new();
    s.colour = RED;
    s.width = 4.0;
    s.tool = ToolKind::Arrow;
    gesture(&mut s, &[(15.0, 100.0), (60.0, 70.0), (90.0, 20.0)], false);
    s.tool = ToolKind::Rectangle;
    // Shift: a square.
    gesture(&mut s, &[(110.0, 15.0), (150.0, 40.0)], true);
    s.colour = BLUE;
    s.tool = ToolKind::Ellipse;
    gesture(&mut s, &[(110.0, 70.0), (200.0, 110.0)], false);
    s.colour = GREEN;
    s.width = 6.0;
    s.tool = ToolKind::Line;
    // Shift: snapped to 45°.
    gesture(&mut s, &[(20.0, 20.0), (70.0, 58.0)], true);
    check("shapes", &layer(210, 120, s.drawing(), View::default()));
}

#[test]
fn text_on_two_lines() {
    fonts();
    let mut s = Session::new();
    s.colour = BLUE;
    s.width = 24.0;
    s.tool = ToolKind::Text;
    s.pointer_down(pt(10.0, 8.0, 0), false).unwrap();
    s.text_insert("Overmark");
    s.text_newline();
    s.text_insert("draws text 123");
    s.text_end(0);
    check("text", &layer(210, 80, s.drawing(), View::default()));
}

#[test]
fn numbered_stamps() {
    fonts();
    let mut s = Session::new();
    s.tool = ToolKind::Stamp;
    s.width = 28.0;
    for (x, colour) in [(25.0, RED), (70.0, YELLOW), (115.0, BLUE)] {
        s.colour = colour;
        gesture(&mut s, &[(x, 25.0)], false);
    }
    check("stamps", &layer(140, 50, s.drawing(), View::default()));
}

#[test]
fn blur_pixelates_a_frozen_frame() {
    let mut s = Session::new();
    s.backdrop = overmark_core::backdrop::Backdrop::Frozen;
    s.tool = ToolKind::Blur;
    s.width = 8.0;
    gesture(&mut s, &[(40.0, 10.0), (150.0, 60.0)], false);
    let screen = test_screen(210, 80);
    let mut out = Pixmap::new(210, 80).unwrap();
    render(
        s.drawing(),
        &mut out.as_mut(),
        View::default(),
        Background::Image(screen.as_ref()),
    );
    check("blur", &out);
}

#[test]
fn laser_with_its_trail() {
    let mut laser = Laser::new();
    for i in 0..30u32 {
        let t = f64::from(i);
        laser.move_to(
            kurbo::Point::new(20.0 + t * 5.5, 40.0 + (t / 4.0).sin() * 20.0),
            u64::from(i) * 16,
        );
    }
    let options = RenderOptions {
        now_ms: 29 * 16,
        laser: Some(LaserLook {
            laser: &laser,
            colour: RED,
            trail_ms: 400,
        }),
        ..RenderOptions::default()
    };
    let mut out = Pixmap::new(210, 80).unwrap();
    render_with(
        &Drawing::new(),
        &mut out.as_mut(),
        &options,
        Background::Transparent,
    );
    check("laser", &out);
}

#[test]
fn disappearing_ink_half_faded() {
    let mut d = Drawing::new();
    // Drawn at 0 and at 2000; at 3300 with a 3000 ms delay, the first is halfway faded out.
    let mut line = |y: f64, t: u64, colour: Colour| {
        let points: Vec<InkPoint> = (0..30)
            .map(|i| InkPoint::mouse(15.0 + f64::from(i) * 6.0, y, t))
            .collect();
        stroke(&mut d, colour, 10.0, &points);
    };
    line(25.0, 0, RED);
    line(55.0, 2000, BLUE);
    let options = RenderOptions {
        now_ms: 3300,
        disappearing_ink_ms: Some(3000),
        ..RenderOptions::default()
    };
    let mut out = Pixmap::new(210, 80).unwrap();
    render_with(&d, &mut out.as_mut(), &options, Background::Transparent);
    check("disappearing_ink", &out);
}

// ---- Backdrops and guides ----

use overmark_core::backdrop::{
    Backdrop, BackdropFill, GuideStyle, Guides, render_guides, render_solid,
};
use overmark_core::settings::{BackdropSettings, GuideSettings};

/// A backdrop with its guides and a stroke on it, at `scale`, as the layers stack: the solid
/// colour, the guides, then the ink.
fn backdrop_image(backdrop: Backdrop, guides: Guides, scale: f32) -> Pixmap {
    let settings = BackdropSettings::default();
    let (w, h) = ((120.0 * scale) as u32, (80.0 * scale) as u32);
    let mut out = Pixmap::new(w, h).unwrap();
    let BackdropFill::Solid(colour) = backdrop.fill(&settings) else {
        panic!("a solid backdrop");
    };
    render_solid(&mut out.as_mut(), colour, None);
    let style = GuideStyle::new(
        &GuideSettings {
            spacing: 16,
            ..GuideSettings::default()
        },
        &settings,
        backdrop,
    );
    let view = View { scale };
    render_guides(&mut out.as_mut(), guides, &style, view, None);
    let mut d = Drawing::new();
    let ink = if backdrop == Backdrop::Blackboard {
        YELLOW
    } else {
        RED
    };
    let points: Vec<InkPoint> = (0..20)
        .map(|i| {
            InkPoint::mouse(
                15.0 + f64::from(i) * 5.0,
                40.0 + (f64::from(i) / 3.0).sin() * 20.0,
                0,
            )
        })
        .collect();
    stroke(&mut d, ink, 6.0, &points);
    let snapshot = out.clone();
    render(
        &d,
        &mut out.as_mut(),
        view,
        Background::Image(snapshot.as_ref()),
    );
    out
}

#[test]
fn solid_backdrops_at_100_and_150() {
    for backdrop in [Backdrop::Whiteboard, Backdrop::Blackboard] {
        for (scale, tag) in [(1.0, "100"), (1.5, "150")] {
            let name = format!("backdrop_{}_{tag}", format!("{backdrop:?}").to_lowercase());
            check(&name, &backdrop_image(backdrop, Guides::Off, scale));
        }
    }
}

#[test]
fn every_guide_at_100_and_150() {
    for guides in [Guides::Grid, Guides::Dots, Guides::Lines] {
        for (scale, tag) in [(1.0, "100"), (1.5, "150")] {
            let g = format!("{guides:?}").to_lowercase();
            check(
                &format!("guides_{g}_whiteboard_{tag}"),
                &backdrop_image(Backdrop::Whiteboard, guides, scale),
            );
            check(
                &format!("guides_{g}_blackboard_{tag}"),
                &backdrop_image(Backdrop::Blackboard, guides, scale),
            );
        }
    }
}

#[test]
fn guides_over_the_live_screen() {
    // On the live layer the guides are their own translucent layer, over whatever is on screen.
    let settings = BackdropSettings::default();
    let style = GuideStyle::new(
        &GuideSettings {
            spacing: 16,
            ..GuideSettings::default()
        },
        &settings,
        Backdrop::Live,
    );
    let mut layer = Pixmap::new(120, 80).unwrap();
    render_guides(
        &mut layer.as_mut(),
        Guides::Grid,
        &style,
        View::default(),
        None,
    );
    check("guides_grid_live_layer", &layer);
}
