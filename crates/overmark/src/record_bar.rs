//! Recording a box on the screen: its outline and the control bar on its top edge, the same in
//! both recording modes (while annotating, and screen capture without the drawing layer).
//!
//! - **The bar** ([`show`], drawn by [`Bar`]): before recording, Record, the timer at 0:00 and
//!   ✕ (cancel); while recording, Pause, Stop, the timer and ✕; paused, Resume in Pause's place.
//!   ✕ on a recording asks first, in the bar itself: "Discard this recording?" with Discard and
//!   Keep. Stop saves: there is no separate Save.
//! - **Where it goes:** always on the box's top edge, just above it when it fits on the monitor,
//!   just inside it when it doesn't (`overmark_core::record::bar_origin`).
//! - **Its window** is the toolbar's kind: egui drawn on the CPU into a layered window of
//!   Overmark's own, excluded from capture, never activated, only its frame taking the pointer,
//!   read by screen readers. The outline is `overmark_win::region::Outline`. Neither is ever in
//!   the video.
//! - [`RecordBox`] holds the outline, the bar and the recording, and knows nothing of the
//!   drawing layer: the app turns its [`BarAction`]s into what they do.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Role, Sense, Ui, WidgetInfo, vec2};
use overmark_core::i18n::tr;
use overmark_core::record::{PixelRect, bar_origin, timer_text};
use overmark_win::record::{Recording, Start};
use overmark_win::region::Outline;
use overmark_win::win32::Rect as ScreenRect;
use overmark_win::win32::dpi::Monitor;
use overmark_win::win32::panel::{Panel, PanelEvent};

use crate::toolbar::icons::Glyph;
use crate::toolbar::ui::{BUTTON, button, glyph, icon, panel_frame, separator};
use crate::toolbar::{draw_frame, raster};
use crate::ui::icons::Icon;
use crate::ui::theme;

/// Where a recording is, as the bar shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The box is chosen; nothing recorded yet.
    Ready,
    Recording,
    Paused,
}

/// What the bar shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    /// The time recorded, as the timer shows it (whole seconds).
    pub elapsed: Duration,
    /// ✕ was pressed on a recording: the bar asks whether to throw it away.
    pub confirm_discard: bool,
}

/// What a control on the bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarAction {
    /// Start recording (before), or go on after a pause (Resume).
    Record,
    Pause,
    /// Stop and save.
    Stop,
    /// ✕: cancels the box before recording; on a recording, asks first.
    Discard,
    /// "Discard this recording?" answered Discard: delete it.
    ConfirmDiscard,
    /// Answered Keep: carry on as before.
    Keep,
}

/// The bar's window, in points: room for the bar in its widest state (the question) and its
/// shadow; the rest is transparent, and the pointer goes through it.
const WINDOW: (f32, f32) = (420.0, 56.0);
/// From the window's edge to the bar's frame, in points (room for its shadow).
const INSET: f32 = 6.0;
/// From the box's edge to the bar, in points, beyond the outline.
const GAP: f32 = 4.0;

/// The timer, a recording dot before it while recording: what's on screen and what a screen
/// reader reads (`record-bar-elapsed`).
fn timer(ui: &mut Ui, view: &View) {
    let p = theme::of(ui);
    let text = timer_text(view.elapsed);
    let name = overmark_core::tr!("record-bar-elapsed", time = text.as_str());
    let font = egui::FontId::monospace(14.0);
    let galley = ui.painter().layout_no_wrap(text, font, p.text);
    let width = galley.size().x + 16.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, BUTTON), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &name));
    let dot = match view.phase {
        Phase::Recording => Some(Color32::from_rgb(0xD9, 0x30, 0x25)),
        Phase::Paused => Some(Color32::from_rgb(0xF5, 0xB7, 0x00)),
        Phase::Ready => None,
    };
    if let Some(colour) = dot {
        ui.painter()
            .circle_filled(Pos2::new(rect.left() + 5.0, rect.center().y), 4.0, colour);
    }
    let at = Pos2::new(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, p.text);
}

/// The bar showing `view`, at the top-left of `ctx`'s screen (its window). Returns what was
/// asked for.
pub fn show(ctx: &egui::Context, view: &View) -> Vec<BarAction> {
    let mut actions = Vec::new();
    egui::Area::new(area_id())
        .anchor(Align2::LEFT_TOP, vec2(INSET, INSET))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let p = theme::of(ui);
            panel_frame(p, false)
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme::space::TOOLBAR_GAP;
                        if view.confirm_discard {
                            confirm(ui, &mut actions);
                        } else {
                            controls(ui, view, &mut actions);
                        }
                    });
                });
        });
    actions
}

fn controls(ui: &mut Ui, view: &View, actions: &mut Vec<BarAction>) {
    let (g, name, tip, action) = match view.phase {
        Phase::Ready => (
            Glyph::Record,
            tr("record-bar-record"),
            tr("record-bar-record-tooltip"),
            BarAction::Record,
        ),
        Phase::Recording => (
            Glyph::Pause,
            tr("record-bar-pause"),
            tr("record-bar-pause-tooltip"),
            BarAction::Pause,
        ),
        Phase::Paused => (
            Glyph::Record,
            tr("record-bar-resume"),
            tr("record-bar-resume-tooltip"),
            BarAction::Record,
        ),
    };
    if button(ui, glyph(g), &name, &tip).clicked() {
        actions.push(action);
    }
    ui.add_enabled_ui(view.phase != Phase::Ready, |ui| {
        if button(
            ui,
            glyph(Glyph::Stop),
            &tr("record-bar-stop"),
            &tr("record-bar-stop-tooltip"),
        )
        .clicked()
        {
            actions.push(BarAction::Stop);
        }
    });
    timer(ui, view);
    separator(ui);
    let (name, tip) = if view.phase == Phase::Ready {
        (tr("record-bar-cancel"), tr("record-bar-cancel-tooltip"))
    } else {
        (tr("record-bar-discard"), tr("record-bar-discard-tooltip"))
    };
    if button(ui, icon(Icon::Cross), &name, &tip).clicked() {
        actions.push(BarAction::Discard);
    }
}

fn confirm(ui: &mut Ui, actions: &mut Vec<BarAction>) {
    let p = theme::of(ui);
    let question = tr("record-bar-discard-question");
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.painter()
                .layout_no_wrap(question.clone(), egui::FontId::proportional(14.0), p.text)
                .size()
                .x
                + 8.0,
            BUTTON,
        ),
        Sense::hover(),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &question));
    ui.painter().text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        &question,
        egui::FontId::proportional(14.0),
        p.text,
    );
    if crate::ui::widgets::danger_button_filled(ui, &tr("record-bar-discard-confirm")).clicked() {
        actions.push(BarAction::ConfirmDiscard);
    }
    if crate::ui::widgets::secondary_button(ui, &tr("record-bar-keep")).clicked() {
        actions.push(BarAction::Keep);
    }
}

/// The bar's window: its own egui (a small context of its own, with the shared fonts and
/// styles), drawn into a [`Panel`].
pub struct Bar {
    ctx: egui::Context,
    textures: raster::Textures,
    started: Instant,
    panel: Panel,
    pixels_per_point: f32,
    events: Vec<egui::Event>,
    pointer_down: bool,
    shown: Option<View>,
    repaint_at: Option<Instant>,
    /// The box and its monitor, to place the bar by its frame.
    area: ScreenRect,
    monitor: ScreenRect,
    /// The frame (points) the window was last placed for: a state of another size (the
    /// question is wider) places it again.
    placed_for: Option<egui::Rect>,
}

/// The bar's area in egui (its frame and contents, in points), by which it's placed.
fn area_id() -> egui::Id {
    egui::Id::unique("overmark-record-bar")
}

/// The bar's visible frame as last laid out in `ctx`, in points (from the window's top-left).
fn frame_points(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.memory(|m| m.area_rect(area_id()))
}

/// The bar window's rectangle for a box `area` on a monitor `monitor` at `scale`, its visible
/// frame being `frame` points (as laid out, from the window's top-left). The **frame** is
/// placed, not the window round it (the window is larger, for the widest state and the
/// shadow, and transparent): on the box's top edge, just above or just inside
/// (`core::record::bar_origin`), at the box's left edge, moved left only as far as keeps the
/// frame on the monitor. The window's transparent rest may then go past the monitor's edge.
pub fn bar_rect(
    area: ScreenRect,
    monitor: ScreenRect,
    scale: f32,
    frame: egui::Rect,
) -> ScreenRect {
    let (w, h) = (
        (WINDOW.0 * scale).round() as i32,
        (WINDOW.1 * scale).round() as i32,
    );
    let offset = (
        (frame.min.x * scale).round() as i32,
        (frame.min.y * scale).round() as i32,
    );
    let size = (
        (frame.width() * scale).ceil() as i32,
        (frame.height() * scale).ceil() as i32,
    );
    // From the box's edge to the frame, beyond the outline (2 points).
    let gap = ((GAP + 2.0) * scale).round() as i32;
    let (x, y) = bar_origin(
        PixelRect::new(area.left, area.top, area.right, area.bottom),
        size,
        PixelRect::new(monitor.left, monitor.top, monitor.right, monitor.bottom),
        gap,
    );
    let (x, y) = (x - offset.0, y - offset.1);
    ScreenRect::new(x, y, x + w, y + h)
}

/// The visible frame's rectangle on the screen, for a bar window at `window` whose frame is
/// `frame` points at `scale`.
pub fn frame_on_screen(window: ScreenRect, scale: f32, frame: egui::Rect) -> ScreenRect {
    // As `bar_rect` measures it: the offset rounded, the size rounded up.
    let left = window.left + (frame.min.x * scale).round() as i32;
    let top = window.top + (frame.min.y * scale).round() as i32;
    ScreenRect::new(
        left,
        top,
        left + (frame.width() * scale).ceil() as i32,
        top + (frame.height() * scale).ceil() as i32,
    )
}

impl Bar {
    /// Opens the bar for a box `area` on `monitor`, showing `view`.
    pub fn open(area: ScreenRect, monitor: &Monitor, view: &View) -> windows::core::Result<Bar> {
        let ctx = egui::Context::default();
        if let Err(error) = crate::ui::install(&ctx) {
            tracing::warn!(?error, "the record bar has no text font");
        }
        ctx.enable_accesskit();
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let scale = monitor.scale();
        // Made where a bar as tall as its window would go, then placed by its frame once it's
        // laid out (before it shows).
        let guess = egui::Rect::from_min_size(
            egui::pos2(INSET, INSET),
            vec2(WINDOW.0 - 2.0 * INSET, WINDOW.1 - 2.0 * INSET),
        );
        let panel = Panel::new(
            bar_rect(area, monitor.rect, scale, guess),
            overmark_win::win32::window::title("window-record-bar-name"),
        )?;
        let mut bar = Bar {
            ctx,
            textures: raster::Textures::default(),
            started: Instant::now(),
            panel,
            pixels_per_point: scale,
            events: Vec::new(),
            pointer_down: false,
            shown: None,
            repaint_at: None,
            area,
            monitor: monitor.rect,
            placed_for: None,
        };
        // An area measures itself in its first frame and draws in its second.
        bar.frame(view);
        bar.frame(view);
        bar.panel.show();
        Ok(bar)
    }

    /// Its window (checks drive it through UI Automation; the layer keeps it on top).
    pub fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.panel.hwnd()
    }

    /// Brings it back on top (over an ink layer that came forward).
    pub fn raise(&self) {
        self.panel.show();
    }

    /// Takes its window's input. Returns whether there was any.
    pub fn take_input(&mut self) -> bool {
        let events = self.panel.take_events();
        let any = !events.is_empty();
        let ppp = self.pixels_per_point;
        for e in events {
            match e {
                PanelEvent::Pointer { x, y, down, .. } => {
                    let pos = Pos2::new(x as f32 / ppp, y as f32 / ppp);
                    self.events.push(egui::Event::PointerMoved(pos));
                    if down != self.pointer_down {
                        self.pointer_down = down;
                        self.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: down,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                PanelEvent::Left => self.events.push(egui::Event::PointerGone),
                PanelEvent::Action(request) => {
                    self.events
                        .push(egui::Event::AccessKitActionRequest(request));
                }
            }
        }
        any
    }

    /// Whether a frame is due: input came, `view` differs from what's shown, or egui's wait is
    /// up.
    pub fn due(&self, view: &View) -> bool {
        !self.events.is_empty()
            || self.shown.as_ref() != Some(view)
            || self.repaint_at.is_some_and(|t| Instant::now() >= t)
    }

    /// How long until egui wants a frame by itself (a tooltip's delay), if it does.
    pub fn wait_ms(&self) -> Option<u32> {
        self.repaint_at.map(|t| {
            t.saturating_duration_since(Instant::now())
                .as_millis()
                .min(u128::from(u32::MAX)) as u32
        })
    }

    /// Draws a frame showing `view`; returns what its controls asked for.
    pub fn frame(&mut self, view: &View) -> Vec<BarAction> {
        let ppp = self.pixels_per_point;
        let r = self.panel.rect();
        let (w, h) = (r.width() as usize, r.height() as usize);
        let events = std::mem::take(&mut self.events);
        let drawn = draw_frame(
            &self.ctx,
            &mut self.textures,
            self.started,
            self.panel.pixels(),
            w,
            h,
            ppp,
            events,
            |ctx| show(ctx, view),
        );
        self.panel.set_solid(
            drawn
                .solid
                .iter()
                .map(|r| {
                    ScreenRect::new(
                        r.min.x as i32,
                        r.min.y as i32,
                        r.max.x as i32,
                        r.max.y as i32,
                    )
                })
                .collect(),
        );
        self.panel.present(None);
        if let Some(tree) = drawn.tree {
            self.panel.set_accessibility(tree);
        }
        self.repaint_at =
            (drawn.delay < Duration::from_secs(3600)).then(|| Instant::now() + drawn.delay);
        self.shown = Some(view.clone());
        // Placed by its visible frame, as laid out in this frame.
        if let Some(frame) = frame_points(&self.ctx)
            && self.placed_for != Some(frame)
        {
            self.placed_for = Some(frame);
            let r = bar_rect(self.area, self.monitor, ppp, frame);
            self.panel.move_to(r.left, r.top);
        }
        drawn.actions
    }

    /// The visible frame's rectangle on the screen (for checks and tests).
    pub fn frame_rect(&self) -> Option<ScreenRect> {
        frame_points(&self.ctx)
            .map(|f| frame_on_screen(self.panel.rect(), self.pixels_per_point, f))
    }
}

/// The outline's colours (B, G, R): white before recording, red while recording, amber paused.
fn outline_colour(phase: Phase) -> [u8; 3] {
    match phase {
        Phase::Ready => [0xF0, 0xF0, 0xF0],
        Phase::Recording => [0x25, 0x30, 0xD9],
        Phase::Paused => [0x00, 0xB7, 0xF5],
    }
}

/// A box chosen to record: its outline, its bar and, once Record is pressed, its recording.
pub struct RecordBox {
    /// The box as placed (on one monitor, even-sized), in screen pixels.
    area: ScreenRect,
    monitor: Monitor,
    outline: Outline,
    bar: Bar,
    recording: Option<Recording>,
    confirm_discard: bool,
}

impl RecordBox {
    /// Opens a box `area` (screen pixels) on `monitor`: its outline and its bar, ready to
    /// record.
    pub fn open(area: ScreenRect, monitor: &Monitor) -> windows::core::Result<RecordBox> {
        let view = View {
            phase: Phase::Ready,
            elapsed: Duration::ZERO,
            confirm_discard: false,
        };
        let outline = Outline::new(
            area,
            monitor.rect,
            monitor.scale(),
            outline_colour(Phase::Ready),
        )?;
        outline.show();
        let bar = Bar::open(area, monitor, &view)?;
        Ok(RecordBox {
            area,
            monitor: monitor.clone(),
            outline,
            bar,
            recording: None,
            confirm_discard: false,
        })
    }

    /// The box, in screen pixels.
    pub fn area(&self) -> ScreenRect {
        self.area
    }

    pub fn monitor(&self) -> &Monitor {
        &self.monitor
    }

    /// What the bar shows now.
    pub fn view(&self) -> View {
        let phase = match &self.recording {
            None => Phase::Ready,
            Some(r) if r.status() == overmark_win::record::Status::Paused => Phase::Paused,
            Some(_) => Phase::Recording,
        };
        View {
            phase,
            // Whole seconds, as the timer shows: the bar draws again when the second changes,
            // not on every turn of the loop.
            elapsed: Duration::from_secs(
                self.recording.as_ref().map_or(0, |r| r.elapsed().as_secs()),
            ),
            confirm_discard: self.confirm_discard,
        }
    }

    /// The bar's input, a frame when one is due, and what its controls asked for.
    pub fn take_actions(&mut self) -> Vec<BarAction> {
        self.bar.take_input();
        let mut actions = Vec::new();
        // A control can change what the bar shows (Record, Pause): then it draws again.
        for _ in 0..3 {
            let view = self.view();
            if !self.bar.due(&view) {
                break;
            }
            actions.extend(self.bar.frame(&view));
        }
        actions
    }

    /// How long the loop may wait before the bar needs a frame: the timer's next second while
    /// recording, or egui's own wait.
    pub fn wait_ms(&self) -> Option<u32> {
        let tick = self
            .recording
            .as_ref()
            .filter(|r| r.status() == overmark_win::record::Status::Recording)
            .map(|r| 1000 - r.elapsed().subsec_millis() + 5);
        match (tick, self.bar.wait_ms()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Starts recording with `start` (its area is this box's), `on_end` called from the
    /// recording's thread if it ends by itself.
    pub fn record(
        &mut self,
        mut start: Start,
        on_end: impl Fn() + Send + 'static,
    ) -> Result<(), overmark_core::record::RecordError> {
        start.area = PixelRect::new(
            self.area.left,
            self.area.top,
            self.area.right,
            self.area.bottom,
        );
        self.recording = Some(Recording::start(start, on_end)?);
        self.follow();
        Ok(())
    }

    pub fn pause(&mut self) {
        if let Some(r) = &self.recording {
            r.pause();
        }
        self.follow();
    }

    pub fn resume(&mut self) {
        if let Some(r) = &self.recording {
            r.resume();
        }
        self.follow();
    }

    /// Whether a recording has started (Record was pressed).
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// The recording, if it has started.
    pub fn recording(&self) -> Option<&Recording> {
        self.recording.as_ref()
    }

    /// ✕ on a recording: the bar asks (or, answered Keep, stops asking).
    pub fn set_confirm_discard(&mut self, ask: bool) {
        self.confirm_discard = ask;
    }

    /// Stops and saves, and the box goes: the saved file, or why it couldn't be. `None` if
    /// nothing was recorded yet.
    pub fn stop(
        self,
    ) -> Option<Result<overmark_win::record::Saved, overmark_core::record::RecordError>> {
        self.recording.map(Recording::stop)
    }

    /// Throws the recording away (its file is deleted), and the box goes.
    pub fn discard(self) {
        if let Some(r) = self.recording {
            r.discard();
        }
    }

    /// Brings the outline and the bar back on top (over an ink layer that came forward).
    pub fn raise(&self) {
        self.outline.show();
        self.bar.raise();
    }

    /// The bar's visible frame on the screen.
    pub fn bar_frame(&self) -> Option<ScreenRect> {
        self.bar.frame_rect()
    }

    /// The bar's window and the outline's (checks find them).
    pub fn bar_hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.bar.hwnd()
    }

    pub fn outline_hwnds(&self) -> Vec<windows::Win32::Foundation::HWND> {
        self.outline.hwnds()
    }

    /// The outline follows the phase.
    fn follow(&mut self) {
        let colour = outline_colour(self.view().phase);
        if let Err(error) = self.outline.set_colour(colour) {
            tracing::warn!(%error, "couldn't recolour the record outline");
        }
    }
}

/// The bar showing `view`, drawn as its window draws it into `width` × `height` pixels at
/// `pixels_per_point`; RGBA, premultiplied. For pictures of it (the window is excluded from
/// capture, so no screenshot shows it) and tests.
pub fn render_offscreen(
    view: &View,
    width: usize,
    height: usize,
    pixels_per_point: f32,
) -> Vec<u8> {
    render_offscreen_with_frame(view, width, height, pixels_per_point).0
}

/// As [`render_offscreen`], with the visible frame as laid out (points).
pub fn render_offscreen_with_frame(
    view: &View,
    width: usize,
    height: usize,
    pixels_per_point: f32,
) -> (Vec<u8>, Option<egui::Rect>) {
    let ctx = egui::Context::default();
    let _ = crate::ui::install(&ctx);
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let mut textures = raster::Textures::default();
    let started = Instant::now();
    let mut bgra = vec![0u8; width * height * 4];
    for _ in 0..2 {
        let _ = draw_frame(
            &ctx,
            &mut textures,
            started,
            &mut bgra,
            width,
            height,
            pixels_per_point,
            Vec::new(),
            |ctx| show(ctx, view),
        );
    }
    for p in bgra.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    (bgra, frame_points(&ctx))
}

/// The bar window's size in points, for pictures of it.
pub const WINDOW_POINTS: (f32, f32) = WINDOW;

#[cfg(test)]
mod tests;
