//! The README's clip of Live and Frozen, rendered off screen with Overmark's own code: the
//! real toolbar (egui, then the CPU rasterizer, as `toolbar_shot`), strokes drawn point by
//! point by core's renderer, the veil's colours and the capture notice's look, over a desktop
//! made here in code (a generated wallpaper and two windows of invented content). Nothing is
//! read from the screen, so nothing private can be in it.
//!
//! ```text
//! cargo run -p overmark --release --example demo_clip -- [<folder>]
//! ```
//!
//! Writes the frames to `<folder>\frames` (default `%TEMP%\overmark-demo-clip`), then, with
//! ffmpeg on the PATH, `assets/overmark-demo.gif` (the README's, 960 px wide), and beside the
//! frames `overmark-demo.mp4` (H.264, 1920 px, no audio) and `overmark-demo-poster.png` (a
//! still from the Frozen part), for the website.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kurbo::{BezPath, PathEl};
use overmark::toolbar::{State, render_offscreen};
use overmark_core::backdrop::{Backdrop, Guides};
use overmark_core::colour::Colour;
use overmark_core::drawing::InkPoint;
use overmark_core::render::{Background, Highlight, RenderOptions, render_with};
use overmark_core::session::Session;
use overmark_core::settings::{DEFAULT_DIM, DEFAULT_PALETTE, Settings};
use overmark_core::text;
use overmark_core::tool::ToolKind;
use overmark_core::tr;
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};

const W: u32 = 1920;
const H: u32 = 1080;
const FPS: u64 = 25;
/// The display scale the clip shows: a 1080p screen at 125 %.
const SCALE: f32 = 1.25;

// The story, in milliseconds.
const OPEN: u64 = 600;
const CIRCLE: (u64, u64) = (1400, 3000);
const FREEZE: u64 = 3800;
const HIGHLIGHT: (u64, u64) = (4500, 5500);
const ARROW: (u64, u64) = (6100, 6700);
const STAMPS: [u64; 2] = [7300, 7800];
const CAPTURE: u64 = 9000;
const CLOSE: u64 = 13000;
const END: u64 = 14500;
/// The still for the website: everything drawn, Frozen.
const POSTER: u64 = 8600;

/// Time the screen is moving over the whole loop: the chart's data goes once round in it, so
/// the loop's end meets its start.
const LIVE_TOTAL: u64 = FREEZE + (END - CLOSE);

const RED: Colour = DEFAULT_PALETTE[0];
const AMBER: Colour = DEFAULT_PALETTE[1];
const YELLOW: Colour = DEFAULT_PALETTE[2];
const BLUE: Colour = DEFAULT_PALETTE[4];

/// The veils, as the overlay shows them (`overmark-win`'s `veil_look`).
const FROZEN_TINT: (u8, u8, u8) = (12, 28, 64);

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("overmark-demo-clip"));
    let frames = out.join("frames");
    let _ = std::fs::remove_dir_all(&frames);
    std::fs::create_dir_all(&frames).unwrap();
    overmark_core::i18n::init("en");
    assert!(
        overmark_win::fonts::ensure_loaded(),
        "Windows' text fonts are needed"
    );

    let desktop = Desktop::new();
    let mut toolbars: HashMap<(Backdrop, ToolKind), Vec<u8>> = HashMap::new();
    let toast = Toast::new();
    let settings = Settings::default();
    let script = Script::new(&desktop);

    let count = END * FPS / 1000;
    for i in 0..count {
        let t = i * 1000 / FPS;
        let frame = render_frame(t, &desktop, &script, &settings, &toast, &mut toolbars);
        save_png(&frame, &frames.join(format!("{i:04}.png")));
        if i == POSTER * FPS / 1000 {
            save_png(&frame, &out.join("overmark-demo-poster.png"));
        }
    }
    println!("{count} frames in {}", frames.display());
    encode(&out, &frames);
}

/// One frame at `t`: the desktop, then (while the layer is open) the veil, the ink, the
/// toolbar and the notice.
fn render_frame(
    t: u64,
    desktop: &Desktop,
    script: &Script,
    settings: &Settings,
    toast: &Toast,
    toolbars: &mut HashMap<(Backdrop, ToolKind), Vec<u8>>,
) -> Pixmap {
    let open = (OPEN..CLOSE).contains(&t);
    let frozen = open && t >= FREEZE;
    let mut base = desktop.at(live_ms(t));
    if !open {
        return base;
    }
    let (veil, alpha) = if frozen {
        (FROZEN_TINT, DEFAULT_DIM.max(0.12))
    } else {
        ((0, 0, 0), DEFAULT_DIM)
    };
    fill_all(
        &mut base,
        Color::from_rgba8(veil.0, veil.1, veil.2, a8(alpha)),
    );

    let session = script.session_at(t);
    let mut frame = Pixmap::new(W, H).unwrap();
    let options = RenderOptions {
        highlight: Highlight::Multiply,
        now_ms: t,
        ..Default::default()
    };
    render_with(
        session.drawing(),
        &mut frame.as_mut(),
        &options,
        Background::Image(base.as_ref()),
    );

    let backdrop = if frozen {
        Backdrop::Frozen
    } else {
        Backdrop::Live
    };
    let tool = session.tool;
    let (tw, th) = toolbar_size();
    let bar = toolbars.entry((backdrop, tool)).or_insert_with(|| {
        let state = State {
            backdrop,
            guides: Guides::Off,
            tool,
            colour: session.colour,
            palette: DEFAULT_PALETTE.to_vec(),
            width: settings.drawing.widths.get(tool),
            can_undo: !session.drawing().is_empty(),
            can_redo: false,
            collapsed: false,
            disappearing_ink: false,
            right_click_undoes: true,
        };
        render_offscreen(&state, tw, th, SCALE, 3, None)
    });
    over(
        &mut frame,
        bar,
        tw,
        th,
        ((W as usize - tw) / 2, (4.0 * SCALE) as usize),
    );

    if t >= CAPTURE {
        let (x, y) = toast.at;
        over(&mut frame, &toast.rgba, toast.w, toast.h, (x, y));
    }
    frame
}

/// The toolbar window's size here: its width, and room for the bar without its strips.
fn toolbar_size() -> (usize, usize) {
    ((1040.0 * SCALE) as usize, (130.0 * SCALE) as usize)
}

/// How long the screen has been moving by `t` (it stands still while Frozen).
fn live_ms(t: u64) -> u64 {
    if t < FREEZE {
        t
    } else if t < CLOSE {
        FREEZE
    } else {
        FREEZE + (t - CLOSE)
    }
}

// ---- The drawing ----

/// A stroke drawn by hand: its tool, colour and width, and its points, timed.
struct Gesture {
    tool: ToolKind,
    colour: Colour,
    width: f32,
    points: Vec<InkPoint>,
    stamp: bool,
}

/// What's drawn, and when.
struct Script {
    gestures: Vec<Gesture>,
}

impl Script {
    fn new(desktop: &Desktop) -> Script {
        let widths = Settings::default().drawing.widths;
        let w = |tool| widths.get(tool).unwrap_or(4.0);
        let mut gestures = Vec::new();

        // Live: a loop round the chart's newest points, as they move.
        let (cx, cy) = desktop.chart_focus();
        let circle = timed(CIRCLE, |s| {
            let a = -2.2 + s * std::f64::consts::TAU * 1.12;
            let r = 1.0 + 0.06 * (s * 9.0).sin();
            (cx + 118.0 * r * a.cos(), cy + 84.0 * r * a.sin())
        });
        gestures.push(Gesture {
            tool: ToolKind::Pen,
            colour: RED,
            width: w(ToolKind::Pen) * SCALE,
            points: circle,
            stamp: false,
        });

        // Frozen: the slide's second line highlighted…
        let (hx0, hx1, hy) = desktop.slide_line(1);
        gestures.push(Gesture {
            tool: ToolKind::Highlighter,
            colour: YELLOW,
            width: w(ToolKind::Highlighter) * SCALE,
            points: timed(HIGHLIGHT, |s| {
                (
                    hx0 - 8.0 + (hx1 - hx0 + 16.0) * s,
                    hy + 1.5 * (s * 5.0).sin(),
                )
            }),
            stamp: false,
        });
        // …an arrow from it to the chart's peak…
        let (px, py) = desktop.frozen_peak();
        let from = (hx0 - 30.0, hy + 10.0);
        let to = (px + 22.0, py + 14.0);
        gestures.push(Gesture {
            tool: ToolKind::Arrow,
            colour: AMBER,
            width: w(ToolKind::Arrow) * SCALE,
            points: timed(ARROW, |s| {
                (from.0 + (to.0 - from.0) * s, from.1 + (to.1 - from.1) * s)
            }),
            stamp: false,
        });
        // …and two numbered stamps.
        let (_, sx, sy) = desktop.slide_line(2);
        for (at, (x, y)) in STAMPS
            .into_iter()
            .zip([(px - 34.0, py - 30.0), (sx + 36.0, sy)])
        {
            gestures.push(Gesture {
                tool: ToolKind::Stamp,
                colour: BLUE,
                width: w(ToolKind::Stamp) * SCALE,
                points: vec![InkPoint::pen(x, y, 0.5, at)],
                stamp: true,
            });
        }
        Script { gestures }
    }

    /// The session as it is at `t`: every point drawn by then, the tool last picked.
    fn session_at(&self, t: u64) -> Session {
        let mut s = Session::new();
        s.tool = ToolKind::Pen;
        s.colour = RED;
        s.width = self.gestures[0].width;
        for g in &self.gestures {
            let first = g.points[0].time_ms;
            // The tool is picked a moment before it's used.
            if t + 300 < first {
                break;
            }
            s.tool = g.tool;
            s.colour = g.colour;
            s.width = g.width;
            if t < first {
                break;
            }
            if g.stamp {
                s.place_stamp(g.points[0]);
                continue;
            }
            s.stroke_begin(g.points[0]);
            let last = g.points[g.points.len() - 1].time_ms;
            for p in g.points[1..].iter().take_while(|p| p.time_ms <= t) {
                s.stroke_extend(*p);
            }
            if t >= last {
                s.stroke_end();
            }
        }
        s
    }
}

/// Points along `path` (0 to 1) over `(from, to)` ms, every 8 ms, eased as a hand moves, with
/// a pen's pressure rising and falling.
fn timed(span: (u64, u64), path: impl Fn(f64) -> (f64, f64)) -> Vec<InkPoint> {
    let (from, to) = span;
    let steps = (to - from) / 8;
    (0..=steps)
        .map(|i| {
            let u = i as f64 / steps as f64;
            let s = u * u * (3.0 - 2.0 * u);
            let (x, y) = path(s);
            let pressure = (0.35 + 0.4 * (u * std::f64::consts::PI).sin()) as f32;
            InkPoint::pen(x, y, pressure, from + i * 8)
        })
        .collect()
}

// ---- The desktop ----

/// The desktop under the layer: a wallpaper and two windows, drawn once; the chart's line is
/// drawn on each frame.
struct Desktop {
    still: Pixmap,
    chart: Rect,
    slide_lines: Vec<(f64, f64, f64)>,
}

const CHART_WINDOW: (f32, f32, f32, f32) = (110.0, 250.0, 960.0, 620.0);
const SLIDE_WINDOW: (f32, f32, f32, f32) = (1150.0, 330.0, 660.0, 470.0);
const INK: (u8, u8, u8) = (0x22, 0x27, 0x30);
const MUTED: (u8, u8, u8) = (0x6B, 0x72, 0x80);
const POINTS: usize = 48;

impl Desktop {
    fn new() -> Desktop {
        let mut p = Pixmap::new(W, H).unwrap();
        wallpaper(&mut p);

        let (x, y, w, h) = CHART_WINDOW;
        window(&mut p, x, y, w, h, "Sign-ups this week");
        label(&mut p, "1,284", x + 40.0, y + 78.0, 46.0, rgb(INK));
        label(
            &mut p,
            "new this week, up 12 %",
            x + 200.0,
            y + 100.0,
            22.0,
            rgb(MUTED),
        );
        let chart = Rect::from_xywh(x + 60.0, y + 180.0, w - 110.0, h - 260.0).unwrap();
        let mut grid = Paint::default();
        grid.set_color_rgba8(0xE5, 0xE7, 0xEB, 255);
        for i in 0..=4 {
            let gy = chart.top() + chart.height() * i as f32 / 4.0;
            fill_rect(&mut p, chart.left(), gy, chart.width(), 1.5, &grid);
        }
        for (i, day) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .enumerate()
        {
            let dx = chart.left() + chart.width() * i as f32 / 6.0 - 16.0;
            label(&mut p, day, dx, chart.bottom() + 18.0, 19.0, rgb(MUTED));
        }

        let (x, y, w, _) = SLIDE_WINDOW;
        window(&mut p, x, y, w, SLIDE_WINDOW.3, "Launch plan");
        label(&mut p, "Launch plan", x + 44.0, y + 84.0, 40.0, rgb(INK));
        let mut accent = Paint::default();
        accent.set_color_rgba8(0x1E, 0x88, 0xE5, 255);
        fill_rect(&mut p, x + 46.0, y + 146.0, 72.0, 5.0, &accent);
        let mut slide_lines = Vec::new();
        for (i, line) in [
            "Beta to testers on Monday",
            "Fix the top three issues",
            "Release notes and a demo",
        ]
        .iter()
        .enumerate()
        {
            let ly = y + 190.0 + i as f32 * 66.0;
            fill_circle(&mut p, x + 56.0, ly + 16.0, 6.0, &accent);
            let size = 28.0;
            label(&mut p, line, x + 80.0, ly, size, rgb(INK));
            let width = text::shape(line, size).width;
            let x0 = f64::from(x + 80.0);
            slide_lines.push((x0, x0 + width, f64::from(ly + size * 0.62)));
        }
        let _ = w;
        Desktop {
            still: p,
            chart,
            slide_lines,
        }
    }

    /// The line `i` of the slide: its text's left and right, and its middle's height.
    fn slide_line(&self, i: usize) -> (f64, f64, f64) {
        self.slide_lines[i]
    }

    /// The chart's values at a moment of live time, 0 to 1.
    fn values(&self, live_ms: u64) -> Vec<f32> {
        let phase = live_ms as f32 / LIVE_TOTAL as f32 * std::f32::consts::TAU;
        (0..POINTS)
            .map(|i| {
                let x = i as f32;
                let v = 0.30
                    + 0.009 * x
                    + 0.10 * (x * 0.31 - phase).sin()
                    + 0.06 * (x * 0.73 + 2.0 * phase).sin()
                    + 0.03 * (x * 1.9 - 3.0 * phase).sin();
                v.clamp(0.04, 0.96)
            })
            .collect()
    }

    fn point(&self, i: usize, v: f32) -> (f32, f32) {
        let c = self.chart;
        (
            c.left() + c.width() * i as f32 / (POINTS - 1) as f32,
            c.bottom() - c.height() * v,
        )
    }

    /// Where the pen's loop goes: round the newest points.
    fn chart_focus(&self) -> (f64, f64) {
        let (x, y) = self.point(POINTS - 5, 0.62);
        (f64::from(x), f64::from(y))
    }

    /// The highest point of the chart as it's frozen.
    fn frozen_peak(&self) -> (f64, f64) {
        let v = self.values(FREEZE);
        let (i, value) = v
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        let (x, y) = self.point(i, *value);
        (f64::from(x), f64::from(y))
    }

    /// The desktop at a moment of live time.
    fn at(&self, live_ms: u64) -> Pixmap {
        let mut p = self.still.clone();
        let values = self.values(live_ms);
        let mut line = PathBuilder::new();
        let mut area = PathBuilder::new();
        let c = self.chart;
        area.move_to(c.left(), c.bottom());
        for (i, v) in values.iter().enumerate() {
            let (x, y) = self.point(i, *v);
            if i == 0 {
                line.move_to(x, y);
            } else {
                line.line_to(x, y);
            }
            area.line_to(x, y);
        }
        area.line_to(c.right(), c.bottom());
        area.close();
        let shade = LinearGradient::new(
            Point::from_xy(0.0, c.top()),
            Point::from_xy(0.0, c.bottom()),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(0x1E, 0x88, 0xE5, 90)),
                GradientStop::new(1.0, Color::from_rgba8(0x1E, 0x88, 0xE5, 8)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        let mut paint = Paint {
            shader: shade,
            anti_alias: true,
            ..Default::default()
        };
        p.fill_path(
            &area.finish().unwrap(),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        paint.set_color_rgba8(0x1E, 0x88, 0xE5, 255);
        let stroke = Stroke {
            width: 4.0,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        };
        p.stroke_path(
            &line.finish().unwrap(),
            &paint,
            &stroke,
            Transform::identity(),
            None,
        );
        let (x, y) = self.point(POINTS - 1, values[POINTS - 1]);
        let mut white = Paint::default();
        white.set_color_rgba8(255, 255, 255, 255);
        white.anti_alias = true;
        fill_circle(&mut p, x, y, 9.0, &white);
        fill_circle(&mut p, x, y, 6.0, &paint);
        p
    }
}

/// A soft landscape at dusk: a sky from deep blue to peach, a low sun's glow and rolling
/// hills, all in code.
fn wallpaper(p: &mut Pixmap) {
    let (w, h) = (W as f32, H as f32);
    let sky = LinearGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(w * 0.2, h),
        vec![
            GradientStop::new(0.0, Color::from_rgba8(0x1F, 0x2A, 0x5C, 255)),
            GradientStop::new(0.45, Color::from_rgba8(0x5B, 0x4E, 0x9A, 255)),
            GradientStop::new(0.8, Color::from_rgba8(0xE8, 0x9A, 0x8C, 255)),
            GradientStop::new(1.0, Color::from_rgba8(0xF6, 0xC8, 0x9F, 255)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    fill_shader(p, sky);
    let glow = RadialGradient::new(
        Point::from_xy(w * 0.68, h * 0.74),
        0.0,
        Point::from_xy(w * 0.68, h * 0.74),
        h * 0.6,
        vec![
            GradientStop::new(0.0, Color::from_rgba8(0xFF, 0xE3, 0xB8, 210)),
            GradientStop::new(1.0, Color::from_rgba8(0xFF, 0xE3, 0xB8, 0)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    fill_shader(p, glow);
    // Hills, back to front.
    for (base, amp, freq, shift, top, bottom) in [
        (
            0.70,
            0.05,
            1.6,
            0.3,
            (0x8C, 0x6F, 0xA8, 200),
            (0x6A, 0x5A, 0x96, 230),
        ),
        (
            0.79,
            0.045,
            2.3,
            1.7,
            (0x5E, 0x4F, 0x8C, 235),
            (0x43, 0x3C, 0x74, 245),
        ),
        (
            0.88,
            0.04,
            1.2,
            4.1,
            (0x37, 0x31, 0x63, 250),
            (0x24, 0x22, 0x48, 255),
        ),
    ] {
        let mut b = PathBuilder::new();
        b.move_to(0.0, h);
        let steps = 96;
        for i in 0..=steps {
            let x = w * i as f32 / steps as f32;
            let u = x / w * std::f32::consts::TAU;
            let y = h
                * (base
                    - amp * (u * freq + shift).sin()
                    - amp * 0.4 * (u * freq * 2.7 + shift * 1.3).sin());
            b.line_to(x, y);
        }
        b.line_to(w, h);
        b.close();
        let shade = LinearGradient::new(
            Point::from_xy(0.0, h * (base - amp)),
            Point::from_xy(0.0, h),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(top.0, top.1, top.2, top.3)),
                GradientStop::new(
                    1.0,
                    Color::from_rgba8(bottom.0, bottom.1, bottom.2, bottom.3),
                ),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        let paint = Paint {
            shader: shade,
            anti_alias: true,
            ..Default::default()
        };
        p.fill_path(
            &b.finish().unwrap(),
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// A plain app window: a soft shadow, a white body with rounded corners, a light title bar
/// with `title` and three neutral dots.
fn window(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, title: &str) {
    let r = 14.0;
    let mut shadow = Paint {
        anti_alias: true,
        ..Default::default()
    };
    for i in (1..=14).rev() {
        let g = i as f32 * 2.0;
        shadow.set_color_rgba8(0x10, 0x10, 0x30, 5);
        fill_rounded(
            p,
            x - g,
            y - g + 10.0,
            w + 2.0 * g,
            h + 2.0 * g,
            r + g,
            &shadow,
        );
    }
    let mut body = Paint {
        anti_alias: true,
        ..Default::default()
    };
    body.set_color_rgba8(0xFF, 0xFF, 0xFF, 255);
    fill_rounded(p, x, y, w, h, r, &body);
    body.set_color_rgba8(0xF3, 0xF4, 0xF6, 255);
    fill_rounded(p, x, y, w, 46.0, r, &body);
    fill_rect(p, x, y + 30.0, w, 16.0, &body);
    body.set_color_rgba8(0xE5, 0xE7, 0xEB, 255);
    fill_rect(p, x, y + 46.0, w, 1.0, &body);
    label(p, title, x + 20.0, y + 12.0, 19.0, rgb((0x37, 0x41, 0x51)));
    body.set_color_rgba8(0xC9, 0xCD, 0xD3, 255);
    for i in 0..3 {
        fill_circle(p, x + w - 30.0 - i as f32 * 24.0, y + 23.0, 6.0, &body);
    }
}

// ---- The capture notice ----

/// The notice after a capture, as `overmark-win`'s toast paints it (its colours, sizes and
/// strings), at the bottom right.
struct Toast {
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    at: (usize, usize),
}

impl Toast {
    fn new() -> Toast {
        let file = r"C:\Users\you\Pictures\Overmark\2026-11-01 09.42.17.png";
        let title = tr!("toast-copied-and-saved", file = file);
        let hint = overmark_core::i18n::tr("toast-hint");
        let s = SCALE;
        let width = 440.0 * s;
        let pad = 14.0 * s;
        let gap = 4.0 * s;
        let (title_size, line_size) = (15.0 * s, 13.0 * s);
        let title_lines = wrap(&title, title_size, width - 2.0 * pad);
        let line_height = |size: f32| text::shape("Ag", size).height as f32;
        let height = pad * 2.0
            + title_lines.len() as f32 * line_height(title_size)
            + gap
            + line_height(line_size);
        let (w, h) = (width as usize, height.ceil() as usize);
        let mut p = Pixmap::new(w as u32, h as u32).unwrap();
        p.fill(Color::from_rgba8(0x28, 0x2B, 0x30, 255));
        let mut y = pad;
        for line in &title_lines {
            label(&mut p, line, pad, y, title_size, Color::WHITE);
            y += line_height(title_size);
        }
        y += gap;
        label(&mut p, &hint, pad, y, line_size, rgb((0xD0, 0xD0, 0xD0)));
        let margin = (16.0 * s) as usize;
        Toast {
            rgba: p.data().to_vec(),
            w,
            h,
            at: (W as usize - margin - w, H as usize - margin - h),
        }
    }
}

/// `text` broken into lines no wider than `width` at `size`, after a space or a backslash
/// (a path breaks at its folders).
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut word = String::new();
    let flush = |lines: &mut Vec<String>, word: &mut String| {
        let line = lines.last_mut().unwrap();
        let joined = format!("{line}{word}");
        if !line.is_empty() && text::shape(joined.trim_end(), size).width as f32 > width {
            lines.push(std::mem::take(word));
        } else {
            *line = joined;
            word.clear();
        }
    };
    for c in text.chars() {
        word.push(c);
        if c == ' ' || c == '\\' {
            flush(&mut lines, &mut word);
        }
    }
    flush(&mut lines, &mut word);
    lines.iter().map(|l| l.trim_end().to_owned()).collect()
}

// ---- Drawing helpers ----

/// `s` in the system font, its top-left at (`x`, `y`): the glyphs core lays out for the Text
/// tool.
fn label(p: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, colour: Color) {
    let shape = text::shape(s, size);
    let Some(path) = skia_path(&shape.glyphs) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(colour);
    paint.anti_alias = true;
    p.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::from_translate(x, y),
        None,
    );
}

fn skia_path(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => b.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, c, p) => b.cubic_to(
                a.x as f32, a.y as f32, c.x as f32, c.y as f32, p.x as f32, p.y as f32,
            ),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

fn fill_rect(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, paint: &Paint) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        p.fill_rect(r, paint, Transform::identity(), None);
    }
}

fn fill_circle(p: &mut Pixmap, x: f32, y: f32, r: f32, paint: &Paint) {
    if let Some(c) = PathBuilder::from_circle(x, y, r) {
        p.fill_path(&c, paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_rounded(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, paint: &Paint) {
    let r = r.min(w / 2.0).min(h / 2.0);
    // A circle's quarter as a cubic.
    let k = r * 0.552_285;
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    b.close();
    if let Some(path) = b.finish() {
        p.fill_path(&path, paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_shader(p: &mut Pixmap, shader: Shader) {
    let paint = Paint {
        shader,
        ..Default::default()
    };
    fill_rect(p, 0.0, 0.0, W as f32, H as f32, &paint);
}

fn fill_all(p: &mut Pixmap, colour: Color) {
    let mut paint = Paint::default();
    paint.set_color(colour);
    fill_rect(p, 0.0, 0.0, W as f32, H as f32, &paint);
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn a8(alpha: f32) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Premultiplied `rgba` (`w` × `h`) over `frame` with its top-left at `at`.
fn over(frame: &mut Pixmap, rgba: &[u8], w: usize, h: usize, at: (usize, usize)) {
    let fw = frame.width() as usize;
    let data = frame.data_mut();
    for y in 0..h {
        for x in 0..w {
            let s = &rgba[(y * w + x) * 4..][..4];
            if s[3] == 0 {
                continue;
            }
            let (fx, fy) = (at.0 + x, at.1 + y);
            if fx >= fw || fy >= H as usize {
                continue;
            }
            let d = &mut data[(fy * fw + fx) * 4..][..4];
            let k = 255 - u16::from(s[3]);
            for c in 0..4 {
                d[c] = (u16::from(s[c]) + (u16::from(d[c]) * k + 127) / 255).min(255) as u8;
            }
        }
    }
}

fn save_png(p: &Pixmap, path: &Path) {
    // Every frame is opaque, so its premultiplied pixels are its colours.
    let rgb: Vec<u8> = p
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect();
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        file,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    );
    image::ImageEncoder::write_image(
        encoder,
        &rgb,
        p.width(),
        p.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
}

// ---- Encoding ----

/// The GIF for the README, the MP4 and poster for the website, with ffmpeg.
fn encode(out: &Path, frames: &Path) {
    let input = frames.join("%04d.png");
    let rate = FPS.to_string();
    let gif = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/overmark-demo.gif");
    let mp4 = out.join("overmark-demo.mp4");
    let runs: [(&Path, Vec<&str>); 2] = [
        (
            &gif,
            vec![
                "-vf",
                "fps=15,scale=960:-1:flags=lanczos,split[a][b];\
                 [a]palettegen=max_colors=256:stats_mode=full[p];\
                 [b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle",
                "-loop",
                "0",
            ],
        ),
        (
            &mp4,
            vec![
                "-an",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "18",
                "-preset",
                "slow",
                "-movflags",
                "+faststart",
            ],
        ),
    ];
    for (target, args) in runs {
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-framerate", &rate, "-i"])
            .arg(&input)
            .args(args)
            .arg(target)
            .status();
        match status {
            Ok(s) if s.success() => {
                let size = std::fs::metadata(target).map_or(0, |m| m.len());
                println!("{} ({:.1} MB)", target.display(), size as f64 / 1e6);
            }
            Ok(s) => eprintln!("ffmpeg failed ({s}) for {}", target.display()),
            Err(e) => {
                eprintln!(
                    "ffmpeg isn't available ({e}): the frames are in {}",
                    frames.display()
                );
                return;
            }
        }
    }
    println!("{}", out.join("overmark-demo-poster.png").display());
}
