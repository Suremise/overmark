//! How long a 4K layer takes to follow the pen: `cargo bench -p overmark-core --bench redraw`.
//!
//! A long stroke is drawn sample by sample on a 3840×2160 layer, and each sample is timed
//! twice: updating the drawing (the outline, and the area that changed) and redrawing that
//! area. Their sum must fit well inside a 144 Hz frame (6.9 ms). A full redraw of the layer is
//! timed too, for comparison. The results go in docs/ARCHITECTURE.md (Strokes and rendering).
//!
//! It's a plain timing program (no harness crate), built optimised by `cargo bench`.

use std::time::{Duration, Instant};

use overmark_core::colour::Colour;
use overmark_core::drawing::{Drawing, InkPoint};
use overmark_core::render::{Background, View, dirty_rect, render, render_region};
use overmark_core::tool::ToolKind;
use tiny_skia::Pixmap;

const W: u32 = 3840;
const H: u32 = 2160;

/// A pen moving across the screen in loops, pressure varying: about 6,000 px of ink.
fn sample(i: u32) -> InkPoint {
    let t = f64::from(i);
    InkPoint::pen(
        200.0 + t * 1.6 + (t / 25.0).cos() * 120.0,
        1080.0 + (t / 25.0).sin() * 600.0 + (t / 300.0).sin() * 300.0,
        (0.3 + (t / 40.0).sin().abs() * 0.7) as f32,
        u64::from(i) * 4,
    )
}

fn stats(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let pick = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    println!(
        "{name:<34} median {:>7.3} ms   p95 {:>7.3} ms   max {:>7.3} ms   ({} samples)",
        ms(pick(0.5)),
        ms(pick(0.95)),
        ms(*times.last().unwrap()),
        times.len()
    );
}

fn run(label: &str, background: Background, samples: u32) {
    let view = View::default();
    let mut layer = Pixmap::new(W, H).unwrap();
    let mut drawing = Drawing::new();
    // Earlier strokes on the layer, as there would be.
    for k in 0..5u32 {
        let (id, _) = drawing.begin(
            ToolKind::Pen,
            Colour::rgb(0x1E, 0x88, 0xE5),
            8.0,
            InkPoint::mouse(300.0, 200.0 + f64::from(k) * 300.0, 0),
        );
        for i in 1..400u32 {
            let x = 300.0 + f64::from(i) * 8.0;
            let y = 200.0 + f64::from(k) * 300.0 + (f64::from(i) / 10.0).sin() * 40.0;
            drawing.push_point(id, InkPoint::mouse(x, y, u64::from(i) * 8));
        }
        drawing.finish(id);
    }
    render(&drawing, &mut layer.as_mut(), view, background);

    let (id, _) = drawing.begin(
        ToolKind::Pen,
        Colour::rgb(0xE5, 0x39, 0x35),
        12.0,
        sample(0),
    );
    let (mut update, mut redraw, mut total) = (Vec::new(), Vec::new(), Vec::new());
    for i in 1..samples {
        let start = Instant::now();
        let area = drawing.push_point(id, sample(i));
        let updated = Instant::now();
        if let Some(rect) = area.and_then(|a| dirty_rect(a, view, W, H)) {
            render_region(&drawing, &mut layer.as_mut(), view, background, rect);
        }
        let done = Instant::now();
        update.push(updated - start);
        redraw.push(done - updated);
        total.push(done - start);
    }
    println!("{label}: a {samples}-sample stroke on a {W}×{H} layer");
    stats("  update the drawing", update);
    stats("  redraw the changed area", redraw);
    stats("  both, per sample", total);

    let mut full = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        render(&drawing, &mut layer.as_mut(), view, background);
        full.push(start.elapsed());
    }
    stats("  full redraw of the layer", full);
}

fn main() {
    // `cargo test --benches` runs this too; keep that quick.
    let samples = if std::env::args().any(|a| a == "--bench") {
        3000
    } else {
        200
    };
    run("Live layer (transparent)", Background::Transparent, samples);
    let mut screen = Pixmap::new(W, H).unwrap();
    screen.fill(tiny_skia::Color::from_rgba8(240, 240, 240, 255));
    run(
        "Capture (over a screen image)",
        Background::Image(screen.as_ref()),
        samples,
    );
}
