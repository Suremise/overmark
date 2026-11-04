//! The overlay: what covers each monitor while Overmark is open.
//!
//! What's under the ink is the **backdrop** (core's [`Backdrop`]): Live (the screen, still
//! moving) and Frozen (the screen as it was), F switching them, and the solid boards (W); the
//! guides (G) layer over any backdrop.
//!
//! Per monitor, bottom to top:
//! - **the backdrop layer** (any backdrop but Live): its image, opaque, visible to capture, so
//!   a call's audience sees the still being discussed;
//! - **the veil**: the dim (black, its level from the settings), and in Frozen a slightly
//!   cooler tint; excluded from capture, so the audience never sees it;
//! - **the ink layer**: takes the mouse, pen and keys (no click-through); the strokes are drawn
//!   here.
//!
//! Everything is made when the overlay opens and dropped when it closes, so nothing of it
//! costs memory while Overmark waits in the tray. That is why the layers are layered windows
//! presented with `UpdateLayeredWindow` (no GPU device: about 7 ms a monitor to make) rather
//! than DirectComposition, whose device takes 85-110 ms to make and 30+ MB to keep
//! (docs/ARCHITECTURE.md, The resident app). The ink layer's empty pixels are alpha 1, not 0:
//! Windows lets clicks on a layered window's fully transparent pixels through.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{WM_ERASEBKGND, WM_KEYDOWN, WM_SYSKEYDOWN};

use crate::core::backdrop::{GuideStyle, Guides};
use crate::core::render::View;
use crate::grabber::Grabber;
use crate::win32::Rect;
use crate::win32::affinity;
use crate::win32::capture::{self, Frame};
use crate::win32::clock;
use crate::win32::dpi::{self, Monitor};
use crate::win32::layered::LayeredSurface;
use crate::win32::window::{self, Handler, Options, Window};

pub use crate::core::backdrop::Backdrop;

/// What F switches to: Frozen goes back to Live, anything else freezes.
pub fn freeze_toggled(b: Backdrop) -> Backdrop {
    if b == Backdrop::Frozen {
        Backdrop::Live
    } else {
        Backdrop::Frozen
    }
}

/// The name the logs use (core's `name` is the translated one, for people).
pub fn log_name(b: Backdrop) -> &'static str {
    match b {
        Backdrop::Live => "Live",
        Backdrop::Frozen => "Frozen",
        Backdrop::Whiteboard => "Whiteboard",
        Backdrop::Blackboard => "Blackboard",
    }
}

/// How the backdrops and guides look (Settings' `[backdrops]` and `[guides]`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Look {
    pub backdrops: crate::core::settings::BackdropSettings,
    pub guides: crate::core::settings::GuideSettings,
}

/// What goes into a layer's pixels under the ink.
#[derive(Clone, Copy)]
enum Fill<'a> {
    /// The frozen frame (BGRA).
    Frame(&'a Frame),
    /// A whiteboard's or blackboard's colour.
    Solid(crate::core::colour::Colour),
}

/// A capture asked for from the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Ctrl+S: the cursor's monitor, with the drawing, as the settings' "after a capture" says.
    Save,
    /// Ctrl+Shift+S: drag a rectangle, then as Save.
    SaveRegion,
    /// Ctrl+C: as Save, to the clipboard only.
    Copy,
    /// Ctrl+Shift+Alt+S: as Save, without the drawing (not Ctrl+Alt, which is AltGr).
    SaveWithoutDrawing,
}

impl Command {
    const ALL: [Command; 4] = [
        Command::Save,
        Command::SaveRegion,
        Command::Copy,
        Command::SaveWithoutDrawing,
    ];

    /// The command a key press asks for: `vk` with Ctrl, Shift and Alt as they were.
    pub fn for_key(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<Command> {
        match (vk, ctrl, shift, alt) {
            (0x53, true, false, false) => Some(Command::Save),
            (0x53, true, true, false) => Some(Command::SaveRegion),
            (0x43, true, false, false) => Some(Command::Copy),
            (0x53, true, true, true) => Some(Command::SaveWithoutDrawing),
            _ => None,
        }
    }

    fn index(self) -> usize {
        Command::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// The message that asks the ink layer for this command, as a key would: how the app's
    /// checks drive captures without global input ([`WM_OVERMARK_COMMAND`]).
    pub fn wparam(self) -> usize {
        self.index()
    }

    fn from_wparam(wp: usize) -> Option<Command> {
        Command::ALL.get(wp).copied()
    }
}

/// Posted to an ink layer with a [`Command::wparam`]: runs that command, as its key does.
pub const WM_OVERMARK_COMMAND: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x40;

/// Something the overlay's windows got, for the app to act on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayEvent {
    /// A key went down in the ink layer (a virtual-key code: `VK_ESCAPE` is 0x1B, F is 0x46)
    /// that isn't a capture's, with Ctrl, Shift and Alt as they were.
    Key {
        vk: u32,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
    /// A key went up (for "hold a tool key and scroll").
    KeyUp(u32),
    /// A character typed (`WM_CHAR`), for the Text tool.
    Char(char),
    /// A pointer sample on monitor `screen`'s layer (screen pixels), with Shift as it was.
    Pointer {
        screen: usize,
        phase: Phase,
        sample: crate::win32::pointer::Sample,
        shift: bool,
    },
    /// The mouse's right button went down over monitor `screen`'s layer (never a pen's barrel
    /// button or touch), after that message's samples. It comes mid-stroke too, when the left
    /// button is held: the app decides.
    RightClick { screen: usize },
    /// The wheel turned (`delta` in 120ths of a notch) over monitor `screen`'s layer.
    Wheel { screen: usize, delta: i32 },
    /// A capture's key, or its [`WM_OVERMARK_COMMAND`].
    Command(Command),
    /// A region was chosen on monitor `screen`: `rect` in that monitor's pixels.
    RegionChosen { screen: usize, rect: Rect },
    /// Choosing a region was cancelled (Esc): the overlay stays open.
    RegionCancelled,
}

/// How long an open took, in milliseconds from the moment it was asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenTiming {
    /// Grabbing the screen (Frozen only).
    pub grab_ms: f64,
    /// Making and filling the layers.
    pub layers_ms: f64,
    /// Until they were shown and DWM had composed a frame with them: visible.
    pub visible_ms: f64,
}

/// Frozen's veil: a slightly cooler tint, even with the dim off.
const FROZEN_TINT: (u8, u8, u8) = (12, 28, 64);
/// The least the frozen tint shows, when the dim is lower (0 to 1).
const FROZEN_TINT_MIN: f32 = 0.12;
/// The ink layer's empty pixel: alpha 1, so clicks on it are the layer's.
const EMPTY: [u8; 4] = [0, 0, 0, 1];

struct Layer {
    window: Window,
    surface: LayeredSurface,
}

impl Layer {
    fn new(rect: Rect, options: Options, handler: Handler) -> windows::core::Result<Layer> {
        Ok(Layer {
            window: Window::new(
                rect,
                Options {
                    layered: true,
                    ..options
                },
                handler,
            )?,
            surface: LayeredSurface::new(rect.width(), rect.height())?,
        })
    }

    fn present(&mut self, origin: (i32, i32)) -> windows::core::Result<()> {
        self.surface.present(self.window.hwnd(), origin, None)
    }
}

struct Screen {
    monitor: Monitor,
    backdrop: Option<Layer>,
    /// The frozen frame, kept for saves (the clean image, before any dim or ink).
    frozen: Option<Frame>,
    veil: Window,
    ink: Layer,
    /// The ink, as core renders it: premultiplied RGBA, transparent.
    pixmap: tiny_skia::Pixmap,
    /// The text caret, in layer pixels, and its colour (RGBA).
    caret: Option<Rect>,
    caret_colour: [u8; 4],
    /// While Frozen, what blurs pixelate (the frozen frame, with the guides when shares see
    /// them: what a save's base is when saves include them), and a target to render over it
    /// (made on the first blur; dropped with the frame).
    frozen_rgba: crate::save::Under,
    over_frame: Option<tiny_skia::Pixmap>,
    /// The guides, a layer of their own between the dim and the ink (visible to a share, or
    /// excluded from capture, as Settings say), and core's cache of their pixels.
    guides: Option<Layer>,
    guide_pixels: crate::core::backdrop::GuideLayer,
}

impl Screen {
    /// Copies `r` of the ink into the layer's pixels (BGRA; empty pixels at alpha 1).
    fn blit(&mut self, r: Rect) {
        let w = self.pixmap.width() as i32;
        let r = r.clamp_to(w, self.pixmap.height() as i32);
        let src = self.pixmap.data();
        let dst = self.ink.surface.pixels();
        if dst.len() != src.len() {
            return;
        }
        for y in r.top..r.bottom {
            let a = ((y * w + r.left) * 4) as usize;
            let b = ((y * w + r.right) * 4) as usize;
            for (d, p) in dst[a..b]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src[a..b].as_chunks::<4>().0)
            {
                *d = if p[3] == 0 {
                    EMPTY
                } else {
                    [p[2], p[1], p[0], p[3]]
                };
            }
        }
    }

    /// Draws the blurs in `r` of the ink over the frozen frame: the layer is transparent, and a
    /// blur pixelates what's under it, so its rectangles are rendered over the frame (with any
    /// ink on top) and copied in, opaque. (They sit over the Frozen tint: a blur is a cover.)
    fn blur_pass(
        &mut self,
        drawing: &crate::core::drawing::Drawing,
        options: &crate::core::render::RenderOptions,
        r: Rect,
        guides: Option<&crate::save::SaveGuides>,
    ) {
        use crate::core::render::{self, Background};
        use crate::core::tool::ToolKind;
        let Some(frame) = self.frozen.as_ref() else {
            return;
        };
        let blurs: Vec<Rect> = drawing
            .items()
            .filter(|item| item.tool == ToolKind::Blur)
            .filter_map(|item| drawing.bounds(item.id))
            .map(|b| {
                Rect::new(
                    b.x0.floor() as i32,
                    b.y0.floor() as i32,
                    b.x1.ceil() as i32,
                    b.y1.ceil() as i32,
                )
                .intersect(&r)
            })
            .filter(|b| !b.is_empty())
            .collect();
        if blurs.is_empty() {
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        if (frame.width, frame.height) != (w as i32, h as i32) {
            return;
        }
        if self.over_frame.is_none() {
            self.over_frame = tiny_skia::Pixmap::new(w, h);
        }
        let (Some(image), Some(over)) = (
            self.frozen_rgba.get(frame, guides),
            self.over_frame.as_mut(),
        ) else {
            return;
        };
        let Some(region) = blurs
            .iter()
            .copied()
            .reduce(|a, b| a.union(&b))
            .and_then(|u| {
                tiny_skia::IntRect::from_xywh(u.left, u.top, u.width() as u32, u.height() as u32)
            })
        else {
            return;
        };
        render::render_region_with(
            drawing,
            &mut over.as_mut(),
            options,
            Background::Image(image.as_ref()),
            region,
        );
        let wi = w as i32;
        let (src, dst) = (over.data(), self.pixmap.data_mut());
        for b in blurs {
            for y in b.top..b.bottom {
                let a = ((y * wi + b.left) * 4) as usize;
                let e = ((y * wi + b.right) * 4) as usize;
                dst[a..e].copy_from_slice(&src[a..e]);
            }
        }
    }

    /// Paints the caret over the layer's pixels.
    fn paint_caret(&mut self) {
        let Some(c) = self.caret else {
            return;
        };
        let w = self.pixmap.width() as i32;
        let [r, g, b, _] = self.caret_colour;
        let px = self.ink.surface.pixels();
        for y in c.top..c.bottom {
            for x in c.left..c.right {
                let i = ((y * w + x) * 4) as usize;
                if i + 4 <= px.len() {
                    px[i..i + 4].copy_from_slice(&[b, g, r, 255]);
                }
            }
        }
    }

    /// Shows `r` of the layer.
    fn present(&mut self, r: Rect) {
        let origin = self.origin();
        if let Err(error) = self
            .ink
            .surface
            .present(self.ink.window.hwnd(), origin, Some(r))
        {
            tracing::warn!(%error, "couldn't show the ink");
        }
    }
    fn origin(&self) -> (i32, i32) {
        (self.monitor.rect.left, self.monitor.rect.top)
    }
}

/// The open overlay, on every monitor. Dropping it closes it.
pub struct Overlay {
    screens: Vec<Screen>,
    backdrop: Backdrop,
    dim: f32,
    events: Rc<RefCell<Vec<OverlayEvent>>>,
    /// Choosing a region: a selection window on every monitor, and what each got (its
    /// monitor's index with it).
    selection: Option<Regions>,
    selection_events: Rc<RefCell<Vec<(usize, SelectionInput)>>>,
    /// The exe that serves clean grabs (`--grabber`), and the helper while it runs.
    grabber_exe: Option<std::path::PathBuf>,
    grabber: Option<Grabber>,
    /// The window that had the focus before the layer took it: it gets it back on close.
    previous_focus: Option<HWND>,
    look: Look,
    guides: Guides,
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // Before the layer's windows go, while this process is still in the foreground.
        if let Some(h) = self.previous_focus.take() {
            let given = window::give_focus_back(h);
            tracing::debug!(given, "focus given back");
        }
    }
}

/// How a pointer sample came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// It touched down (or the mouse's button went down).
    Down,
    /// It moved, touching or hovering.
    Move,
    /// It lifted.
    Up,
    /// It left the layer (hovering out of range, or out of the window).
    Leave,
}

/// Posted to an ink layer: a pointer sample, as [`crate::win32::pointer::pack`] packs it, with
/// the phase in `wparam`'s bits 14-15 (0 down, 1 move, 2 up, 3 leave) and Shift in bit 16. The
/// same path as `WM_POINTER*` after the sample is read: how checks draw without global input.
pub const WM_OVERMARK_POINTER: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x41;
/// Posted to an ink layer: a key, `wparam` its virtual-key code, `lparam` bit 0 Ctrl, 1 Shift,
/// 2 Alt, 3 released. (A posted `WM_KEYDOWN` can't say which modifiers are down.)
pub const WM_OVERMARK_KEY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x42;
/// Posted to an ink layer: the mouse's right button went down, as [`OverlayEvent::RightClick`]
/// (no parameters). Windows can't make a real one without global input.
pub const WM_OVERMARK_RIGHT_CLICK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x43;

/// Whether layer event `e` still goes to the drawing while a region is chosen. The selection
/// covers every monitor, but whatever reaches an ink layer meanwhile (a window over ours, a
/// monitor added, a race as the selection opens) never draws, erases, undoes or changes the
/// tool: no pointer sample but a lift, no right-click, no wheel. Keys and the rest go on as
/// usual. A lift only lets go of a contact: one still down as the selection began was ended
/// where it was then (the app's drawing ends it), so the lift adds nothing to it.
pub fn reaches_ink_while_choosing(e: &OverlayEvent) -> bool {
    match e {
        OverlayEvent::Pointer { phase, .. } => matches!(phase, Phase::Up | Phase::Leave),
        OverlayEvent::RightClick { .. } | OverlayEvent::Wheel { .. } => false,
        _ => true,
    }
}

fn phase_bits(p: Phase) -> usize {
    match p {
        Phase::Down => 0,
        Phase::Move => 1,
        Phase::Up => 2,
        Phase::Leave => 3,
    }
}

/// The `wparam` and `lparam` of a [`WM_OVERMARK_POINTER`].
pub fn pointer_message(
    s: &crate::win32::pointer::Sample,
    phase: Phase,
    shift: bool,
) -> (usize, isize) {
    let (wp, lp) = crate::win32::pointer::pack(s);
    (wp | phase_bits(phase) << 14 | usize::from(shift) << 16, lp)
}

/// The `wparam` and `lparam` of a [`WM_OVERMARK_KEY`].
pub fn key_message(vk: u32, ctrl: bool, shift: bool, alt: bool, up: bool) -> (usize, isize) {
    (
        vk as usize,
        isize::from(ctrl) | isize::from(shift) << 1 | isize::from(alt) << 2 | isize::from(up) << 3,
    )
}

fn key_event(vk: u32, ctrl: bool, shift: bool, alt: bool) -> OverlayEvent {
    match Command::for_key(vk, ctrl, shift, alt) {
        Some(c) => OverlayEvent::Command(c),
        None => OverlayEvent::Key {
            vk,
            ctrl,
            shift,
            alt,
        },
    }
}

/// The characters one `WM_CHAR` code unit completes: a character of its own, or with the high
/// surrogate before it (kept in `high` until its low one comes), a character beyond U+FFFF. A
/// surrogate without its other half is U+FFFD, the replacement character, so nothing typed
/// vanishes silently.
pub fn utf16_char(high: &mut Option<u16>, unit: u16) -> [Option<char>; 2] {
    const REPLACEMENT: char = char::REPLACEMENT_CHARACTER;
    match unit {
        0xD800..=0xDBFF => [high.replace(unit).map(|_| REPLACEMENT), None],
        0xDC00..=0xDFFF => match high.take() {
            Some(h) => [
                char::decode_utf16([h, unit]).next().and_then(Result::ok),
                None,
            ],
            None => [Some(REPLACEMENT), None],
        },
        _ => [
            high.take().map(|_| REPLACEMENT),
            char::from_u32(u32::from(unit)),
        ],
    }
}

fn ink_handler(events: Rc<RefCell<Vec<OverlayEvent>>>, screen: usize) -> Handler {
    use crate::win32::pointer;
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_ACTIVATEAPP, WM_CHAR, WM_KEYUP, WM_MOUSEWHEEL, WM_POINTERDOWN, WM_POINTERLEAVE,
        WM_POINTERUP, WM_POINTERUPDATE, WM_POINTERWHEEL, WM_SYSKEYUP,
    };
    // A character beyond U+FFFF comes as two WM_CHARs, its UTF-16 surrogates.
    let surrogate = std::cell::Cell::new(None::<u16>);
    Box::new(move |_, msg, wp, lp| match msg {
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let phase = match msg {
                WM_POINTERDOWN => Phase::Down,
                WM_POINTERUP => Phase::Up,
                _ => Phase::Move,
            };
            let (_, shift, _) = window::modifiers();
            let samples = pointer::samples(wp);
            let right = phase != Phase::Up && pointer::right_button_down(wp);
            let n = samples.len();
            let mut e = events.borrow_mut();
            for (k, sample) in samples.into_iter().enumerate() {
                // A down or up message's coalesced history before its last sample is movement.
                let phase = if k + 1 == n { phase } else { Phase::Move };
                e.push(OverlayEvent::Pointer {
                    screen,
                    phase,
                    sample,
                    shift,
                });
            }
            if right {
                e.push(OverlayEvent::RightClick { screen });
            }
            // Handled, so Windows makes no mouse messages from it (and no context menu).
            Some(LRESULT(0))
        }
        WM_OVERMARK_RIGHT_CLICK => {
            events
                .borrow_mut()
                .push(OverlayEvent::RightClick { screen });
            Some(LRESULT(0))
        }
        WM_POINTERLEAVE => {
            events.borrow_mut().push(OverlayEvent::Pointer {
                screen,
                phase: Phase::Leave,
                sample: pointer::Sample {
                    // The pointer that left (wparam's low word).
                    id: (wp.0 & 0xFFFF) as u32,
                    ..pointer::unpack(0, 0, clock::now_100ns())
                },
                shift: false,
            });
            None
        }
        // The wheel comes as a pointer message with the mouse in pointer mode, or as the
        // mouse's own where it doesn't (a precision touchpad's, say): the delta is in both's
        // high word.
        WM_POINTERWHEEL | WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            events
                .borrow_mut()
                .push(OverlayEvent::Wheel { screen, delta });
            Some(LRESULT(0))
        }
        WM_OVERMARK_POINTER => {
            let phase = match (wp.0 >> 14) & 3 {
                0 => Phase::Down,
                1 => Phase::Move,
                2 => Phase::Up,
                _ => Phase::Leave,
            };
            events.borrow_mut().push(OverlayEvent::Pointer {
                screen,
                phase,
                sample: pointer::unpack(wp.0 & 0x3FFF, lp.0, clock::now_100ns()),
                shift: wp.0 & (1 << 16) != 0,
            });
            Some(LRESULT(0))
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let (ctrl, shift, alt) = window::modifiers();
            events
                .borrow_mut()
                .push(key_event(wp.0 as u32, ctrl, shift, alt));
            Some(LRESULT(0))
        }
        WM_KEYUP | WM_SYSKEYUP => {
            events.borrow_mut().push(OverlayEvent::KeyUp(wp.0 as u32));
            Some(LRESULT(0))
        }
        // The keyboard leaving the app or coming back while the layer is open, for the log:
        // keys reach the layer only while it's in the foreground, and it can lose that after
        // it opens. Every top-level window of ours gets this, so only the first screen's logs.
        WM_ACTIVATEAPP if screen == 0 => {
            log_activation(wp.0 != 0, lp.0 as u32);
            None
        }
        WM_OVERMARK_KEY => {
            let bit = |n: u32| lp.0 & (1 << n) != 0;
            let vk = wp.0 as u32;
            events.borrow_mut().push(if bit(3) {
                OverlayEvent::KeyUp(vk)
            } else {
                key_event(vk, bit(0), bit(1), bit(2))
            });
            Some(LRESULT(0))
        }
        WM_CHAR => {
            let mut high = surrogate.get();
            let chars = utf16_char(&mut high, wp.0 as u16);
            surrogate.set(high);
            let mut e = events.borrow_mut();
            e.extend(chars.into_iter().flatten().map(OverlayEvent::Char));
            Some(LRESULT(0))
        }
        WM_OVERMARK_COMMAND => {
            if let Some(c) = Command::from_wparam(wp.0) {
                events.borrow_mut().push(OverlayEvent::Command(c));
            }
            Some(LRESULT(0))
        }
        _ => None,
    })
}

fn no_erase() -> Handler {
    Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1)))
}

fn quiet() -> Options {
    Options {
        click_through: true,
        no_activate: true,
        ..Default::default()
    }
}

/// The veil's colour and opacity over a backdrop; `None` when it has nothing to show.
fn veil_look(backdrop: Backdrop, dim: f32) -> Option<((u8, u8, u8), u8)> {
    let dim = dim.clamp(0.0, 1.0);
    match backdrop {
        Backdrop::Live if dim <= 0.0 => None,
        Backdrop::Live => Some(((0, 0, 0), (dim * 255.0).round() as u8)),
        Backdrop::Frozen => Some((
            FROZEN_TINT,
            (dim.max(FROZEN_TINT_MIN) * 255.0).round() as u8,
        )),
        // The boards hide the screen: nothing to dim.
        Backdrop::Whiteboard | Backdrop::Blackboard => None,
    }
}

/// A backdrop layer over `monitor` showing `fill` (opaque), not shown yet.
fn backdrop_layer(monitor: &Monitor, fill: Fill) -> windows::core::Result<Layer> {
    let mut layer = Layer::new(monitor.rect, quiet(), Box::new(|_, _, _, _| None))?;
    fill_layer(&mut layer, monitor, fill)?;
    Ok(layer)
}

/// Fills a backdrop layer's pixels with `fill` (opaque) and shows them.
fn fill_layer(layer: &mut Layer, monitor: &Monitor, fill: Fill) -> windows::core::Result<()> {
    let px = layer.surface.pixels();
    match fill {
        Fill::Frame(image) => {
            if px.len() == image.bgra.len() {
                px.copy_from_slice(&image.bgra);
                for p in px.as_chunks_mut::<4>().0 {
                    p[3] = 255;
                }
            }
        }
        Fill::Solid(c) => {
            for p in px.as_chunks_mut::<4>().0 {
                *p = [c.b, c.g, c.r, 255];
            }
        }
    }
    layer.present((monitor.rect.left, monitor.rect.top))
}

/// Shows `fill` in `s`'s backdrop layer (made if there's none), under the dim.
fn show_backdrop(s: &mut Screen, fill: Fill) -> windows::core::Result<()> {
    match s.backdrop.as_mut() {
        Some(layer) => fill_layer(layer, &s.monitor, fill)?,
        None => s.backdrop = Some(backdrop_layer(&s.monitor, fill)?),
    }
    if let Some(layer) = &s.backdrop {
        layer.window.show(false);
        layer.window.place_below(s.veil.hwnd());
    }
    Ok(())
}

/// A frame of a solid backdrop's colour, the monitor's size: what a save starts from.
fn solid_frame(rect: Rect, c: crate::core::colour::Colour) -> Frame {
    let pixels = (rect.width().max(0) * rect.height().max(0)) as usize;
    Frame {
        width: rect.width(),
        height: rect.height(),
        bgra: [c.b, c.g, c.r, 255].repeat(pixels),
    }
}

/// Logs whether the layer has the keyboard as it opens (`took`: what `SetForegroundWindow`
/// said; then the window really in the foreground, and the one it was taken from). Keys reach the layer only while one of our windows is in the foreground:
/// when Windows refuses (its foreground lock), they go on to the app that had it, often a
/// full-screen one such as a remote-desktop client, until the user clicks the layer.
fn log_focus(took: bool, previous: Option<HWND>) {
    let now = window::foreground();
    let ours = now.is_some_and(window::is_ours);
    let (foreground, before) = (window::describe(now), window::describe(previous));
    if ours {
        tracing::info!(took, foreground, before, "the layer has the keyboard");
    } else {
        tracing::warn!(
            took,
            foreground,
            before,
            "the layer didn't get the keyboard: keys go to the foreground window until the layer is clicked"
        );
    }
}

/// Logs the keyboard coming to this app (`active`) or leaving it while the layer is open, with
/// the window it went to (`thread` is the other app's thread, as `WM_ACTIVATEAPP` gives it).
fn log_activation(active: bool, thread: u32) {
    let foreground = window::describe(window::foreground());
    if active {
        tracing::info!(foreground, "the keyboard came to the layer");
    } else {
        tracing::info!(foreground, thread, "the keyboard left the layer");
    }
}

impl Overlay {
    /// Opens the overlay on every monitor over `backdrop`, with the dim at `dim` (0 to 0.8).
    /// `asked_100ns` is when it was asked for (a hotkey's arrival), for the timing. Frozen grabs
    /// the screen first, before any window of ours shows: what was there at the press.
    /// `grabber_exe` serves later clean grabs ([`crate::grabber`]); without it, they fall back.
    pub fn open(
        backdrop: Backdrop,
        dim: f32,
        asked_100ns: i64,
        grabber_exe: Option<&std::path::Path>,
        look: Look,
        guides: Guides,
    ) -> windows::core::Result<(Overlay, OpenTiming)> {
        let since = |t: i64| (clock::now_100ns() - t) as f64 / 10_000.0;
        let previous_focus = window::foreground();
        let monitors = dpi::monitors();
        let mut timing = OpenTiming::default();
        let solid = match backdrop.fill(&look.backdrops) {
            crate::core::backdrop::BackdropFill::Solid(c) => Some(c),
            _ => None,
        };
        let images: Vec<Option<Frame>> = match backdrop {
            Backdrop::Frozen => monitors
                .iter()
                .map(|m| capture::gdi_screen(m.rect))
                .collect(),
            _ => monitors.iter().map(|_| None).collect(),
        };
        timing.grab_ms = since(asked_100ns);
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut screens = Vec::with_capacity(monitors.len());
        for (m, image) in monitors.into_iter().zip(images) {
            let layer = match (&image, solid) {
                (Some(i), _) => Some(backdrop_layer(&m, Fill::Frame(i))?),
                (None, Some(c)) => Some(backdrop_layer(&m, Fill::Solid(c))?),
                (None, None) => None,
            };
            let frozen = layer.as_ref().and(image);
            let veil = Window::new(m.rect, quiet(), no_erase())?;
            affinity::set_excluded(veil.hwnd(), true)?;
            let index = screens.len();
            let mut ink = Layer::new(
                m.rect,
                Options::default(),
                ink_handler(events.clone(), index),
            )?;
            let pixmap = tiny_skia::Pixmap::new(m.rect.width() as u32, m.rect.height() as u32)
                .ok_or_else(|| {
                    windows::core::Error::new(
                        windows::Win32::Foundation::E_OUTOFMEMORY,
                        "no ink pixmap",
                    )
                })?;
            for p in ink.surface.pixels().as_chunks_mut::<4>().0 {
                *p = EMPTY;
            }
            ink.present((m.rect.left, m.rect.top))?;
            screens.push(Screen {
                monitor: m,
                backdrop: layer,
                frozen,
                veil,
                ink,
                pixmap,
                caret: None,
                caret_colour: [0, 0, 0, 255],
                frozen_rgba: Default::default(),
                over_frame: None,
                guides: None,
                guide_pixels: Default::default(),
            });
        }
        // A grab that failed leaves that backdrop out: open Live rather than half Frozen.
        let backdrop = if backdrop != Backdrop::Live && screens.iter().any(|s| s.backdrop.is_none())
        {
            tracing::warn!("a monitor couldn't be grabbed: opening Live");
            for s in &mut screens {
                s.backdrop = None;
                s.frozen = None;
            }
            Backdrop::Live
        } else {
            backdrop
        };
        timing.layers_ms = since(asked_100ns);
        let mut overlay = Overlay {
            screens,
            backdrop,
            dim,
            events,
            selection: None,
            selection_events: Rc::new(RefCell::new(Vec::new())),
            grabber_exe: grabber_exe.map(std::path::Path::to_path_buf),
            grabber: None,
            previous_focus,
            look,
            guides,
        };
        for s in &overlay.screens {
            if let Some(layer) = &s.backdrop {
                layer.window.show(false);
            }
        }
        overlay.apply_guides();
        overlay.apply_veil();
        for s in &overlay.screens {
            s.ink.window.show(s.monitor.primary);
        }
        if let Some(primary) = overlay.screens.iter().find(|s| s.monitor.primary) {
            let took = primary.ink.window.focus();
            log_focus(took, overlay.previous_focus);
        }
        // Not waiting for the compositor here: the app shows the toolbar first, so both come
        // in the same frame, then waits once ([`Overlay::shown`]).
        timing.visible_ms = since(asked_100ns);
        Ok((overlay, timing))
    }

    /// Waits for the compositor to show what's been made (the layers, and the toolbar the app
    /// adds), and returns the milliseconds since `asked_100ns`: the hotkey-to-visible time.
    pub fn shown(&self, asked_100ns: i64) -> f64 {
        window::wait_for_composition();
        (clock::now_100ns() - asked_100ns) as f64 / 10_000.0
    }

    pub fn backdrop(&self) -> Backdrop {
        self.backdrop
    }

    /// The dim's level now (0 to 0.8).
    pub fn dim(&self) -> f32 {
        self.dim
    }

    /// Changes the dim while the layer is open (Settings saved a new level).
    pub fn set_dim(&mut self, dim: f32) {
        let dim = dim.clamp(0.0, 0.8);
        if dim != self.dim {
            self.dim = dim;
            self.apply_veil();
        }
    }

    /// Shows the veil for the current backdrop, in its place in the z-order.
    fn apply_veil(&mut self) {
        let look = veil_look(self.backdrop, self.dim);
        for s in &self.screens {
            match look {
                None => s.veil.hide(),
                Some(((r, g, b), alpha)) => {
                    s.veil.fill_gdi(r, g, b);
                    s.veil.set_opacity(alpha);
                    s.veil.show(false);
                    s.veil.fill_gdi(r, g, b);
                    // Over the backdrop layer, under the guides and the ink.
                    let above = s
                        .guides
                        .as_ref()
                        .map_or(s.ink.window.hwnd(), |g| g.window.hwnd());
                    s.veil.place_below(above);
                }
            }
        }
    }

    /// Our windows, to leave out of a clean grab.
    fn windows(&self) -> Vec<HWND> {
        let mut out = Vec::new();
        for s in &self.screens {
            out.push(s.ink.window.hwnd());
            out.push(s.veil.hwnd());
            if let Some(layer) = &s.backdrop {
                out.push(layer.window.hwnd());
            }
            if let Some(layer) = &s.guides {
                out.push(layer.window.hwnd());
            }
        }
        out
    }

    /// Starts what a later freeze needs (the grabber, with a magnifier per monitor: about
    /// 100 ms in all, in its own process), so F answers quickly. The app calls it once the
    /// overlay shows.
    pub fn prepare(&mut self) {
        if self.grabber.is_some() {
            return;
        }
        let Some(exe) = self.grabber_exe.clone() else {
            return;
        };
        let ours = self.windows();
        match Grabber::start(&exe) {
            Ok(mut g) => {
                for s in &self.screens {
                    if let Err(error) = g.prepare(s.monitor.rect, &ours) {
                        tracing::warn!(%error, monitor = %s.monitor.name, "the grabber couldn't prepare");
                    }
                }
                self.grabber = Some(g);
            }
            Err(error) => tracing::warn!(%error, "the grabber didn't start"),
        }
    }

    /// The monitor `i` now, without our windows. The grabber's magnifier leaves them out while
    /// they stay on screen; without it, the ink layer is excluded from capture for a GDI grab
    /// (which a share can see blink: docs/ARCHITECTURE.md, Saving).
    fn clean_grab(&mut self, i: usize) -> Option<Frame> {
        self.prepare();
        let ours = self.windows();
        let rect = self.screens[i].monitor.rect;
        if let Some(g) = self.grabber.as_mut() {
            match g.grab(rect, &ours) {
                Ok(Some(f)) if f.width == rect.width() && f.height == rect.height() => {
                    return Some(f);
                }
                Ok(_) => tracing::warn!("the grabber gave no image"),
                Err(error) => {
                    tracing::warn!(%error, "the grabber failed");
                    self.grabber = None;
                }
            }
        }
        let s = &self.screens[i];
        tracing::warn!(monitor = %s.monitor.name, "no clean grab: excluding the ink layer for a GDI grab");
        // Every layer of ours a capture can see: the ink, the guides, and the backdrop (a
        // board, or the frozen frame), so the grab is the screen under them.
        let visible: Vec<HWND> = std::iter::once(s.ink.window.hwnd())
            .chain(s.guides.as_ref().map(|g| g.window.hwnd()))
            .chain(s.backdrop.as_ref().map(|b| b.window.hwnd()))
            .collect();
        for &h in &visible {
            let _ = affinity::set_excluded(h, true);
        }
        window::wait_for_composition();
        let f = capture::gdi_screen(s.monitor.rect);
        let _ = affinity::set_excluded(s.ink.window.hwnd(), false);
        if let Some(b) = &s.backdrop {
            let _ = affinity::set_excluded(b.window.hwnd(), false);
        }
        if let Some(g) = &s.guides {
            let _ = affinity::set_excluded(g.window.hwnd(), !self.look.guides.show_on_shares);
        }
        f
    }

    /// Switches the backdrop, keeping the strokes. Frozen from Live grabs the screen as it is
    /// now, without our windows; from a whiteboard or blackboard it shows the frame kept from
    /// before (the same moment), grabbing only if there's none. The boards keep the frozen
    /// frame until the layer closes; Live lets it go. Returns how long it took, in milliseconds.
    pub fn set_backdrop(&mut self, backdrop: Backdrop) -> windows::core::Result<f64> {
        use crate::core::backdrop::BackdropFill;
        let t = clock::now_secs();
        if backdrop == self.backdrop {
            return Ok(0.0);
        }
        match backdrop.fill(&self.look.backdrops) {
            BackdropFill::Frame => {
                let kept = self.backdrop != Backdrop::Live
                    && self.screens.iter().all(|s| s.frozen.is_some());
                if !kept {
                    let mut images = Vec::new();
                    for i in 0..self.screens.len() {
                        images.push(self.clean_grab(i));
                    }
                    if images.iter().any(Option::is_none) {
                        tracing::warn!(
                            "a monitor couldn't be grabbed: the backdrop stays as it was"
                        );
                        return Ok((clock::now_secs() - t) * 1000.0);
                    }
                    for (s, image) in self.screens.iter_mut().zip(images) {
                        s.frozen = image;
                        s.frozen_rgba.reset();
                    }
                }
                for s in &mut self.screens {
                    let Some(frame) = s.frozen.take() else {
                        continue;
                    };
                    let shown = show_backdrop(s, Fill::Frame(&frame));
                    s.frozen = Some(frame);
                    shown?;
                }
                // The new backdrop windows are ours too: each grab sends the list again.
            }
            BackdropFill::Solid(c) => {
                for s in &mut self.screens {
                    show_backdrop(s, Fill::Solid(c))?;
                }
            }
            BackdropFill::Screen => {
                for s in &mut self.screens {
                    s.backdrop = None;
                    s.frozen = None;
                    s.frozen_rgba.reset();
                    s.over_frame = None;
                }
            }
        }
        self.backdrop = backdrop;
        // The guides' colour suits the backdrop.
        self.apply_guides();
        self.apply_veil();
        Ok((clock::now_secs() - t) * 1000.0)
    }

    /// Settings changed how the backdrops and guides look while the layer is open: a
    /// board shows its new colour, and the guides are drawn again with the new spacing and
    /// shown or kept from shares as now set. The strokes stay.
    pub fn set_look(&mut self, look: Look) -> windows::core::Result<()> {
        use crate::core::backdrop::BackdropFill;
        if look == self.look {
            return Ok(());
        }
        let before = self.backdrop.fill(&self.look.backdrops);
        let shares_changed = look.guides.show_on_shares != self.look.guides.show_on_shares;
        self.look = look;
        let now = self.backdrop.fill(&self.look.backdrops);
        if let BackdropFill::Solid(c) = now
            && now != before
        {
            for s in &mut self.screens {
                show_backdrop(s, Fill::Solid(c))?;
            }
        }
        if shares_changed {
            // A fresh layer, so whether shares see it is set again.
            for s in &mut self.screens {
                s.guides = None;
            }
        }
        self.apply_guides();
        self.apply_veil();
        Ok(())
    }

    /// The guides over the backdrop.
    pub fn guides(&self) -> Guides {
        self.guides
    }

    /// Shows `guides` over the backdrop on every monitor (Off: none), keeping the strokes.
    pub fn set_guides(&mut self, guides: Guides) {
        self.guides = guides;
        self.apply_guides();
        self.apply_veil();
    }

    /// The guides' style over the current backdrop on monitor `i`, and the view (its scale:
    /// the spacing is in pixels at 100 %).
    fn guide_style(&self, i: usize) -> Option<(GuideStyle, View)> {
        let s = self.screens.get(i)?;
        let style = GuideStyle::new(&self.look.guides, &self.look.backdrops, self.backdrop);
        Some((
            style,
            View {
                scale: s.monitor.scale(),
            },
        ))
    }

    /// What a save of monitor `i` draws its guides with, if saves include them (Settings'
    /// `guides.include_in_saves`) and there are any.
    pub fn save_guides(&self, i: usize) -> Option<(Guides, GuideStyle, View)> {
        if self.guides == Guides::Off || !self.look.guides.include_in_saves {
            return None;
        }
        let (style, view) = self.guide_style(i)?;
        Some((self.guides, style, view))
    }

    /// Makes each monitor's guides layer match the guides, the backdrop and the settings: made,
    /// redrawn only when something about them changed (core's `GuideLayer`), or dropped.
    fn apply_guides(&mut self) {
        let show_on_shares = self.look.guides.show_on_shares;
        for i in 0..self.screens.len() {
            let Some((style, view)) = self.guide_style(i) else {
                continue;
            };
            let guides = self.guides;
            let s = &mut self.screens[i];
            if guides == Guides::Off {
                s.guides = None;
                continue;
            }
            let (w, h) = (s.pixmap.width(), s.pixmap.height());
            let fresh = !s.guide_pixels.is_for(w, h, guides, &style, view) || s.guides.is_none();
            if !fresh {
                continue;
            }
            if s.guides.is_none() {
                match Layer::new(s.monitor.rect, quiet(), Box::new(|_, _, _, _| None)) {
                    Ok(layer) => s.guides = Some(layer),
                    Err(error) => {
                        tracing::warn!(%error, "couldn't make the guides' layer");
                        continue;
                    }
                }
            }
            let origin = s.origin();
            let ink = s.ink.window.hwnd();
            let Some(pixels) = s.guide_pixels.layer(w, h, guides, &style, view) else {
                continue;
            };
            let Some(layer) = s.guides.as_mut() else {
                continue;
            };
            let dst = layer.surface.pixels();
            if dst.len() == pixels.data().len() {
                for (d, p) in dst
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(pixels.data().as_chunks::<4>().0)
                {
                    *d = [p[2], p[1], p[0], p[3]];
                }
            }
            if let Err(error) = affinity::set_excluded(layer.window.hwnd(), !show_on_shares) {
                tracing::warn!(%error, "couldn't set whether shares see the guides");
            }
            if let Err(error) = layer.present(origin) {
                tracing::warn!(%error, "couldn't show the guides");
            }
            layer.window.show(false);
            layer.window.place_below(ink);
        }
    }

    /// The keys and other events since the last call. Runs the region selection on the way:
    /// its drag is followed here, and only its outcome comes out.
    pub fn take_events(&mut self) -> Vec<OverlayEvent> {
        let events = std::mem::take(&mut *self.events.borrow_mut());
        let inputs = std::mem::take(&mut *self.selection_events.borrow_mut());
        let sifted = sift(
            events,
            inputs,
            self.selection.as_mut().map(|r| &mut r.chooser),
        );
        if let Some(regions) = self.selection.as_mut() {
            for i in sifted.redraw {
                regions.redraw(i);
            }
        }
        if sifted.ended {
            self.selection = None;
            if let Some(s) = self.screens.iter().find(|s| s.monitor.primary) {
                s.ink.window.focus();
            }
        }
        sifted.out
    }

    /// The monitor the screen point (`x`, `y`) is on, by index; the primary's if it's on none.
    pub fn screen_at(&self, x: i32, y: i32) -> usize {
        self.screens
            .iter()
            .position(|s| {
                let r = s.monitor.rect;
                x >= r.left && x < r.right && y >= r.top && y < r.bottom
            })
            .or_else(|| self.screens.iter().position(|s| s.monitor.primary))
            .unwrap_or(0)
    }

    /// The monitor `i` covers.
    pub fn monitor(&self, i: usize) -> Option<&Monitor> {
        self.screens.get(i).map(|s| &s.monitor)
    }

    /// The primary monitor's layer: where a test drawing (`--test-drawing`) is put. Each
    /// monitor has a drawing of its own.
    pub fn drawing_screen(&self) -> usize {
        self.screens
            .iter()
            .position(|s| s.monitor.primary)
            .unwrap_or(0)
    }

    /// The clean image of monitor `i` for a capture: in Frozen, the frozen frame; in Live, a
    /// grab without our windows, taken now. Never the composited overlay.
    pub fn capture_frame(&mut self, i: usize) -> Option<Frame> {
        if self.backdrop == Backdrop::Frozen
            && let Some(f) = self.screens.get(i).and_then(|s| s.frozen.clone())
        {
            return Some(f);
        }
        if let crate::core::backdrop::BackdropFill::Solid(c) =
            self.backdrop.fill(&self.look.backdrops)
        {
            return self.screens.get(i).map(|s| solid_frame(s.monitor.rect, c));
        }
        if i < self.screens.len() {
            self.clean_grab(i)
        } else {
            None
        }
    }

    /// Draws `drawing` on monitor `i`'s ink layer in full.
    pub fn show_drawing(&mut self, i: usize, drawing: &crate::core::drawing::Drawing) {
        let options = crate::core::render::RenderOptions::default();
        self.redraw(i, drawing, &options, None);
    }

    /// Redraws monitor `i`'s ink layer where `area` (drawing coordinates; `None`: all of it)
    /// changed, exactly as a full render would (core's `render_region_with`), and shows only
    /// that rectangle. The layer is transparent: a highlighter is a tint here
    /// (`options.highlight`), its true darken being a capture's (docs/ARCHITECTURE.md, Tools).
    pub fn redraw(
        &mut self,
        i: usize,
        drawing: &crate::core::drawing::Drawing,
        options: &crate::core::render::RenderOptions,
        area: Option<kurbo::Rect>,
    ) {
        use crate::core::render::{self, Background};
        let frozen = self.backdrop == Backdrop::Frozen;
        // A blur pixelates the guides with the frame where the layer it's on shows them: on
        // screen and to a share when shares see the guides, as a save does when saves
        // include them.
        let guides = (self.look.guides.show_on_shares && self.guides != Guides::Off)
            .then(|| {
                self.guide_style(i)
                    .map(|(style, view)| (self.guides, style, view))
            })
            .flatten();
        let Some(s) = self.screens.get_mut(i) else {
            return;
        };
        let (w, h) = (s.pixmap.width(), s.pixmap.height());
        let rect = match area {
            None => {
                render::render_with(
                    drawing,
                    &mut s.pixmap.as_mut(),
                    options,
                    Background::Transparent,
                );
                Some(Rect::new(0, 0, w as i32, h as i32))
            }
            Some(a) => render::dirty_rect(a, options.view, w, h).and_then(|ir| {
                render::render_region_with(
                    drawing,
                    &mut s.pixmap.as_mut(),
                    options,
                    Background::Transparent,
                    ir,
                )
                .map(|r| {
                    Rect::new(
                        r.x(),
                        r.y(),
                        r.x() + r.width() as i32,
                        r.y() + r.height() as i32,
                    )
                })
            }),
        };
        if let Some(r) = rect {
            if frozen {
                s.blur_pass(drawing, options, r, guides.as_ref());
            }
            s.blit(r);
            let shown = match s.caret {
                Some(c)
                    if c.left < r.right
                        && r.left < c.right
                        && c.top < r.bottom
                        && r.top < c.bottom =>
                {
                    s.paint_caret();
                    r.union(&c)
                }
                _ => r,
            };
            s.present(shown);
        }
    }

    /// Shows the text caret on monitor `i` at `caret` (drawing coordinates; `None`: no caret),
    /// in `colour` (RGBA).
    pub fn set_caret(&mut self, i: usize, caret: Option<kurbo::Rect>, colour: [u8; 4]) {
        let Some(s) = self.screens.get_mut(i) else {
            return;
        };
        let (w, h) = (s.pixmap.width() as i32, s.pixmap.height() as i32);
        let new = caret
            .map(|c| {
                Rect::new(
                    c.x0.floor() as i32,
                    c.y0.floor() as i32,
                    (c.x1.ceil() as i32).max(c.x0.floor() as i32 + 2),
                    c.y1.ceil() as i32,
                )
                .clamp_to(w, h)
            })
            .filter(|r| !r.is_empty());
        let old = s.caret;
        if old == new && s.caret_colour == colour {
            return;
        }
        s.caret = new;
        s.caret_colour = colour;
        let mut shown = Rect::default();
        if let Some(o) = old {
            s.blit(o);
            shown = shown.union(&o);
        }
        if let Some(n) = new {
            s.paint_caret();
            shown = shown.union(&n);
        }
        if !shown.is_empty() {
            s.present(shown);
        }
    }

    /// The monitors' rectangles, by index.
    pub fn screen_rects(&self) -> Vec<Rect> {
        self.screens.iter().map(|s| s.monitor.rect).collect()
    }

    /// The number of monitors.
    pub fn screen_count(&self) -> usize {
        self.screens.len()
    }

    /// Starts choosing a region on every monitor: `images[i]` is what monitor `i` will save
    /// (its frame with its drawing), which its selection shows still while the screen goes on
    /// underneath. The drag can start on any monitor, and the region stays on the monitor it
    /// started on. `focus` (the monitor under the cursor) takes the keyboard, for Esc. The
    /// result comes as [`OverlayEvent::RegionChosen`] or [`OverlayEvent::RegionCancelled`].
    pub fn begin_region(
        &mut self,
        focus: usize,
        images: &[crate::save::Image],
    ) -> windows::core::Result<()> {
        let mut windows = Vec::with_capacity(self.screens.len());
        for (i, (s, image)) in self.screens.iter().zip(images).enumerate() {
            windows.push(Selection::new(
                i,
                &s.monitor,
                image,
                self.selection_events.clone(),
            )?);
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
            w.layer.window.focus();
        }
        let sizes = windows.iter().map(|w| w.size()).collect();
        self.selection = Some(Regions {
            windows,
            chooser: Chooser::new(sizes),
        });
        Ok(())
    }

    /// Whether a region is being chosen.
    pub fn is_choosing_region(&self) -> bool {
        self.selection.is_some()
    }

    /// Monitor `i`'s selection window, while a region is being chosen (for checks to drive).
    pub fn selection_window(&self, i: usize) -> Option<HWND> {
        self.selection
            .as_ref()
            .and_then(|r| r.windows.iter().find(|w| w.screen == i))
            .map(|w| w.layer.window.hwnd())
    }

    /// The primary monitor's ink layer (for tests and checks to post keys to).
    pub fn ink_window(&self) -> Option<HWND> {
        self.screens
            .iter()
            .find(|s| s.monitor.primary)
            .or(self.screens.first())
            .map(|s| s.ink.window.hwnd())
    }

    /// The monitors it covers.
    pub fn monitors(&self) -> Vec<Monitor> {
        self.screens.iter().map(|s| s.monitor.clone()).collect()
    }

    #[allow(dead_code)]
    fn origins(&self) -> Vec<(i32, i32)> {
        self.screens.iter().map(Screen::origin).collect()
    }
}

/// What the selection window got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionInput {
    Down(i32, i32),
    Move(i32, i32),
    Up(i32, i32),
    Escape,
}

/// The rectangle between two points, the far edges included.
fn span(a: (i32, i32), b: (i32, i32)) -> Rect {
    Rect::new(
        a.0.min(b.0),
        a.1.min(b.1),
        a.0.max(b.0) + 1,
        a.1.max(b.1) + 1,
    )
}

/// What [`sift`] made of a turn's events.
#[derive(Debug, Default, PartialEq)]
struct Sifted {
    /// For the app: the layers' events, then the selection's outcome if it ended.
    out: Vec<OverlayEvent>,
    /// The monitors whose selection changed, to be shown again (while it goes on).
    redraw: Vec<usize>,
    /// The selection ended (chosen or cancelled): its windows go.
    ended: bool,
}

/// One turn's events: the layers' `events`, and the selection windows' `inputs` run through
/// `chooser` while a region is chosen. Everything the layers got in that turn arrived while the
/// selection was up (even if it ends in the same turn), so only what
/// [`reaches_ink_while_choosing`] lets through goes on.
fn sift(
    events: Vec<OverlayEvent>,
    inputs: Vec<(usize, SelectionInput)>,
    chooser: Option<&mut Chooser>,
) -> Sifted {
    let Some(chooser) = chooser else {
        return Sifted {
            out: events,
            ..Sifted::default()
        };
    };
    let mut sifted = Sifted {
        out: events
            .into_iter()
            .filter(reaches_ink_while_choosing)
            .collect(),
        ..Sifted::default()
    };
    for (screen, input) in inputs {
        let done = match chooser.input(screen, input) {
            Choice::Going(None) => continue,
            Choice::Going(Some(changed)) => {
                if !sifted.redraw.contains(&changed) {
                    sifted.redraw.push(changed);
                }
                continue;
            }
            Choice::Chosen { screen, rect } => OverlayEvent::RegionChosen { screen, rect },
            Choice::Cancelled => OverlayEvent::RegionCancelled,
        };
        sifted.out.push(done);
        sifted.ended = true;
        sifted.redraw.clear();
        break;
    }
    sifted
}

/// Choosing a region: each monitor's selection window, and the drag they share.
struct Regions {
    windows: Vec<Selection>,
    chooser: Chooser,
}

impl Regions {
    /// Shows monitor `i`'s drag as it is now.
    fn redraw(&mut self, i: usize) {
        let rect = self.chooser.rect(i);
        if let Some(w) = self.windows.iter_mut().find(|w| w.screen == i) {
            w.rect = rect;
            w.redraw();
        }
    }
}

/// What a selection input did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    /// Still choosing; the monitor whose drag changed, to be shown again.
    Going(Option<usize>),
    /// A region on monitor `screen`, in its pixels.
    Chosen { screen: usize, rect: Rect },
    /// Esc on any monitor.
    Cancelled,
}

/// The drag across the monitors' selection windows (each monitor's pixels): one at a time, on
/// the monitor where its button went down, clipped to that monitor.
struct Chooser {
    /// Each monitor's size.
    sizes: Vec<(i32, i32)>,
    /// The drag: its monitor, where it started and the rectangle so far.
    drag: Option<(usize, (i32, i32), Rect)>,
}

impl Chooser {
    fn new(sizes: Vec<(i32, i32)>) -> Chooser {
        Chooser { sizes, drag: None }
    }

    /// Monitor `i`'s rectangle so far (empty while nothing is dragged there).
    fn rect(&self, i: usize) -> Rect {
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
    fn input(&mut self, screen: usize, input: SelectionInput) -> Choice {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The test message is a right-click on its screen's layer; a pointer message Windows
    /// knows nothing of (a pointer id that isn't live) is nothing at all.
    #[test]
    fn a_right_click_comes_from_its_layer() {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::WM_POINTERDOWN;
        let events = Rc::new(RefCell::new(Vec::new()));
        let handler = ink_handler(events.clone(), 2);
        let none = HWND::default();
        assert!(handler(none, WM_OVERMARK_RIGHT_CLICK, WPARAM(0), LPARAM(0)).is_some());
        assert_eq!(*events.borrow(), [OverlayEvent::RightClick { screen: 2 }]);
        events.borrow_mut().clear();
        handler(none, WM_POINTERDOWN, WPARAM(0xBEEF), LPARAM(0));
        assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    }

    fn decode(units: &[u16]) -> String {
        let mut high = None;
        units
            .iter()
            .flat_map(|&u| utf16_char(&mut high, u))
            .flatten()
            .collect()
    }

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

    #[test]
    fn typed_characters_beyond_the_bmp_arrive_whole() {
        let units: Vec<u16> = "A😀B中𝄞".encode_utf16().collect();
        assert_eq!(decode(&units), "A😀B中𝄞");
        // A surrogate without its other half shows as U+FFFD, never vanishing.
        assert_eq!(decode(&[0xD83D, 0x41]), "\u{FFFD}A");
        assert_eq!(decode(&[0xDE00, 0x42]), "\u{FFFD}B");
        assert_eq!(decode(&[0xD83D, 0xD83D, 0xDE00]), "\u{FFFD}😀");
    }

    #[test]
    fn the_veil_follows_the_dim_and_tints_frozen() {
        assert_eq!(veil_look(Backdrop::Live, 0.0), None);
        assert_eq!(veil_look(Backdrop::Live, 0.35), Some(((0, 0, 0), 89)));
        // Frozen always shows its tint, at least FROZEN_TINT_MIN, and the dim's level above it.
        assert_eq!(veil_look(Backdrop::Frozen, 0.0), Some((FROZEN_TINT, 31)));
        assert_eq!(veil_look(Backdrop::Frozen, 0.35), Some((FROZEN_TINT, 89)));
        assert_eq!(veil_look(Backdrop::Live, 2.0), Some(((0, 0, 0), 255)));
    }

    #[test]
    fn capture_keys_map_to_commands() {
        let s = 0x53;
        assert_eq!(Command::for_key(s, true, false, false), Some(Command::Save));
        assert_eq!(
            Command::for_key(s, true, true, false),
            Some(Command::SaveRegion)
        );
        assert_eq!(
            Command::for_key(0x43, true, false, false),
            Some(Command::Copy)
        );
        assert_eq!(
            Command::for_key(s, true, true, true),
            Some(Command::SaveWithoutDrawing)
        );
        // Ctrl+Alt+S is AltGr+S on many keyboards: not a capture.
        assert_eq!(Command::for_key(s, true, false, true), None);
        assert_eq!(Command::for_key(s, false, false, false), None);
        assert_eq!(Command::for_key(0x43, true, true, false), None);
        for c in Command::ALL {
            assert_eq!(Command::from_wparam(c.wparam()), Some(c));
        }
        assert_eq!(Command::from_wparam(99), None);
    }

    use SelectionInput::{Down, Escape, Move, Up};

    /// Two monitors: A (0) at 1920 × 1080 and B (1) at 2560 × 1440.
    fn two_monitors() -> Chooser {
        Chooser::new(vec![(1920, 1080), (2560, 1440)])
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

    fn pointer(screen: usize, phase: Phase) -> OverlayEvent {
        OverlayEvent::Pointer {
            screen,
            phase,
            sample: crate::win32::pointer::Sample {
                id: 1,
                kind: crate::win32::pointer::PointerKind::Mouse,
                x: 0,
                y: 0,
                pressure: None,
                tilt: None,
                eraser: false,
                in_contact: phase != Phase::Up,
                time_100ns: 0,
            },
            shift: false,
        }
    }

    /// A turn whose selection ends (Esc, or the region's lift) still sifts the layers' events
    /// that came in it: they arrived while the selection was up, so no press, move or
    /// right-click from it reaches the app, only the lift, the keys and the outcome.
    #[test]
    fn the_layers_events_from_a_turn_the_selection_ends_in_are_sifted_too() {
        let events = || {
            vec![
                OverlayEvent::RightClick { screen: 0 },
                pointer(1, Phase::Down),
                pointer(1, Phase::Move),
                pointer(1, Phase::Up),
                OverlayEvent::KeyUp(0x10),
            ]
        };
        let kept = vec![pointer(1, Phase::Up), OverlayEvent::KeyUp(0x10)];
        let mut c = two_monitors();
        let sifted = sift(events(), vec![(0, Escape)], Some(&mut c));
        let mut want = kept.clone();
        want.push(OverlayEvent::RegionCancelled);
        assert_eq!(
            sifted,
            Sifted {
                out: want,
                redraw: vec![],
                ended: true
            }
        );
        let mut c = two_monitors();
        let inputs = vec![(1, Down(10, 10)), (1, Move(40, 40)), (1, Up(50, 60))];
        let sifted = sift(events(), inputs, Some(&mut c));
        let mut want = kept.clone();
        want.push(OverlayEvent::RegionChosen {
            screen: 1,
            rect: Rect::new(10, 10, 51, 61),
        });
        assert_eq!((sifted.out, sifted.ended), (want, true));
        // Still choosing: sifted, and the monitor dragged on is shown again, once.
        let mut c = two_monitors();
        let sifted = sift(
            events(),
            vec![(1, Down(10, 10)), (1, Move(40, 40))],
            Some(&mut c),
        );
        assert_eq!(
            sifted,
            Sifted {
                out: kept,
                redraw: vec![1],
                ended: false
            }
        );
        // With no selection, everything goes on as it came.
        let sifted = sift(events(), vec![(0, Escape)], None);
        assert_eq!((sifted.out, sifted.ended), (events(), false));
    }

    /// While a region is chosen, nothing on an ink layer draws, erases, undoes or turns the
    /// tool; a lift still comes (it ends a stroke begun before), and keys go on.
    #[test]
    fn only_lifts_and_keys_reach_the_ink_while_a_region_is_chosen() {
        let sample = crate::win32::pointer::Sample {
            id: 1,
            kind: crate::win32::pointer::PointerKind::Mouse,
            x: 0,
            y: 0,
            pressure: None,
            tilt: None,
            eraser: false,
            in_contact: true,
            time_100ns: 0,
        };
        let pointer = |phase| OverlayEvent::Pointer {
            screen: 1,
            phase,
            sample,
            shift: false,
        };
        assert!(!reaches_ink_while_choosing(&pointer(Phase::Down)));
        assert!(!reaches_ink_while_choosing(&pointer(Phase::Move)));
        assert!(reaches_ink_while_choosing(&pointer(Phase::Up)));
        assert!(reaches_ink_while_choosing(&pointer(Phase::Leave)));
        assert!(!reaches_ink_while_choosing(&OverlayEvent::RightClick {
            screen: 1
        }));
        assert!(!reaches_ink_while_choosing(&OverlayEvent::Wheel {
            screen: 0,
            delta: 120
        }));
        assert!(reaches_ink_while_choosing(&OverlayEvent::KeyUp(0x50)));
    }

    #[test]
    fn a_selection_spans_its_corners_either_way() {
        assert_eq!(span((10, 20), (5, 40)), Rect::new(5, 20, 11, 41));
        assert_eq!(span((3, 3), (3, 3)), Rect::new(3, 3, 4, 4));
    }

    #[test]
    fn f_swaps_live_and_frozen() {
        assert_eq!(freeze_toggled(Backdrop::Live), Backdrop::Frozen);
        assert_eq!(freeze_toggled(Backdrop::Frozen), Backdrop::Live);
        assert_eq!(freeze_toggled(Backdrop::Whiteboard), Backdrop::Frozen);
        assert_eq!(freeze_toggled(Backdrop::Blackboard), Backdrop::Frozen);
    }

    #[test]
    fn the_dim_is_never_over_a_board() {
        assert!(veil_look(Backdrop::Live, 0.35).is_some());
        assert!(
            veil_look(Backdrop::Frozen, 0.0).is_some(),
            "Frozen keeps its tint"
        );
        assert_eq!(veil_look(Backdrop::Whiteboard, 0.35), None);
        assert_eq!(veil_look(Backdrop::Blackboard, 0.8), None);
    }

    #[test]
    fn a_solid_frame_is_the_colour_everywhere() {
        let c = crate::core::colour::Colour::rgb(0x26, 0x2A, 0x2E);
        let f = solid_frame(Rect::new(-10, 0, 10, 5), c);
        assert_eq!((f.width, f.height, f.bgra.len()), (20, 5, 400));
        assert!(
            f.bgra
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0x2E, 0x2A, 0x26, 255])
        );
    }
}
