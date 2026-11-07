//! The toolbar's own line icons, drawn in code like `ui::icons` (no icon font or image ships:
//! the licence rule). The shared ones (copy, bin, cross, more) come from `ui::icons`.

use std::f32::consts::{FRAC_PI_2, PI};

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use overmark_core::backdrop::{Backdrop, Guides};
use overmark_core::tool::ToolKind;

/// The toolbar's icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// A tray with an arrow down into it.
    Save,
    /// A dashed rectangle.
    Region,
    /// A snowflake: the Frozen backdrop.
    Freeze,
    /// Two bars: freeze the screen, as a media pause.
    Pause,
    /// A triangle pointing right: let the screen move again.
    Play,
    /// A dot in a ring: record a video.
    Record,
    /// A square: stop recording.
    Stop,
    /// An arrow curving back to the left.
    Undo,
    /// An arrow curving on to the right.
    Redo,
    /// A chevron pointing up (collapse) or down (expand).
    ChevronUp,
    ChevronDown,
    /// Six dots, for dragging the toolbar.
    Grip,
    /// A stroke fading to dots: disappearing ink.
    Fade,
    // The tools (their strip).
    /// A pencil.
    Pen,
    /// A marker over a broad tinted band.
    Highlighter,
    /// An arrow up to the right.
    Arrow,
    Rectangle,
    Ellipse,
    /// A line up to the right.
    Line,
    /// A capital T.
    Text,
    /// A circled 1.
    Stamp,
    /// Blocks of differing shade.
    Blur,
    /// A tilted block over a line.
    Eraser,
    /// A dot with rays.
    Laser,
    // The backdrops (Frozen is the snowflake).
    /// A screen on its stand.
    Live,
    /// A board on an easel, empty or filled.
    Whiteboard,
    Blackboard,
    // The guides.
    /// A square struck through.
    NoGuides,
    Grid,
    Dots,
    /// Ruled lines.
    Lines,
}

impl Glyph {
    /// A tool's icon.
    pub fn tool(t: ToolKind) -> Glyph {
        match t {
            ToolKind::Pen => Glyph::Pen,
            ToolKind::Highlighter => Glyph::Highlighter,
            ToolKind::Arrow => Glyph::Arrow,
            ToolKind::Rectangle => Glyph::Rectangle,
            ToolKind::Ellipse => Glyph::Ellipse,
            ToolKind::Line => Glyph::Line,
            ToolKind::Text => Glyph::Text,
            ToolKind::Stamp => Glyph::Stamp,
            ToolKind::Blur => Glyph::Blur,
            ToolKind::Eraser => Glyph::Eraser,
            ToolKind::Laser => Glyph::Laser,
        }
    }

    /// A backdrop's icon.
    pub fn backdrop(b: Backdrop) -> Glyph {
        match b {
            Backdrop::Live => Glyph::Live,
            Backdrop::Frozen => Glyph::Freeze,
            Backdrop::Whiteboard => Glyph::Whiteboard,
            Backdrop::Blackboard => Glyph::Blackboard,
        }
    }

    /// The guides' icon.
    pub fn guides(g: Guides) -> Glyph {
        match g {
            Guides::Off => Glyph::NoGuides,
            Guides::Grid => Glyph::Grid,
            Guides::Dots => Glyph::Dots,
            Guides::Lines => Glyph::Lines,
        }
    }
}

impl Glyph {
    /// Draws the icon in `rect` (made square, centred) with `colour`.
    pub fn paint(self, painter: &Painter, rect: Rect, colour: Color32) {
        let side = rect.width().min(rect.height());
        let rect = Rect::from_center_size(rect.center(), vec2(side, side));
        let width = (side / 11.0).max(1.25);
        let stroke = Stroke::new(width, colour);
        let at = |x: f32, y: f32| pos2(rect.left() + x * side, rect.top() + y * side);
        let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
        let arc = |centre: Pos2, r: f32, from: f32, to: f32| {
            let n = 16;
            let points: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let t = from + (to - from) * i as f32 / n as f32;
                    pos2(centre.x + r * t.cos(), centre.y + r * t.sin())
                })
                .collect();
            painter.add(Shape::line(points, stroke));
        };
        match self {
            Glyph::Save => {
                line(at(0.5, 0.08), at(0.5, 0.62));
                line(at(0.3, 0.42), at(0.5, 0.62));
                line(at(0.7, 0.42), at(0.5, 0.62));
                painter.add(Shape::line(
                    vec![at(0.1, 0.6), at(0.1, 0.9), at(0.9, 0.9), at(0.9, 0.6)],
                    stroke,
                ));
            }
            Glyph::Region => {
                // Dashes round a rectangle, and its corners.
                let (l, t, r, b) = (0.1, 0.2, 0.9, 0.8);
                for i in 0..4 {
                    let x0 = l + (r - l) * i as f32 / 4.0;
                    let x1 = x0 + (r - l) / 8.0;
                    line(at(x0, t), at(x1, t));
                    line(at(x0, b), at(x1, b));
                }
                for i in 0..3 {
                    let y0 = t + (b - t) * i as f32 / 3.0;
                    let y1 = y0 + (b - t) / 6.0;
                    line(at(l, y0), at(l, y1));
                    line(at(r, y0), at(r, y1));
                }
                line(at(r - 0.06, b), at(r, b));
                line(at(r, b - 0.08), at(r, b));
            }
            Glyph::Freeze => {
                let c = at(0.5, 0.5);
                for k in 0..3 {
                    let a = k as f32 * PI / 3.0 + FRAC_PI_2;
                    let d = vec2(a.cos(), a.sin()) * side * 0.42;
                    line(c - d, c + d);
                    for end in [1.0f32, -1.0] {
                        let tip = c + d * end * 0.7;
                        let side_dir = vec2(-a.sin(), a.cos()) * side * 0.12;
                        line(tip, tip + d * end * 0.25 + side_dir);
                        line(tip, tip + d * end * 0.25 - side_dir);
                    }
                }
            }
            Glyph::Pause => {
                line(at(0.36, 0.2), at(0.36, 0.8));
                line(at(0.64, 0.2), at(0.64, 0.8));
            }
            Glyph::Play => {
                painter.add(Shape::closed_line(
                    vec![at(0.3, 0.18), at(0.82, 0.5), at(0.3, 0.82)],
                    stroke,
                ));
            }
            Glyph::Record => {
                painter.circle_stroke(at(0.5, 0.5), side * 0.42, stroke);
                painter.circle_filled(at(0.5, 0.5), side * 0.24, colour);
            }
            Glyph::Stop => {
                painter.rect_filled(
                    Rect::from_min_max(at(0.24, 0.24), at(0.76, 0.76)),
                    side * 0.06,
                    colour,
                );
            }
            Glyph::Undo | Glyph::Redo => {
                let flip = |x: f32| if self == Glyph::Undo { x } else { 1.0 - x };
                // A half circle from the arrow's head round to its tail.
                let centre = at(0.5, 0.58);
                let r = side * 0.3;
                if self == Glyph::Undo {
                    arc(centre, r, PI, 2.0 * PI + 0.35);
                } else {
                    arc(centre, r, -0.35 - PI, 0.0);
                }
                let head = at(flip(0.2), 0.58);
                line(head, at(flip(0.07), 0.42));
                line(head, at(flip(0.35), 0.45));
            }
            Glyph::ChevronUp => {
                painter.add(Shape::line(
                    vec![at(0.2, 0.65), at(0.5, 0.35), at(0.8, 0.65)],
                    stroke,
                ));
            }
            Glyph::ChevronDown => {
                painter.add(Shape::line(
                    vec![at(0.2, 0.35), at(0.5, 0.65), at(0.8, 0.35)],
                    stroke,
                ));
            }
            Glyph::Fade => {
                // A wave drawn whole on the left, breaking into dots that shrink to the right.
                let wave = |t: f32| at(t, 0.5 - 0.18 * (t * std::f32::consts::TAU).sin());
                let solid: Vec<Pos2> = (0..=10)
                    .map(|i| wave(0.08 + 0.4 * i as f32 / 10.0))
                    .collect();
                painter.add(Shape::line(solid, stroke));
                for (k, t) in [0.6f32, 0.74, 0.88].into_iter().enumerate() {
                    painter.circle_filled(wave(t), width * (0.9 - 0.2 * k as f32), colour);
                }
            }
            Glyph::Pen => {
                // A pencil from bottom left to top right: its sides, its end and its point.
                line(at(0.26, 0.64), at(0.66, 0.24));
                line(at(0.36, 0.74), at(0.76, 0.34));
                line(at(0.66, 0.24), at(0.76, 0.34));
                line(at(0.26, 0.64), at(0.14, 0.86));
                line(at(0.36, 0.74), at(0.14, 0.86));
            }
            Glyph::Highlighter => {
                painter.line_segment(
                    [at(0.12, 0.84), at(0.88, 0.84)],
                    Stroke::new(width * 3.0, colour.gamma_multiply(0.45)),
                );
                painter.add(Shape::closed_line(
                    vec![at(0.32, 0.62), at(0.62, 0.16), at(0.8, 0.28), at(0.5, 0.74)],
                    stroke,
                ));
                line(at(0.32, 0.62), at(0.3, 0.76));
                line(at(0.5, 0.74), at(0.3, 0.76));
            }
            Glyph::Arrow => {
                line(at(0.18, 0.82), at(0.8, 0.2));
                line(at(0.8, 0.2), at(0.46, 0.22));
                line(at(0.8, 0.2), at(0.78, 0.54));
            }
            Glyph::Rectangle => {
                painter.rect_stroke(
                    Rect::from_min_max(at(0.12, 0.24), at(0.88, 0.76)),
                    0.0,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            Glyph::Ellipse => {
                let c = at(0.5, 0.5);
                let points: Vec<Pos2> = (0..32)
                    .map(|i| {
                        let t = i as f32 / 32.0 * std::f32::consts::TAU;
                        pos2(c.x + 0.4 * side * t.cos(), c.y + 0.28 * side * t.sin())
                    })
                    .collect();
                painter.add(Shape::closed_line(points, stroke));
            }
            Glyph::Line => {
                line(at(0.14, 0.86), at(0.86, 0.14));
            }
            Glyph::Text => {
                line(at(0.2, 0.18), at(0.8, 0.18));
                line(at(0.5, 0.18), at(0.5, 0.86));
            }
            Glyph::Stamp => {
                painter.circle_stroke(at(0.5, 0.5), side * 0.4, stroke);
                line(at(0.52, 0.3), at(0.52, 0.7));
                line(at(0.52, 0.3), at(0.4, 0.4));
            }
            Glyph::Blur => {
                let shades = [0.9, 0.35, 0.65, 0.4, 1.0, 0.3, 0.7, 0.3, 0.85];
                let cell = 0.76 / 3.0;
                for (i, a) in shades.into_iter().enumerate() {
                    let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
                    let min = at(0.12 + cx * cell, 0.12 + cy * cell);
                    let r = Rect::from_min_size(min, vec2(cell * side, cell * side));
                    painter.rect_filled(r.shrink(0.5), 1.0, colour.gamma_multiply(a));
                }
            }
            Glyph::Eraser => {
                painter.add(Shape::closed_line(
                    vec![at(0.12, 0.56), at(0.46, 0.22), at(0.84, 0.6), at(0.5, 0.94)],
                    stroke,
                ));
                line(at(0.29, 0.39), at(0.67, 0.77));
                line(at(0.5, 0.94), at(0.92, 0.94));
            }
            Glyph::Laser => {
                let c = at(0.5, 0.5);
                painter.circle_filled(c, side * 0.13, colour);
                for k in 0..8 {
                    let a = k as f32 * PI / 4.0;
                    let d = vec2(a.cos(), a.sin()) * side;
                    line(c + d * 0.26, c + d * 0.42);
                }
            }
            Glyph::Live => {
                painter.rect_stroke(
                    Rect::from_min_max(at(0.08, 0.14), at(0.92, 0.7)),
                    2.0,
                    stroke,
                    egui::StrokeKind::Middle,
                );
                line(at(0.5, 0.7), at(0.5, 0.86));
                line(at(0.3, 0.86), at(0.7, 0.86));
                painter.add(Shape::convex_polygon(
                    vec![at(0.42, 0.3), at(0.62, 0.42), at(0.42, 0.54)],
                    colour,
                    Stroke::NONE,
                ));
            }
            Glyph::Whiteboard | Glyph::Blackboard => {
                let board = Rect::from_min_max(at(0.1, 0.12), at(0.9, 0.68));
                if self == Glyph::Blackboard {
                    painter.rect_filled(board, 2.0, colour);
                } else {
                    painter.rect_stroke(board, 2.0, stroke, egui::StrokeKind::Middle);
                }
                line(at(0.32, 0.68), at(0.22, 0.9));
                line(at(0.68, 0.68), at(0.78, 0.9));
            }
            Glyph::NoGuides | Glyph::Grid => {
                let r = Rect::from_min_max(at(0.14, 0.14), at(0.86, 0.86));
                painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Middle);
                if self == Glyph::NoGuides {
                    line(at(0.14, 0.86), at(0.86, 0.14));
                } else {
                    for t in [0.38, 0.62] {
                        line(at(t, 0.14), at(t, 0.86));
                        line(at(0.14, t), at(0.86, t));
                    }
                }
            }
            Glyph::Dots => {
                for y in [0.22, 0.5, 0.78] {
                    for x in [0.22, 0.5, 0.78] {
                        painter.circle_filled(at(x, y), width * 1.1, colour);
                    }
                }
            }
            Glyph::Lines => {
                for y in [0.25, 0.5, 0.75] {
                    line(at(0.12, y), at(0.88, y));
                }
            }
            Glyph::Grip => {
                for (x, y) in [
                    (0.35, 0.25),
                    (0.65, 0.25),
                    (0.35, 0.5),
                    (0.65, 0.5),
                    (0.35, 0.75),
                    (0.65, 0.75),
                ] {
                    painter.circle_filled(at(x, y), width * 0.9, colour);
                }
            }
        }
    }
}
