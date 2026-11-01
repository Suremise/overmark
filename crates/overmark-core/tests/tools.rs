//! The tools through the public API: the highlighter's modes, blur, and region redraws with
//! every tool.

use overmark_core::colour::Colour;
use overmark_core::drawing::InkPoint;
use overmark_core::drawing::ItemId;
use overmark_core::render::{
    self, Background, Highlight, LaserLook, RenderOptions, TINT_ALPHA, View, dirty_rect,
    render_highlights, render_region_with, render_with,
};
use overmark_core::session::Session;
use overmark_core::tool::ToolKind;
use tiny_skia::{Color, IntRect, Pixmap};

const YELLOW: Colour = Colour::rgb(0xFD, 0xD8, 0x35);

fn at(x: f64, y: f64) -> InkPoint {
    InkPoint::mouse(x, y, 0)
}

/// A highlighter stroke across a 100×40 layer at y = 20.
fn highlighted() -> Session {
    let mut s = Session::new();
    s.tool = ToolKind::Highlighter;
    s.colour = YELLOW;
    s.width = 16.0;
    s.pointer_down(at(5.0, 20.0), false).unwrap();
    for x in (10..=95).step_by(5) {
        s.pointer_move(at(f64::from(x), 20.0), false);
    }
    s.pointer_up(at(95.0, 20.0), false);
    s
}

fn pixel(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = p.pixel(x, y).unwrap();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

fn screen(colour: [u8; 4]) -> Pixmap {
    let mut p = Pixmap::new(100, 40).unwrap();
    p.fill(Color::from_rgba8(
        colour[0], colour[1], colour[2], colour[3],
    ));
    p
}

#[test]
fn the_highlighter_multiplies_over_a_frame_and_keeps_dark_text_dark() {
    let s = highlighted();
    let options = RenderOptions::default(); // Auto: Multiply over an image.
    for (under, expected) in [
        ([255, 255, 255, 255], [0xFD, 0xD8, 0x35, 255]), // white × yellow = yellow
        ([20, 20, 20, 255], [20, 17, 4, 255]),           // dark text stays dark
    ] {
        let frame = screen(under);
        let mut out = Pixmap::new(100, 40).unwrap();
        render_with(
            s.drawing(),
            &mut out.as_mut(),
            &options,
            Background::Image(frame.as_ref()),
        );
        let got = pixel(&out, 50, 20);
        for c in 0..4 {
            assert!(
                got[c].abs_diff(expected[c]) <= 2,
                "over {under:?}: {got:?} vs {expected:?}"
            );
        }
        // Outside the stroke, the frame as it was.
        assert_eq!(pixel(&out, 50, 2), under);
    }
}

#[test]
fn darken_takes_the_minimum() {
    let s = highlighted();
    let options = RenderOptions {
        highlight: Highlight::Darken,
        ..RenderOptions::default()
    };
    let frame = screen([100, 255, 255, 255]);
    let mut out = Pixmap::new(100, 40).unwrap();
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &options,
        Background::Image(frame.as_ref()),
    );
    assert_eq!(pixel(&out, 50, 20), [100, 0xD8, 0x35, 255]);
}

#[test]
fn on_the_live_layer_it_is_a_translucent_tint_or_left_to_its_own_layer() {
    let s = highlighted();
    let mut out = Pixmap::new(100, 40).unwrap();
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &RenderOptions::default(),
        Background::Transparent,
    );
    let alpha = f32::from(pixel(&out, 50, 20)[3]) / 255.0;
    assert!((alpha - TINT_ALPHA).abs() < 0.01, "{alpha}");

    let skip = RenderOptions {
        highlight: Highlight::Skip,
        ..RenderOptions::default()
    };
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &skip,
        Background::Transparent,
    );
    assert!(out.pixels().iter().all(|p| p.alpha() == 0));

    // Its own layer: opaque yellow, for a MIN_BLEND (darken) window.
    render_highlights(s.drawing(), &mut out.as_mut(), &skip);
    assert_eq!(pixel(&out, 50, 20), [0xFD, 0xD8, 0x35, 255]);
    assert_eq!(pixel(&out, 50, 2)[3], 0);
}

#[test]
fn render_highlights_leaves_out_other_ink() {
    let mut s = highlighted();
    s.tool = ToolKind::Pen;
    s.colour = Colour::rgb(0, 0, 255);
    s.pointer_down(at(5.0, 5.0), false).unwrap();
    s.pointer_move(at(95.0, 5.0), false);
    s.pointer_up(at(95.0, 5.0), false);
    let mut out = Pixmap::new(100, 40).unwrap();
    render_highlights(s.drawing(), &mut out.as_mut(), &RenderOptions::default());
    assert!(out.pixels().iter().all(|p| p.blue() < 200 || p.red() > 200));
}

/// A frame with detail to pixelate: a fine checkerboard.
fn checkerboard(w: u32, h: u32) -> Pixmap {
    let mut p = Pixmap::new(w, h).unwrap();
    for (i, px) in p.data_mut().chunks_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let v = if (x / 2 + y / 2) % 2 == 0 { 30 } else { 230 };
        px.copy_from_slice(&[v, v, v, 255]);
    }
    p
}

#[test]
fn blur_pixelates_the_frame_and_nothing_without_one() {
    let mut s = Session::new();
    s.backdrop = overmark_core::backdrop::Backdrop::Frozen;
    s.tool = ToolKind::Blur;
    s.width = 8.0;
    s.pointer_down(at(16.0, 8.0), false).unwrap();
    s.pointer_up(at(48.0, 32.0), false);
    let frame = checkerboard(64, 40);
    let mut out = Pixmap::new(64, 40).unwrap();
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &RenderOptions::default(),
        Background::Image(frame.as_ref()),
    );
    // Inside: each 8 px block is one flat grey (the checkerboard's average).
    let block = pixel(&out, 16, 8);
    for (x, y) in [(17, 9), (23, 15), (20, 12)] {
        assert_eq!(pixel(&out, x, y), block);
    }
    assert!(block[0] > 100 && block[0] < 160, "{block:?}");
    // Outside: untouched.
    assert_eq!(pixel(&out, 2, 2), pixel(&frame, 2, 2));
    // On a transparent layer, a blur draws nothing.
    render_with(
        s.drawing(),
        &mut out.as_mut(),
        &RenderOptions::default(),
        Background::Transparent,
    );
    assert!(out.pixels().iter().all(|p| p.alpha() == 0));
}

/// Every tool's item, redrawn piece by piece, matches a full render exactly, over a frame and
/// on a transparent layer.
#[test]
fn region_redraws_are_exact_with_every_tool() {
    let (w, h) = (240u32, 160u32);
    let frame = checkerboard(w, h);
    for background in [Background::Transparent, Background::Image(frame.as_ref())] {
        let mut s = Session::new();
        s.backdrop = overmark_core::backdrop::Backdrop::Frozen;
        let options = RenderOptions::default();
        let view = View::default();
        let mut live = Pixmap::new(w, h).unwrap();
        render_with(s.drawing(), &mut live.as_mut(), &options, background);
        let redraw = |s: &Session, live: &mut Pixmap, area: Option<kurbo::Rect>| {
            if let Some(rect) = area.and_then(|a| dirty_rect(a, view, w, h)) {
                render_region_with(s.drawing(), &mut live.as_mut(), &options, background, rect);
            }
        };
        let tools = [
            ToolKind::Pen,
            ToolKind::Highlighter,
            ToolKind::Arrow,
            ToolKind::Rectangle,
            ToolKind::Ellipse,
            ToolKind::Line,
            ToolKind::Blur,
            ToolKind::Stamp,
        ];
        for (k, tool) in tools.into_iter().enumerate() {
            s.tool = tool;
            s.width = 6.0 + k as f32;
            let y = 15.0 + k as f64 * 17.0;
            let area = s.pointer_down(at(20.0 + k as f64 * 3.0, y), false).unwrap();
            redraw(&s, &mut live, area);
            for i in 1..12 {
                let p = at(
                    20.0 + f64::from(i) * 16.0,
                    y + (f64::from(i) * 0.7).sin() * 12.0,
                );
                let area = s.pointer_move(p, k % 2 == 0);
                redraw(&s, &mut live, area);
            }
            let area = s.pointer_up(at(200.0, y + 9.0), false);
            redraw(&s, &mut live, area);
        }
        // An eraser swipe and an undo, redrawn piecemeal too.
        s.tool = ToolKind::Eraser;
        let area = s.pointer_down(at(100.0, 0.0), false).unwrap();
        redraw(&s, &mut live, area);
        let area = s.pointer_up(at(100.0, 60.0), false);
        redraw(&s, &mut live, area);
        let area = s.undo();
        redraw(&s, &mut live, area);

        let mut full = Pixmap::new(w, h).unwrap();
        render_with(s.drawing(), &mut full.as_mut(), &options, background);
        let differ = live
            .pixels()
            .iter()
            .zip(full.pixels())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differ, 0, "{differ} pixels differ");
    }
}

#[test]
fn a_whole_region_is_the_same_as_a_render_with_options() {
    let s = highlighted();
    let frame = checkerboard(100, 40);
    let mut full = Pixmap::new(100, 40).unwrap();
    let mut region = Pixmap::new(100, 40).unwrap();
    let options = RenderOptions::default();
    let background = Background::Image(frame.as_ref());
    render_with(s.drawing(), &mut full.as_mut(), &options, background);
    render_region_with(
        s.drawing(),
        &mut region.as_mut(),
        &options,
        background,
        IntRect::from_xywh(0, 0, 100, 40).unwrap(),
    );
    assert!(full.data() == region.data());
}

// ---- Regressions: blur dependencies and animation clearing ----

fn white(w: u32, h: u32) -> Pixmap {
    let mut p = Pixmap::new(w, h).unwrap();
    p.fill(Color::from_rgba8(255, 255, 255, 255));
    p
}

/// Redraws `area` as a caller would: through dirty_rect and render_region_with.
fn redraw_area(
    s: &Session,
    live: &mut Pixmap,
    options: &RenderOptions,
    background: Background,
    area: Option<kurbo::Rect>,
) {
    let (w, h) = (live.width(), live.height());
    if let Some(rect) = area.and_then(|a| dirty_rect(a, options.view, w, h)) {
        render_region_with(s.drawing(), &mut live.as_mut(), options, background, rect);
    }
}

fn differing(a: &Pixmap, b: &Pixmap) -> usize {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .filter(|(x, y)| x != y)
        .count()
}

/// A red line under a large blur, on a frozen white frame.
fn line_under_blur() -> (Session, ItemId) {
    let mut s = Session::new();
    s.backdrop = overmark_core::backdrop::Backdrop::Frozen;
    s.colour = Colour::rgb(255, 0, 0);
    s.tool = ToolKind::Line;
    s.width = 4.0;
    s.pointer_down(at(40.0, 40.0), false).unwrap();
    s.pointer_up(at(45.0, 40.0), false);
    let line = s.drawing().items().last().unwrap().id;
    s.tool = ToolKind::Blur;
    s.width = 32.0;
    s.pointer_down(at(0.0, 0.0), false).unwrap();
    s.pointer_up(at(96.0, 96.0), false);
    (s, line)
}

#[test]
fn changes_under_a_blur_redraw_the_whole_blur() {
    let frame = white(100, 100);
    let background = Background::Image(frame.as_ref());
    let options = RenderOptions::default();
    type Change = fn(&mut Session, ItemId) -> Option<kurbo::Rect>;
    let changes: [(&str, Change); 3] = [
        ("recolour", |s, line| {
            s.recolour(line, Colour::rgb(0, 0, 255))
        }),
        ("erase", |s, _| {
            s.tool = ToolKind::Eraser;
            let a = s.pointer_down(at(42.0, 40.0), false).unwrap();
            let b = s.pointer_up(at(42.0, 40.0), false);
            a.into_iter().chain(b).reduce(|x, y| x.union(y))
        }),
        ("undo the blur, then redo it", |s, _| {
            let a = s.undo();
            let b = s.redo();
            a.into_iter().chain(b).reduce(|x, y| x.union(y))
        }),
    ];
    for (name, change) in changes {
        let (mut s, line) = line_under_blur();
        let mut live = Pixmap::new(100, 100).unwrap();
        render_with(s.drawing(), &mut live.as_mut(), &options, background);
        let area = change(&mut s, line);
        redraw_area(&s, &mut live, &options, background, area);
        let mut full = Pixmap::new(100, 100).unwrap();
        render_with(s.drawing(), &mut full.as_mut(), &options, background);
        assert_eq!(differing(&live, &full), 0, "{name}");
    }
}

#[test]
fn a_redrawn_region_reports_the_blur_it_grew_to() {
    let (s, _) = line_under_blur();
    let frame = white(100, 100);
    let mut live = Pixmap::new(100, 100).unwrap();
    let small = IntRect::from_xywh(36, 36, 13, 8).unwrap();
    let redrawn = render_region_with(
        s.drawing(),
        &mut live.as_mut(),
        &RenderOptions::default(),
        Background::Image(frame.as_ref()),
        small,
    )
    .unwrap();
    assert_eq!((redrawn.left(), redrawn.top()), (0, 0));
    assert_eq!((redrawn.right(), redrawn.bottom()), (96, 96));
    // Without a frame there's nothing to pixelate, so nothing to grow for.
    let redrawn = render_region_with(
        s.drawing(),
        &mut live.as_mut(),
        &RenderOptions::default(),
        Background::Transparent,
        small,
    )
    .unwrap();
    assert_eq!(redrawn, small);
}

#[test]
fn ink_fading_under_a_blur_redraws_the_blur() {
    let frame = white(100, 100);
    let background = Background::Image(frame.as_ref());
    let (mut s, _) = line_under_blur();
    let mut live = Pixmap::new(100, 100).unwrap();
    let at_time = |now| RenderOptions {
        now_ms: now,
        disappearing_ink_ms: Some(1000),
        ..RenderOptions::default()
    };
    render_with(s.drawing(), &mut live.as_mut(), &at_time(0), background);
    // Frame by frame through the fade, then pruned, as the app would.
    for now in (0..=2000).step_by(100) {
        let options = at_time(now);
        let area = render::animated_area(s.drawing(), &options);
        redraw_area(&s, &mut live, &options, background, area);
        let gone = s.prune_faded(now, 1000);
        redraw_area(&s, &mut live, &options, background, gone);
        let mut full = Pixmap::new(100, 100).unwrap();
        render_with(s.drawing(), &mut full.as_mut(), &options, background);
        assert_eq!(differing(&live, &full), 0, "at {now} ms");
    }
    assert_eq!(
        s.drawing().items().len(),
        1,
        "the line faded; the blur stays"
    );
}

/// The laser (moving, then hidden and still) and a fading stroke, animated frame by frame
/// from animated_area, end exactly as an empty layer: nothing is left behind.
#[test]
fn animations_clear_themselves_to_the_last_pixel() {
    let (w, h) = (120u32, 80u32);
    let background = Background::Transparent;
    let mut s = Session::new();
    s.stroke_begin(InkPoint::mouse(10.0, 60.0, 0));
    s.stroke_extend(InkPoint::mouse(100.0, 60.0, 0));
    s.stroke_end();
    s.tool = ToolKind::Laser;
    s.pointer_down(InkPoint::mouse(50.0, 30.0, 0), false)
        .unwrap();
    for i in 1..10u32 {
        s.pointer_move(
            InkPoint::mouse(50.0 + f64::from(i) * 3.0, 30.0, u64::from(i) * 10),
            false,
        );
    }
    s.laser_hide();
    let trail_ms = 100;
    let mut live = Pixmap::new(w, h).unwrap();
    for now in (0..=2000u64).step_by(7) {
        let laser = s.laser().clone();
        let options = RenderOptions {
            now_ms: now,
            disappearing_ink_ms: Some(300),
            laser: Some(LaserLook {
                laser: &laser,
                colour: Colour::rgb(255, 0, 0),
                trail_ms,
            }),
            ..RenderOptions::default()
        };
        let area = render::animated_area(s.drawing(), &options);
        redraw_area(&s, &mut live, &options, background, area);
        let gone = s.prune_faded(now, 300);
        redraw_area(&s, &mut live, &options, background, gone);
        s.laser_prune(now, trail_ms);
    }
    assert!(s.drawing().is_empty());
    assert!(s.laser().painted_bounds().is_none());
    let empty = Pixmap::new(w, h).unwrap();
    assert_eq!(differing(&live, &empty), 0);
}

#[test]
fn a_still_hidden_laser_is_cleared_after_its_trail() {
    let mut laser = overmark_core::laser::Laser::new();
    laser.move_to(kurbo::Point::new(50.0, 50.0), 0);
    laser.hide();
    let mut live = Pixmap::new(100, 100).unwrap();
    let s = Session::new();
    for now in [0u64, 90, 101, 150] {
        let options = RenderOptions {
            now_ms: now,
            laser: Some(LaserLook {
                laser: &laser,
                colour: Colour::rgb(255, 0, 0),
                trail_ms: 100,
            }),
            ..RenderOptions::default()
        };
        let area = render::animated_area(s.drawing(), &options);
        redraw_area(&s, &mut live, &options, Background::Transparent, area);
        laser.prune(now, 100);
    }
    assert!(live.pixels().iter().all(|p| p.alpha() == 0));
}

#[test]
fn render_region_reports_the_blur_it_grew_to_as_well() {
    let (s, _) = line_under_blur();
    let frame = white(100, 100);
    let mut live = Pixmap::new(100, 100).unwrap();
    let small = IntRect::from_xywh(36, 36, 13, 8).unwrap();
    let redrawn = overmark_core::render::render_region(
        s.drawing(),
        &mut live.as_mut(),
        View::default(),
        Background::Image(frame.as_ref()),
        small,
    )
    .unwrap();
    assert_eq!(
        (
            redrawn.left(),
            redrawn.top(),
            redrawn.right(),
            redrawn.bottom()
        ),
        (0, 0, 96, 96)
    );
    // Outside the target: nothing redrawn.
    let outside = IntRect::from_xywh(500, 500, 10, 10).unwrap();
    assert!(
        overmark_core::render::render_region(
            s.drawing(),
            &mut live.as_mut(),
            View::default(),
            Background::Transparent,
            outside,
        )
        .is_none()
    );
}

// ---- Saves over each backdrop ----

/// A save composed over each backdrop, with guides on and off: the base shows where there's
/// nothing else, the guides only when included, the highlighter darkens, and the ink is on top.
#[test]
fn saves_over_every_backdrop_with_guides_on_and_off() {
    use overmark_core::backdrop::{Backdrop, BackdropFill, GuideLayer, GuideStyle, Guides};
    use overmark_core::composite::{ChannelOrder, SaveBase, SaveLayers, compose_save};
    use overmark_core::render::render_highlights;
    use overmark_core::settings::{BackdropSettings, GuideSettings};

    let (w, h) = (64u32, 48u32);
    let backdrops = BackdropSettings::default();
    let guide_settings = GuideSettings {
        spacing: 16,
        ..GuideSettings::default()
    };
    // The clean grab (Live) and the frozen frame (Frozen): two different opaque images.
    let grab = checkerboard(w, h);
    let mut frame = Pixmap::new(w, h).unwrap();
    frame.fill(Color::from_rgba8(90, 140, 200, 255));

    // Ink: a blue pen stroke along y = 30; a yellow highlighter along y = 10.
    let mut s = Session::new();
    s.tool = ToolKind::Pen;
    s.colour = Colour::rgb(0, 0, 255);
    s.width = 6.0;
    s.pointer_down(at(4.0, 30.0), false).unwrap();
    s.pointer_move(at(60.0, 30.0), false);
    s.pointer_up(at(60.0, 30.0), false);
    s.tool = ToolKind::Highlighter;
    s.colour = YELLOW;
    s.width = 10.0;
    s.pointer_down(at(4.0, 10.0), false).unwrap();
    s.pointer_move(at(60.0, 10.0), false);
    s.pointer_up(at(60.0, 10.0), false);
    let options = RenderOptions {
        highlight: Highlight::Skip,
        ..RenderOptions::default()
    };
    let mut ink = Pixmap::new(w, h).unwrap();
    render_with(
        s.drawing(),
        &mut ink.as_mut(),
        &options,
        Background::Transparent,
    );
    let mut highlight = Pixmap::new(w, h).unwrap();
    render_highlights(s.drawing(), &mut highlight.as_mut(), &options);

    let px = |v: &[u8], x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [v[i], v[i + 1], v[i + 2], v[i + 3]]
    };
    for backdrop in Backdrop::ALL {
        let mut cache = GuideLayer::new();
        let style = GuideStyle::new(&guide_settings, &backdrops, backdrop);
        let guides = cache
            .layer(w, h, Guides::Grid, &style, View::default())
            .unwrap()
            .data()
            .to_vec();
        let base = match backdrop.fill(&backdrops) {
            BackdropFill::Screen => SaveBase::Image(grab.data()),
            BackdropFill::Frame => SaveBase::Image(frame.data()),
            BackdropFill::Solid(colour) => SaveBase::Solid {
                colour,
                order: ChannelOrder::Rgba,
                pixels: (w * h) as usize,
            },
        };
        let bare = compose_save(base, &SaveLayers::default());
        for include_guides in [true, false] {
            let save = compose_save(
                base,
                &SaveLayers {
                    guides: include_guides.then_some(guides.as_slice()),
                    highlight: Some(highlight.data()),
                    ink: Some(ink.data()),
                },
            );
            assert_eq!(save.len(), (w * h * 4) as usize);
            // Off the ink and the guides (a grid cell's middle): the base as it was.
            assert_eq!(px(&save, 40, 40), px(&bare, 40, 40), "{backdrop:?}");
            // On a grid line, away from the ink: the guide shows only when included.
            let on_line = px(&save, 16, 40) != px(&bare, 16, 40);
            assert_eq!(
                on_line, include_guides,
                "{backdrop:?}, guides {include_guides}"
            );
            // The pen's ink is on top: opaque blue.
            assert_eq!(px(&save, 40, 30), [0, 0, 255, 255], "{backdrop:?}");
            // The highlighter darkens: blue goes, as yellow has none.
            assert!(
                px(&save, 40, 10)[2] < 60,
                "{backdrop:?}: {:?}",
                px(&save, 40, 10)
            );
        }
    }
}

/// A board looks the same on screen and in a save, whatever alpha its colour was given: both
/// boards, both channel orders.
#[test]
fn a_board_is_opaque_on_screen_and_in_saves_alike() {
    use overmark_core::backdrop::{Backdrop, BackdropFill, render_solid};
    use overmark_core::composite::{ChannelOrder, SaveBase, SaveLayers, compose_save};
    use overmark_core::settings::BackdropSettings;

    let settings = BackdropSettings {
        whiteboard_colour: Colour {
            a: 128,
            ..Colour::rgb(255, 0, 0)
        },
        blackboard_colour: Colour {
            a: 10,
            ..Colour::rgb(10, 20, 30)
        },
        ..BackdropSettings::default()
    };
    for backdrop in [Backdrop::Whiteboard, Backdrop::Blackboard] {
        let BackdropFill::Solid(colour) = backdrop.fill(&settings) else {
            panic!("a board is solid");
        };
        assert_eq!(colour.a, 255, "{backdrop:?}");
        let mut screen = Pixmap::new(1, 1).unwrap();
        render_solid(&mut screen.as_mut(), colour, None);
        let shown = screen.data().to_vec();
        let rgba = compose_save(
            SaveBase::Solid {
                colour,
                order: ChannelOrder::Rgba,
                pixels: 1,
            },
            &SaveLayers::default(),
        );
        let bgra = compose_save(
            SaveBase::Solid {
                colour,
                order: ChannelOrder::Bgra,
                pixels: 1,
            },
            &SaveLayers::default(),
        );
        assert_eq!(shown, rgba, "{backdrop:?}: screen and RGBA save");
        assert_eq!(
            [bgra[2], bgra[1], bgra[0], bgra[3]].to_vec(),
            rgba,
            "{backdrop:?}: BGRA"
        );
        assert_eq!(shown[3], 255);
    }
}
