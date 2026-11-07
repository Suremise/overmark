//! Choosing a box on the screen: a selection window over each monitor, showing a still image
//! of it darkened, with the box being dragged bright, a hint at the top and the box's size by
//! its corner. Esc cancels. A drag stays on the monitor it began on.
//!
//! Saving a region and recording one both use it: the drawing layer opens it over the composed
//! images of its monitors ([`crate::overlay::Overlay::begin_region`]), and a recording made
//! without the drawing layer can open a [`Picker`] of its own over plain screen grabs.
//! Every window here is excluded from capture, so a share sees the screen go on underneath.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_SYSKEYDOWN};

use crate::overlay::Layer;
use crate::win32::Rect;
use crate::win32::affinity;
use crate::win32::dpi::Monitor;
use crate::win32::window::{self, Handler, Named, Options};

/// How choosing ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    /// A box on monitor `screen` (an index into the monitors given), in its pixels.
    Chosen { screen: usize, rect: Rect },
    /// Esc on any monitor.
    Cancelled,
}

/// Choosing a box: each monitor's selection window, and the drag they share.
pub struct Picker {
    windows: Vec<Selection>,
    chooser: Chooser,
    inputs: Rc<RefCell<Vec<(usize, SelectionInput)>>>,
}

impl Picker {
    /// Opens a selection over each of `monitors`, showing `images[i]` (what monitor `i` looks
    /// like, still) darkened. `focus` (the monitor under the cursor) takes the keyboard, for Esc.
    pub fn open(
        monitors: &[Monitor],
        images: &[crate::save::Image],
        focus: usize,
    ) -> windows::core::Result<Picker> {
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let mut windows = Vec::with_capacity(monitors.len());
        for (i, (monitor, image)) in monitors.iter().zip(images).enumerate() {
            windows.push(Selection::new(i, monitor, image, inputs.clone())?);
        }
        // Shown together, the cursor's monitor's last: it takes the keyboard.
        for w in windows.iter().filter(|w| w.screen != focus) {
            w.layer.window.show(false);
        }
        if let Some(w) = windows
            .iter()
            .find(|w| w.screen == focus)
            .or(windows.first())
        {
            w.layer.window.show(true);
            let took = w.layer.window.focus();
            tracing::debug!(
                screen = w.screen,
                monitor = %monitors[w.screen].name,
                took,
                foreground = %Named(window::foreground()),
                "the selection's keyboard"
            );
        }
        let sizes = windows.iter().map(|w| w.size()).collect();
        Ok(Picker {
            windows,
            chooser: Chooser::new(sizes),
            inputs,
        })
    }

    /// What the selection windows got since the last call, for the overlay to sift with the
    /// layers' own events ([`Picker::chooser`] runs them).
    pub(crate) fn take_inputs(&mut self) -> Vec<(usize, SelectionInput)> {
        std::mem::take(&mut *self.inputs.borrow_mut())
    }

    pub(crate) fn chooser(&mut self) -> &mut Chooser {
        &mut self.chooser
    }

    /// Runs what came since the last call: the drag is shown as it goes, and the outcome comes
    /// once choosing ends (then drop the picker: its windows go).
    pub fn poll(&mut self) -> Option<Picked> {
        for (screen, input) in self.take_inputs() {
            match self.chooser.input(screen, input) {
                Choice::Going(None) => {}
                Choice::Going(Some(changed)) => self.redraw(changed),
                Choice::Chosen { screen, rect } => return Some(Picked::Chosen { screen, rect }),
                Choice::Cancelled => return Some(Picked::Cancelled),
            }
        }
        None
    }

    /// Shows monitor `i`'s drag as it is now.
    pub(crate) fn redraw(&mut self, i: usize) {
        let rect = self.chooser.rect(i);
        if let Some(w) = self.windows.iter_mut().find(|w| w.screen == i) {
            w.rect = rect;
            w.redraw();
        }
    }

    /// Monitor `i`'s selection window (checks drive it with posted mouse messages).
    pub fn window(&self, i: usize) -> Option<HWND> {
        self.windows
            .iter()
            .find(|w| w.screen == i)
            .map(|w| w.layer.window.hwnd())
    }

    /// Whether `hwnd` is one of its windows.
    pub fn owns(&self, hwnd: HWND) -> bool {
        self.windows.iter().any(|w| w.layer.window.hwnd() == hwnd)
    }
}

/// What the selection window got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionInput {
    Down(i32, i32),
    Move(i32, i32),
    Up(i32, i32),
    Escape,
}

/// The rectangle between two points, the far edges included.
pub(crate) fn span(a: (i32, i32), b: (i32, i32)) -> Rect {
    Rect::new(
        a.0.min(b.0),
        a.1.min(b.1),
        a.0.max(b.0) + 1,
        a.1.max(b.1) + 1,
    )
}

/// What a selection input did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Still choosing; the monitor whose drag changed, to be shown again.
    Going(Option<usize>),
    /// A region on monitor `screen`, in its pixels.
    Chosen { screen: usize, rect: Rect },
    /// Esc on any monitor.
    Cancelled,
}

/// The drag across the monitors' selection windows (each monitor's pixels): one at a time, on
/// the monitor where its button went down, clipped to that monitor.
pub(crate) struct Chooser {
    /// Each monitor's size.
    sizes: Vec<(i32, i32)>,
    /// The drag: its monitor, where it started and the rectangle so far.
    drag: Option<(usize, (i32, i32), Rect)>,
}

impl Chooser {
    pub(crate) fn new(sizes: Vec<(i32, i32)>) -> Chooser {
        Chooser { sizes, drag: None }
    }

    /// Monitor `i`'s rectangle so far (empty while nothing is dragged there).
    pub(crate) fn rect(&self, i: usize) -> Rect {
        match self.drag {
            Some((at, _, r)) if at == i => r,
            _ => Rect::default(),
        }
    }

    /// The rectangle from `start` to `to` on monitor `i`, clipped to it (with the mouse
    /// captured, a drag goes on past the monitor's edge).
    fn span_on(&self, i: usize, start: (i32, i32), to: (i32, i32)) -> Rect {
        let (w, h) = self.sizes[i];
        span(start, to).clamp_to(w, h)
    }

    /// Monitor `screen`'s selection window got `input`.
    pub(crate) fn input(&mut self, screen: usize, input: SelectionInput) -> Choice {
        if input == SelectionInput::Escape {
            return Choice::Cancelled;
        }
        if screen >= self.sizes.len() {
            return Choice::Going(None);
        }
        match (input, self.drag) {
            // Another monitor's input while a drag goes on waits for it to end.
            (_, Some((at, ..))) if at != screen => Choice::Going(None),
            (SelectionInput::Down(x, y), _) => {
                let rect = self.span_on(screen, (x, y), (x, y));
                self.drag = Some((screen, (x, y), rect));
                Choice::Going(Some(screen))
            }
            (SelectionInput::Move(x, y), Some((at, start, _))) => {
                self.drag = Some((at, start, self.span_on(at, start, (x, y))));
                Choice::Going(Some(at))
            }
            (SelectionInput::Up(x, y), Some((at, start, _))) => {
                let rect = self.span_on(at, start, (x, y));
                self.drag = None;
                // A click without a drag chooses nothing: keep choosing.
                if rect.width() < 4 || rect.height() < 4 {
                    Choice::Going(Some(at))
                } else {
                    Choice::Chosen { screen: at, rect }
                }
            }
            // A move or a lift with no drag (the button went down elsewhere).
            _ => Choice::Going(None),
        }
    }
}

/// A line of white text on a dark pill, for the region selection (drawn with core's own text
/// shaping and Windows' fonts, as the Text tool is).
struct Label {
    /// BGRA, premultiplied, `width` × `height`.
    pixels: Vec<u8>,
    width: i32,
    height: i32,
}

impl Label {
    /// `text` at 14 points times `scale`; `None` without fonts (nothing to show).
    fn new(text: &str, scale: f32) -> Option<Label> {
        use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
        let size = 14.0 * scale;
        let shape = crate::core::text::shape(text, size);
        if shape.glyphs.elements().is_empty() {
            return None;
        }
        let pad_x = (10.0 * scale).round();
        let pad_y = (5.0 * scale).round();
        let w = (shape.width as f32 + 2.0 * pad_x).ceil() as u32;
        let h = (shape.height as f32 + 2.0 * pad_y).ceil() as u32;
        let mut pixmap = Pixmap::new(w, h)?;
        let mut paint = Paint::default();
        paint.set_color_rgba8(20, 20, 20, 210);
        paint.anti_alias = true;
        let radius = h as f32 / 2.0;
        let pill = {
            let mut pb = PathBuilder::new();
            pb.push_circle(radius, radius, radius);
            pb.push_circle(w as f32 - radius, radius, radius);
            pb.push_rect(tiny_skia::Rect::from_xywh(
                radius,
                0.0,
                (w as f32 - 2.0 * radius).max(0.0),
                h as f32,
            )?);
            pb.finish()?
        };
        pixmap.fill_path(
            &pill,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        let mut pb = PathBuilder::new();
        for el in shape.glyphs.elements() {
            use kurbo::PathEl;
            let p = |pt: kurbo::Point| (pt.x as f32, pt.y as f32);
            match *el {
                PathEl::MoveTo(a) => pb.move_to(p(a).0, p(a).1),
                PathEl::LineTo(a) => pb.line_to(p(a).0, p(a).1),
                PathEl::QuadTo(a, b) => pb.quad_to(p(a).0, p(a).1, p(b).0, p(b).1),
                PathEl::CurveTo(a, b, c) => {
                    pb.cubic_to(p(a).0, p(a).1, p(b).0, p(b).1, p(c).0, p(c).1)
                }
                PathEl::ClosePath => pb.close(),
            }
        }
        if let Some(glyphs) = pb.finish() {
            paint.set_color_rgba8(255, 255, 255, 255);
            pixmap.fill_path(
                &glyphs,
                &paint,
                FillRule::Winding,
                Transform::from_translate(pad_x, pad_y),
                None,
            );
        }
        let mut pixels = pixmap.take();
        for p in pixels.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
        Some(Label {
            pixels,
            width: w as i32,
            height: h as i32,
        })
    }

    /// Draws it over `px` (BGRA, opaque, `w` × `h`) with its top-left at `at`, clipped.
    fn draw(&self, px: &mut [u8], w: i32, h: i32, at: (i32, i32)) {
        for y in 0..self.height {
            let ty = at.1 + y;
            if !(0..h).contains(&ty) {
                continue;
            }
            for x in 0..self.width {
                let tx = at.0 + x;
                if !(0..w).contains(&tx) {
                    continue;
                }
                let s = &self.pixels[((y * self.width + x) * 4) as usize..][..4];
                let keep = 255 - u16::from(s[3]);
                let i = ((ty * w + tx) * 4) as usize;
                for c in 0..3 {
                    px[i + c] = (u16::from(s[c]) + u16::from(px[i + c]) * keep / 255) as u8;
                }
            }
        }
    }
}

/// The region's hint, the text fonts loaded first: a region can be chosen before the layer's
/// first turn has loaded them (the tray's Capture a region…, on a fresh start).
fn hint_label(scale: f32) -> Option<Label> {
    crate::fonts::ensure_loaded();
    Label::new(&crate::core::i18n::tr("region-hint"), scale)
}

/// One monitor's part of choosing a region: a window over it, excluded from capture, showing
/// the image to be saved still, darkened outside the rectangle being dragged.
struct Selection {
    /// Its monitor's index.
    screen: usize,
    layer: Layer,
    origin: (i32, i32),
    /// The monitor's scale, for the labels' size.
    scale: f32,
    /// "Drag to select a region. Esc cancels.", made once (BGRA, premultiplied, and its size).
    hint: Option<Label>,
    /// The image (BGRA, opaque) and its darkened copy.
    bright: Vec<u8>,
    shaded: Vec<u8>,
    width: i32,
    height: i32,
    /// The rectangle being dragged here (empty while there's none).
    rect: Rect,
}

impl Selection {
    /// Monitor `screen`'s selection window, made and filled but not shown yet.
    fn new(
        screen: usize,
        monitor: &Monitor,
        image: &crate::save::Image,
        inputs: Rc<RefCell<Vec<(usize, SelectionInput)>>>,
    ) -> windows::core::Result<Selection> {
        use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
        let xy = |lp: windows::Win32::Foundation::LPARAM| {
            (
                (lp.0 & 0xFFFF) as u16 as i16 as i32,
                ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
            )
        };
        let handler: Handler = Box::new(move |hwnd, msg, wp, lp| match msg {
            WM_LBUTTONDOWN => {
                window::capture_mouse(hwnd);
                inputs
                    .borrow_mut()
                    .push((screen, SelectionInput::Down(xy(lp).0, xy(lp).1)));
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                inputs
                    .borrow_mut()
                    .push((screen, SelectionInput::Move(xy(lp).0, xy(lp).1)));
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                window::release_mouse();
                inputs
                    .borrow_mut()
                    .push((screen, SelectionInput::Up(xy(lp).0, xy(lp).1)));
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if wp.0 == 0x1B => {
                inputs.borrow_mut().push((screen, SelectionInput::Escape));
                Some(LRESULT(0))
            }
            _ => None,
        });
        let mut layer = Layer::new(
            monitor.rect,
            Options {
                title: Some(window::title("window-selection-name")),
                ..Default::default()
            },
            handler,
        )?;
        affinity::set_excluded(layer.window.hwnd(), true)?;
        let bright = image.to_bgra();
        let shaded: Vec<u8> = bright
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let d = |c: u8| (u16::from(c) * 55 / 100) as u8;
                [d(p[0]), d(p[1]), d(p[2]), 255]
            })
            .collect();
        let px = layer.surface.pixels();
        if px.len() == shaded.len() {
            px.copy_from_slice(&shaded);
        }
        let origin = (monitor.rect.left, monitor.rect.top);
        layer.present(origin)?;
        let scale = monitor.scale();
        let hint = hint_label(scale);
        let mut selection = Selection {
            screen,
            layer,
            origin,
            scale,
            hint,
            bright,
            shaded,
            width: monitor.rect.width(),
            height: monitor.rect.height(),
            rect: Rect::default(),
        };
        // The hint from the start, before anything is dragged.
        selection.redraw();
        Ok(selection)
    }

    /// Shows the image darkened but for `rect`, which is bright, with a white edge; the hint
    /// at the top, and the region's size by it while there is one.
    fn redraw(&mut self) {
        let w = self.width;
        let px = self.layer.surface.pixels();
        if px.len() != self.shaded.len() {
            return;
        }
        px.copy_from_slice(&self.shaded);
        let r = self.rect.clamp_to(w, (px.len() as i32) / (w * 4));
        if !r.is_empty() {
            for y in r.top..r.bottom {
                let a = ((y * w + r.left) * 4) as usize;
                let b = ((y * w + r.right) * 4) as usize;
                px[a..b].copy_from_slice(&self.bright[a..b]);
                for x in [r.left, r.right - 1] {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
            for y in [r.top, r.bottom - 1] {
                let a = ((y * w + r.left) * 4) as usize;
                let b = ((y * w + r.right) * 4) as usize;
                for p in px[a..b].as_chunks_mut::<4>().0 {
                    *p = [255, 255, 255, 255];
                }
            }
        }
        let h = (px.len() as i32) / (w * 4);
        let gap = (12.0 * self.scale).round() as i32;
        if let Some(hint) = &self.hint {
            hint.draw(px, w, h, ((w - hint.width) / 2, gap));
        }
        if !r.is_empty() {
            let text = crate::core::tr!(
                "region-size",
                width = r.width().to_string(),
                height = r.height().to_string()
            );
            if let Some(size) = Label::new(&text, self.scale) {
                // Below the region's bottom-right corner, or inside it if there's no room.
                let x = (r.right - size.width).max(0);
                let below = r.bottom + gap / 2;
                let y = if below + size.height <= h {
                    below
                } else {
                    (r.bottom - size.height - gap / 2).max(0)
                };
                size.draw(px, w, h, (x, y));
            }
        }
        let origin = self.origin;
        if let Err(error) = self.layer.present(origin) {
            tracing::warn!(%error, "couldn't show the selection");
        }
    }

    /// Its monitor's size.
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

/// The four strips of a box's outline, `thickness` wide, in screen pixels: each just outside
/// the box's edge, or just inside it where outside would be off `monitor` (a full-screen box,
/// a box at the monitor's edge). Top, bottom, left, right.
pub fn outline_strips(area: Rect, thickness: i32, monitor: Rect) -> [Rect; 4] {
    let t = thickness;
    let top_out = area.top - t >= monitor.top;
    let bottom_out = area.bottom + t <= monitor.bottom;
    let left_out = area.left - t >= monitor.left;
    let right_out = area.right + t <= monitor.right;
    let (l, r) = (
        if left_out { area.left - t } else { area.left },
        if right_out {
            area.right + t
        } else {
            area.right
        },
    );
    let top = if top_out {
        Rect::new(l, area.top - t, r, area.top)
    } else {
        Rect::new(l, area.top, r, area.top + t)
    };
    let bottom = if bottom_out {
        Rect::new(l, area.bottom, r, area.bottom + t)
    } else {
        Rect::new(l, area.bottom - t, r, area.bottom)
    };
    // The sides run between the top and bottom strips.
    let (y0, y1) = (top.bottom, bottom.top);
    let left = if left_out {
        Rect::new(area.left - t, y0, area.left, y1)
    } else {
        Rect::new(area.left, y0, area.left + t, y1)
    };
    let right = if right_out {
        Rect::new(area.right, y0, area.right + t, y1)
    } else {
        Rect::new(area.right - t, y0, area.right, y1)
    };
    [top, bottom, left, right]
}

/// The outline round a box being recorded: four thin windows, excluded from capture (the video
/// never shows them), click-through (drawing and clicks go on underneath) and never activated.
pub struct Outline {
    strips: Vec<(Layer, Rect)>,
}

impl Outline {
    /// The outline of `area` on `monitor`, 2 points thick at `scale`, in `colour` (B, G, R).
    pub fn new(
        area: Rect,
        monitor: Rect,
        scale: f32,
        colour: [u8; 3],
    ) -> windows::core::Result<Outline> {
        let thickness = (2.0 * scale).round().max(1.0) as i32;
        let mut strips = Vec::with_capacity(4);
        for rect in outline_strips(area, thickness, monitor) {
            if rect.is_empty() {
                continue;
            }
            let layer = Layer::new(
                rect,
                Options {
                    click_through: true,
                    no_activate: true,
                    title: Some(window::title("window-record-outline-name")),
                    ..Default::default()
                },
                Box::new(|_, _, _, _| None),
            )?;
            affinity::set_excluded(layer.window.hwnd(), true)?;
            strips.push((layer, rect));
        }
        let mut outline = Outline { strips };
        outline.set_colour(colour)?;
        Ok(outline)
    }

    /// Fills it with `colour` (B, G, R): white before recording, red while recording.
    pub fn set_colour(&mut self, colour: [u8; 3]) -> windows::core::Result<()> {
        for (layer, rect) in &mut self.strips {
            for p in layer.surface.pixels().as_chunks_mut::<4>().0 {
                *p = [colour[0], colour[1], colour[2], 255];
            }
            layer.present((rect.left, rect.top))?;
        }
        Ok(())
    }

    /// Shows it (or brings it back on top), never activating.
    pub fn show(&self) {
        for (layer, _) in &self.strips {
            layer.window.show(false);
        }
    }

    /// Its windows (checks find them, and test that they're excluded from capture).
    pub fn hwnds(&self) -> Vec<HWND> {
        self.strips.iter().map(|(l, _)| l.window.hwnd()).collect()
    }
}

/// Two monitors: A (0) at 1920 × 1080 and B (1) at 2560 × 1440 (for the overlay's tests too).
#[cfg(test)]
pub(crate) fn two_monitors() -> Chooser {
    Chooser::new(vec![(1920, 1080), (2560, 1440)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use SelectionInput::{Down, Escape, Move, Up};

    /// Run alone (nextest gives each test its own process), the fonts aren't loaded yet: as on
    /// a fresh start, when the tray's Capture a region… is the first thing done.
    #[test]
    fn the_hint_is_there_from_a_cold_start() {
        crate::core::i18n::init("en");
        let cold = !crate::core::text::has_fonts();
        assert!(hint_label(1.0).is_some(), "cold start: {cold}");
    }

    #[test]
    fn the_region_labels_are_white_text_on_a_dark_pill_clipped_to_the_screen() {
        assert!(crate::fonts::ensure_loaded(), "Windows' text fonts");
        crate::core::i18n::init("en");
        let hint = Label::new(&crate::core::i18n::tr("region-hint"), 1.0).expect("a label");
        assert!(
            hint.width > 100 && hint.height >= 16,
            "{}x{}",
            hint.width,
            hint.height
        );
        // Over a mid-grey screen, at its top-left and half off the right edge.
        let (w, h) = (300, 60);
        let mut px = [128u8, 128, 128, 255].repeat((w * h) as usize);
        hint.draw(&mut px, w, h, (w - hint.width / 2, 10));
        let at = |x: i32, y: i32| px[((y * w + x) * 4) as usize];
        // Inside the pill: dark or white text (a glyph); outside it: the screen untouched.
        let row: Vec<u8> = ((w - hint.width / 2)..w)
            .map(|x| at(x, 10 + hint.height / 2))
            .collect();
        assert!(row.iter().any(|&v| v < 60), "the pill");
        assert!(row.iter().any(|&v| v > 200), "white text");
        assert_eq!(at(5, 5), 128);
        assert_eq!(at(w - 1, h - 1), 128);
        let size = crate::core::tr!("region-size", width = "640", height = "140");
        assert!(size.contains("640") && size.contains("140"));
        assert!(Label::new(&size, 1.5).is_some());
    }

    /// The selection opened with the cursor on monitor A (it has the keyboard); the drag is on
    /// monitor B: the region is B's, in B's pixels, and A shows no rectangle.
    #[test]
    fn a_region_is_chosen_on_whichever_monitor_the_drag_starts_on() {
        let mut c = two_monitors();
        assert_eq!(c.input(1, Down(100, 200)), Choice::Going(Some(1)));
        assert_eq!(c.input(1, Move(300, 260)), Choice::Going(Some(1)));
        assert_eq!(c.rect(1), Rect::new(100, 200, 301, 261));
        assert!(c.rect(0).is_empty());
        assert_eq!(
            c.input(1, Up(400, 300)),
            Choice::Chosen {
                screen: 1,
                rect: Rect::new(100, 200, 401, 301)
            }
        );
        // And on A, the cursor's monitor, as before.
        let mut c = two_monitors();
        c.input(0, Down(10, 10));
        assert_eq!(
            c.input(0, Up(50, 60)),
            Choice::Chosen {
                screen: 0,
                rect: Rect::new(10, 10, 51, 61)
            }
        );
    }

    /// The mouse is captured by the window the drag began in, so a drag past its monitor's
    /// edge goes on there in that monitor's pixels: the region stops at the edge.
    #[test]
    fn a_region_is_clipped_to_the_monitor_its_drag_began_on() {
        let mut c = two_monitors();
        c.input(0, Down(1800, 1000));
        c.input(0, Move(2500, 1300));
        assert_eq!(c.rect(0), Rect::new(1800, 1000, 1920, 1080));
        assert_eq!(
            c.input(0, Up(-50, 1200)),
            Choice::Chosen {
                screen: 0,
                rect: Rect::new(0, 1000, 1801, 1080)
            }
        );
    }

    /// One drag at a time: another monitor's input meanwhile waits; a click alone chooses
    /// nothing and choosing goes on; Esc on any monitor cancels.
    #[test]
    fn one_drag_at_a_time_and_esc_anywhere_cancels() {
        let mut c = two_monitors();
        c.input(1, Down(100, 100));
        assert_eq!(c.input(0, Down(5, 5)), Choice::Going(None));
        assert_eq!(c.input(0, Up(500, 500)), Choice::Going(None));
        assert!(matches!(
            c.input(1, Up(200, 200)),
            Choice::Chosen { screen: 1, .. }
        ));
        let mut c = two_monitors();
        c.input(0, Down(5, 5));
        assert_eq!(c.input(0, Up(6, 7)), Choice::Going(Some(0)), "a click");
        assert!(c.rect(0).is_empty());
        // A lift or a move with no drag (its button went down elsewhere) does nothing.
        assert_eq!(c.input(1, Up(9, 9)), Choice::Going(None));
        assert_eq!(c.input(1, Move(9, 9)), Choice::Going(None));
        assert_eq!(
            c.input(5, Down(9, 9)),
            Choice::Going(None),
            "no such monitor"
        );
        assert_eq!(c.input(1, Escape), Choice::Cancelled);
        let mut c = two_monitors();
        c.input(1, Down(5, 5));
        assert_eq!(c.input(0, Escape), Choice::Cancelled, "mid-drag, elsewhere");
    }

    #[test]
    fn a_selection_spans_its_corners_either_way() {
        assert_eq!(span((10, 20), (5, 40)), Rect::new(5, 20, 11, 41));
        assert_eq!(span((3, 3), (3, 3)), Rect::new(3, 3, 4, 4));
    }

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn the_outline_is_just_outside_a_box_with_room() {
        let area = Rect::new(100, 200, 500, 400);
        let [top, bottom, left, right] = outline_strips(area, 2, MONITOR);
        assert_eq!(top, Rect::new(98, 198, 502, 200));
        assert_eq!(bottom, Rect::new(98, 400, 502, 402));
        assert_eq!(left, Rect::new(98, 200, 100, 400));
        assert_eq!(right, Rect::new(500, 200, 502, 400));
        // Nothing of it is inside the box: it's never in the video, excluded or not.
        for s in [top, bottom, left, right] {
            assert!(s.intersect(&area).is_empty(), "{s:?}");
        }
    }

    #[test]
    fn a_full_screen_box_has_its_outline_just_inside() {
        let [top, bottom, left, right] = outline_strips(MONITOR, 3, MONITOR);
        assert_eq!(top, Rect::new(0, 0, 1920, 3));
        assert_eq!(bottom, Rect::new(0, 1077, 1920, 1080));
        assert_eq!(left, Rect::new(0, 3, 3, 1077));
        assert_eq!(right, Rect::new(1917, 3, 1920, 1077));
        // A box at the top-left corner only: inside there, outside elsewhere.
        let corner = Rect::new(0, 0, 300, 200);
        let [top, bottom, left, right] = outline_strips(corner, 2, MONITOR);
        assert_eq!(top, Rect::new(0, 0, 302, 2));
        assert_eq!(bottom, Rect::new(0, 200, 302, 202));
        assert_eq!(left, Rect::new(0, 2, 2, 200));
        assert_eq!(right, Rect::new(300, 2, 302, 200));
    }

    /// Real windows of ours, off every screen: each strip is excluded from capture, takes no
    /// clicks, and nothing took the foreground.
    #[test]
    fn the_outlines_windows_are_excluded_click_through_and_never_activated() {
        use windows::Win32::UI::WindowsAndMessaging::{WS_EX_NOACTIVATE, WS_EX_TRANSPARENT};
        crate::core::i18n::init("en");
        let foreground = window::foreground();
        let far = Rect::new(-30_000, -30_000, -29_000, -29_500);
        let monitor = Rect::new(-31_000, -31_000, -28_000, -28_000);
        let outline = Outline::new(far, monitor, 1.0, [0, 0, 255]).expect("an outline");
        outline.show();
        assert_eq!(outline.hwnds().len(), 4);
        for hwnd in outline.hwnds() {
            assert!(
                crate::win32::affinity::is_excluded(hwnd),
                "excluded from capture"
            );
            let ex = window::ex_style(hwnd);
            assert!(ex & WS_EX_TRANSPARENT.0 != 0, "click-through");
            assert!(ex & WS_EX_NOACTIVATE.0 != 0, "never activated");
        }
        assert_eq!(
            window::foreground(),
            foreground,
            "nothing took the foreground"
        );
    }
}
