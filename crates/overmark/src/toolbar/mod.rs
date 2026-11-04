//! The drawing toolbar: floating at the top centre of the monitor under the cursor,
//! draggable (where it's left is remembered per monitor), collapsible to a pill, hidden from
//! screen shares, read by screen readers.
//!
//! **egui without a GPU:** the toolbar is the shared egui look ([`crate::ui`]) run on the app's
//! own loop; each frame is tessellated by epaint and drawn on the CPU ([`raster`]) into a
//! layered window of Overmark's own (`overmark_win::win32::panel`), excluded from capture and
//! kept above the ink. An eframe window would cost 186-277 ms and 50-116 MB to make (measured
//! when it was built), far over the opening's budget; this costs a few milliseconds and no GPU device.
//! It draws a frame only when something changed (input, the drawing's state, or egui asking
//! for one, as a tooltip's delay): no CPU when nothing moves.
//!
//! **Screen readers:** egui's AccessKit tree goes to AccessKit's own Windows adapter on the
//! toolbar's window, and an action asked for there (Narrator's, UI Automation's Invoke) comes
//! back into egui as the click it stands for.

pub mod icons;
pub mod raster;
pub mod ui;

use std::time::Instant;

use eframe::egui::{self, Pos2, Rect, ViewportId, vec2};
use overmark_core::settings::ToolbarPosition;
use overmark_win::win32::Rect as ScreenRect;
use overmark_win::win32::dpi::Monitor;
use overmark_win::win32::panel::{Panel, PanelEvent};

pub use ui::{Action, State};

/// The toolbar's window, in points: room for the toolbar and the lists its pickers open
/// below it; the rest is transparent, and the pointer goes through it to the ink.
const WINDOW: (f32, f32) = (1040.0, 360.0);
/// From the monitor's top edge to the window's, by default, in points.
const TOP: f32 = 4.0;

/// The toolbar's egui: its context (fonts, styles, the font atlas) and the textures it gave,
/// kept from one opening to the next, so an opening only makes a window and draws a frame.
pub struct Look {
    ctx: egui::Context,
    textures: raster::Textures,
    started: Instant,
}

impl Look {
    /// Makes it, with the shared fonts and styles.
    pub fn new() -> Look {
        let ctx = egui::Context::default();
        if let Err(error) = crate::ui::install(&ctx) {
            tracing::warn!(?error, "the toolbar has no text font");
        }
        ctx.enable_accesskit();
        // No animated transitions (a button greying out, a hover's fade): a change is one
        // frame, not several, so the toolbar costs a frame when something changes and nothing
        // after.
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        Look {
            ctx,
            textures: raster::Textures::default(),
            started: Instant::now(),
        }
    }

    /// Draws the toolbar once, off screen, at `pixels_per_point` (the monitor's scale): its
    /// glyphs go into the font atlas now, not at the first opening.
    pub fn warm(&mut self, state: &State, pixels_per_point: f32) {
        let t = Instant::now();
        let (w, h) = (
            (WINDOW.0 * pixels_per_point) as usize,
            (WINDOW.1 * pixels_per_point) as usize,
        );
        let mut pixels = vec![0u8; w * h * 4];
        // Two frames: an area measures itself in its first and draws in its second, so the
        // first opening's first frame is shown at once.
        for _ in 0..2 {
            let _ = run_frame(self, state, &mut pixels, w, h, pixels_per_point, Vec::new());
        }
        // Its size is kept (an area shows from its second frame: this was its first).
        // A toolbar window made and dropped, never shown: Windows' UI Automation, which its
        // screen-reader adapter starts, loads now rather than at the first opening (some 25 ms).
        let _ = Panel::new(ScreenRect::new(0, 0, 1, 1), "Overmark toolbar");
        tracing::info!(ms = t.elapsed().as_secs_f64() * 1000.0, "toolbar warmed");
    }
}

impl Default for Look {
    fn default() -> Self {
        Look::new()
    }
}

/// What a frame gave: what its controls asked for, its accessibility tree, when it wants
/// the next frame, the grip's rectangle and what's solid (pixels).
type Frame = (
    Vec<Action>,
    Option<egui::accesskit::TreeUpdate>,
    std::time::Duration,
    Option<Rect>,
    Vec<Rect>,
);

/// What takes the pointer, in pixels: the frames of what's shown (the toolbar, a strip, the
/// More menu), and their border. Not a tooltip, nor a shadow round a frame: those are drawn
/// below the toolbar, and a press there that the window took would never draw.
fn solid(ctx: &egui::Context, ppp: f32) -> Vec<Rect> {
    ctx.memory(|m| {
        let mut layers: Vec<_> = m
            .areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|l| matches!(l.order, egui::Order::Middle | egui::Order::Foreground))
            .filter_map(|l| m.area_rect(l.id))
            .map(|r| {
                let r = r.expand(1.0);
                Rect::from_min_max((r.min * ppp).floor(), (r.max * ppp).ceil())
            })
            .collect();
        // In a stable order (for tests).
        layers.sort_by(|a, b| {
            a.min
                .y
                .total_cmp(&b.min.y)
                .then(a.min.x.total_cmp(&b.min.x))
        });
        layers
    })
}

/// One frame of the toolbar showing `state` with `events`, into `pixels` (BGRA, `w` × `h`).
fn run_frame(
    look: &mut Look,
    state: &State,
    pixels: &mut [u8],
    w: usize,
    h: usize,
    ppp: f32,
    events: Vec<egui::Event>,
) -> Frame {
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            vec2(w as f32 / ppp, h as f32 / ppp),
        )),
        time: Some(look.started.elapsed().as_secs_f64()),
        events,
        focused: true,
        ..Default::default()
    };
    raw.viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(ppp);
    let mut actions = Vec::new();
    let output = look.ctx.run_ui(raw, |ui| {
        actions = ui::show(ui.ctx(), state);
    });
    let grip = look
        .ctx
        .data(|d| d.get_temp::<Rect>(ui::grip_id()))
        .map(|r| Rect::from_min_max(r.min * ppp, r.max * ppp));
    let solid = solid(&look.ctx, ppp);
    look.textures.set(&output.textures_delta);
    let primitives = look.ctx.tessellate(output.shapes, output.pixels_per_point);
    pixels.fill(0);
    raster::paint(
        &mut raster::Target {
            pixels,
            width: w,
            height: h,
        },
        output.pixels_per_point,
        &primitives,
        &look.textures,
    );
    look.textures.free(&output.textures_delta);
    let mut delta = output.textures_delta;
    delta.clear();
    let delay = output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| v.repaint_delay)
        .unwrap_or(std::time::Duration::MAX);
    (
        actions,
        output.platform_output.accesskit_update,
        delay,
        grip,
        solid,
    )
}

/// The toolbar while the layer is open.
pub struct Toolbar {
    look: Look,
    panel: Panel,
    monitor: Monitor,
    pixels_per_point: f32,
    events: Vec<egui::Event>,
    pointer_down: bool,
    /// Pressed since the last [`Toolbar::take_pressed`].
    pressed: bool,
    /// Where the grip is, in client pixels (from the last frame).
    grip: Option<Rect>,
    /// Dragging the grip: where the pointer and the window were when it started.
    drag: Option<((i32, i32), (i32, i32))>,
    /// Where it was left, if it moved since the last [`Toolbar::take_moved`].
    moved: bool,
    /// What it last showed, and when egui wants another frame (if it does).
    shown: Option<State>,
    repaint_at: Option<Instant>,
}

/// The window's rectangle for `monitor` at `scale`, at `saved` (its top-left from the
/// monitor's, kept inside it) or else at the top centre.
pub fn place(monitor: ScreenRect, scale: f32, saved: Option<ToolbarPosition>) -> ScreenRect {
    let w = ((WINDOW.0 * scale).round() as i32)
        .min(monitor.width())
        .max(1);
    let h = ((WINDOW.1 * scale).round() as i32)
        .min(monitor.height())
        .max(1);
    let (x, y) = match saved {
        Some(p) => (p.x, p.y),
        None => ((monitor.width() - w) / 2, (TOP * scale).round() as i32),
    };
    let x = x.clamp(0, (monitor.width() - w).max(0));
    let y = y.clamp(0, (monitor.height() - h).max(0));
    ScreenRect::new(
        monitor.left + x,
        monitor.top + y,
        monitor.left + x + w,
        monitor.top + y + h,
    )
}

impl Toolbar {
    /// Opens it on `monitor`, where it was left there (`saved`) or at the top centre, and
    /// draws its first frame.
    pub fn open(
        look: Look,
        monitor: &Monitor,
        saved: Option<ToolbarPosition>,
        state: &State,
    ) -> Result<(Toolbar, Vec<Action>), (Look, windows::core::Error)> {
        let t0 = Instant::now();
        let scale = monitor.scale();
        let rect = place(monitor.rect, scale, saved);
        let panel = match Panel::new(rect, "Overmark toolbar") {
            Ok(p) => p,
            Err(error) => return Err((look, error)),
        };
        let panel_ms = t0.elapsed().as_secs_f64() * 1000.0;
        panel.set_title(&overmark_core::i18n::tr("toolbar-name"));
        let mut toolbar = Toolbar {
            look,
            panel,
            monitor: monitor.clone(),
            pixels_per_point: scale,
            events: Vec::new(),
            pointer_down: false,
            pressed: false,
            grip: None,
            drag: None,
            moved: false,
            shown: None,
            repaint_at: None,
        };
        let t1 = Instant::now();
        let mut actions = toolbar.frame(state);
        // A frame that only measured (egui wants the next at once) isn't shown: draw that one.
        let mut frames = 1;
        while toolbar.wait_ms() == Some(0) && frames < 3 {
            actions.extend(toolbar.frame(state));
            frames += 1;
        }
        let frame_ms = t1.elapsed().as_secs_f64() * 1000.0;
        toolbar.panel.show();
        tracing::debug!(panel_ms, frame_ms, frames, "toolbar made");
        Ok((toolbar, actions))
    }

    /// Closes it, keeping its egui for the next opening.
    pub fn close(self) -> Look {
        let look = self.look;
        // Nothing it had open (More's menu, a group's strip) comes back with it; its size
        // stays known, so the next opening's first frame shows it.
        egui::Popup::close_all(&look.ctx);
        ui::set_open_group(&look.ctx, None);
        look
    }

    /// Whether the width's strip is open: the keys come to the ink layer
    /// (the toolbar's window never takes the focus), so the app gives its slider the arrows.
    pub fn width_open(&self) -> bool {
        ui::open_group(&self.look.ctx) == Some(ui::Group::Width)
    }

    /// A key for the toolbar, as if it had the focus: pressed and let go, at the next frame.
    pub fn key(&mut self, key: egui::Key) {
        for pressed in [true, false] {
            self.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    /// Whether it was pressed (a control or the grip) since the last call.
    pub fn take_pressed(&mut self) -> bool {
        std::mem::take(&mut self.pressed)
    }

    /// Brings it back over the ink layer (which comes forward when it takes the focus).
    pub fn raise(&self) {
        self.panel.show();
    }

    /// Its window (checks find it, and the layer keeps it on top).
    pub fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.panel.hwnd()
    }

    /// The monitor it's on (its device name keys the remembered position).
    pub fn monitor(&self) -> &Monitor {
        &self.monitor
    }

    /// Its top-left corner from the monitor's, once it has moved since the last call: to
    /// remember in Settings.
    pub fn take_moved(&mut self) -> Option<ToolbarPosition> {
        if !std::mem::take(&mut self.moved) {
            return None;
        }
        let r = self.panel.rect();
        Some(ToolbarPosition {
            x: r.left - self.monitor.rect.left,
            y: r.top - self.monitor.rect.top,
        })
    }

    /// Takes its window's input. Returns whether there was any (a frame is due).
    pub fn take_input(&mut self) -> bool {
        let events = self.panel.take_events();
        let any = !events.is_empty();
        let ppp = self.pixels_per_point;
        for e in events {
            match e {
                PanelEvent::Pointer { x, y, screen, down } => {
                    self.pressed |= down && !self.pointer_down && self.drag.is_none();
                    if let Some(((cx, cy), (wx, wy))) = self.drag {
                        // The grip is held: the toolbar follows the pointer.
                        if down {
                            self.move_to(wx + screen.0 - cx, wy + screen.1 - cy);
                            continue;
                        }
                        self.drag = None;
                        self.moved = true;
                    }
                    // A press on the grip: the toolbar follows the pointer until it's let go.
                    if down
                        && !self.pointer_down
                        && self
                            .grip
                            .is_some_and(|g| g.contains(Pos2::new(x as f32, y as f32)))
                    {
                        let r = self.panel.rect();
                        self.drag = Some((screen, (r.left, r.top)));
                        continue;
                    }
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
                PanelEvent::Left => {
                    if self.drag.is_none() {
                        self.events.push(egui::Event::PointerGone);
                    }
                }
                PanelEvent::Action(request) => {
                    self.events
                        .push(egui::Event::AccessKitActionRequest(request));
                }
            }
        }
        any
    }

    fn move_to(&mut self, x: i32, y: i32) {
        // Kept on its monitor.
        let m = self.monitor.rect;
        let r = self.panel.rect();
        let x = x.clamp(m.left, (m.right - r.width()).max(m.left));
        let y = y.clamp(m.top, (m.bottom - r.height()).max(m.top));
        self.panel.move_to(x, y);
    }

    /// Whether a frame is due: input came, `state` differs from what's shown, or egui's
    /// wait is up.
    pub fn due(&self, state: &State) -> bool {
        !self.events.is_empty()
            || self.shown.as_ref() != Some(state)
            || self.repaint_at.is_some_and(|t| Instant::now() >= t)
    }

    /// How long until egui wants a frame by itself (a tooltip's delay, an animation), if it
    /// does: the loop's wait.
    pub fn wait_ms(&self) -> Option<u32> {
        self.repaint_at.map(|t| {
            t.saturating_duration_since(Instant::now())
                .as_millis()
                .min(u128::from(u32::MAX)) as u32
        })
    }

    /// Draws a frame showing `state`, with the input since the last; returns what its
    /// controls asked for.
    pub fn frame(&mut self, state: &State) -> Vec<Action> {
        let started = Instant::now();
        let ppp = self.pixels_per_point;
        let r = self.panel.rect();
        let (w, h) = (r.width() as usize, r.height() as usize);
        let events = std::mem::take(&mut self.events);
        let (actions, tree, delay, grip, solid) = run_frame(
            &mut self.look,
            state,
            self.panel.pixels(),
            w,
            h,
            ppp,
            events,
        );
        self.grip = grip;
        self.panel.set_solid(
            solid
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
        if let Some(tree) = tree {
            self.panel.set_accessibility(tree);
        }
        self.repaint_at =
            (delay < std::time::Duration::from_secs(3600)).then(|| Instant::now() + delay);
        self.shown = Some(state.clone());
        tracing::trace!(
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "toolbar frame"
        );
        actions
    }
}

/// The toolbar showing `state`, drawn as its window draws it (egui, then [`raster`]) into
/// `width` × `height` pixels at `pixels_per_point`, with `pointer` hovering if given; RGBA,
/// premultiplied. For pictures of it (the window itself is excluded from capture) and tests.
pub fn render_offscreen(
    state: &State,
    width: usize,
    height: usize,
    pixels_per_point: f32,
    frames: usize,
    pointer: Option<Pos2>,
) -> Vec<u8> {
    let mut look = Look::new();
    let mut bgra = vec![0u8; width * height * 4];
    for _ in 0..frames.max(1) {
        let events = pointer.map(egui::Event::PointerMoved).into_iter().collect();
        let _ = run_frame(
            &mut look,
            state,
            &mut bgra,
            width,
            height,
            pixels_per_point,
            events,
        );
    }
    for p in bgra.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_opens_at_the_top_centre_or_where_it_was_left_kept_on_its_monitor() {
        let m = ScreenRect::new(-1920, 0, 0, 1080);
        let r = place(m, 1.0, None);
        assert_eq!((r.width(), r.height()), (1040, 360));
        assert_eq!(r.left - m.left, (1920 - 1040) / 2);
        assert_eq!(r.top, 4);
        // At 150 %.
        let r = place(m, 1.5, None);
        assert_eq!(r.width(), 1560);
        // Where it was left.
        let r = place(m, 1.0, Some(ToolbarPosition { x: 100, y: 500 }));
        assert_eq!((r.left, r.top), (-1820, 500));
        // Off the monitor (a smaller one now): pulled back inside.
        let r = place(m, 1.0, Some(ToolbarPosition { x: 5000, y: -40 }));
        assert_eq!((r.right, r.top), (0, 0));
    }

    /// The pointer goes through the window's transparent pixels (a layered window takes it on
    /// any pixel that isn't alpha 0): with a strip open, below the toolbar only the strip's
    /// own pixels and the few of its shadow are drawn, and the rest is clear.
    #[test]
    fn with_a_strip_open_only_its_own_pixels_take_the_pointer() {
        overmark_core::i18n::init("en");
        let state = State {
            backdrop: overmark_core::backdrop::Backdrop::Live,
            guides: overmark_core::backdrop::Guides::Off,
            tool: overmark_core::tool::ToolKind::Pen,
            colour: overmark_core::settings::DEFAULT_PALETTE[0],
            palette: overmark_core::settings::DEFAULT_PALETTE.to_vec(),
            width: Some(4.0),
            can_undo: true,
            can_redo: false,
            collapsed: false,
            disappearing_ink: false,
            right_click_undoes: true,
        };
        let (w, h) = (1040, 360);
        for group in [
            ui::Group::Backdrop,
            ui::Group::Guides,
            ui::Group::Tool,
            ui::Group::Colour,
            ui::Group::Width,
        ] {
            let mut look = Look::new();
            let mut pixels = vec![0u8; w * h * 4];
            ui::set_open_group(&look.ctx, Some(group));
            for _ in 0..3 {
                let _ = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, Vec::new());
            }
            let alpha = |x: usize, y: usize| pixels[(y * w + x) * 4 + 3];
            // Below the toolbar's row (which ends by y 64): the strip's box, opaque, and what
            // isn't clear round it.
            let (top, bottom) = (64, h);
            let bbox = |min: u8| {
                let mut b = (w, h, 0, 0);
                for y in top..bottom {
                    for x in 0..w {
                        if alpha(x, y) >= min {
                            b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                        }
                    }
                }
                b
            };
            let solid = bbox(250);
            let any = bbox(1);
            assert!(
                solid.2 > solid.0 && solid.3 > solid.1,
                "{group:?}'s strip is drawn"
            );
            let spill = [
                solid.0 - any.0,
                solid.1.saturating_sub(any.1),
                any.2 - solid.2,
                any.3 - solid.3,
            ];
            assert!(
                spill.iter().all(|s| *s <= 8),
                "{group:?}: drawn {any:?}, the strip {solid:?}: {spill:?} past its edges"
            );
            // Its middle takes the pointer; 10 px under it, it goes through.
            assert_eq!(alpha((solid.0 + solid.2) / 2, (solid.1 + solid.3) / 2), 255);
            assert_eq!(alpha((solid.0 + solid.2) / 2, solid.3 + 10), 0);
        }
    }

    /// A press just below the toolbar sometimes didn't draw. A
    /// tooltip is drawn there (and the shadows' faint edges round the frames), in the
    /// toolbar's window, which took the press. Now only the frames of what's shown take the
    /// pointer: the toolbar's and a strip's, not a tooltip nor a few pixels out.
    #[test]
    fn only_the_toolbars_and_the_strips_frames_take_the_pointer_not_a_tooltip() {
        overmark_core::i18n::init("en");
        let state = State {
            backdrop: overmark_core::backdrop::Backdrop::Live,
            guides: overmark_core::backdrop::Guides::Off,
            tool: overmark_core::tool::ToolKind::Pen,
            colour: overmark_core::settings::DEFAULT_PALETTE[0],
            palette: overmark_core::settings::DEFAULT_PALETTE.to_vec(),
            width: Some(4.0),
            can_undo: true,
            can_redo: false,
            collapsed: false,
            disappearing_ink: false,
            right_click_undoes: true,
        };
        let (w, h) = (1040, 360);
        let takes = |solid: &[Rect], x: usize, y: usize| {
            solid
                .iter()
                .any(|r| r.contains(Pos2::new(x as f32 + 0.5, y as f32 + 0.5)))
        };
        // Tooltips at once, for the test (the app's wait for them is half a second).
        let mut look = Look::new();
        look.ctx.all_styles_mut(|s| {
            s.interaction.tooltip_delay = 0.0;
            s.interaction.show_tooltips_only_when_still = false;
        });
        let mut pixels = vec![0u8; w * h * 4];
        let alpha = |pixels: &[u8], x: usize, y: usize| pixels[(y * w + x) * 4 + 3];
        let mut solid = Vec::new();
        for _ in 0..3 {
            (.., solid) = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, Vec::new());
        }
        assert_eq!(solid.len(), 1, "the toolbar alone: {solid:?}");
        let bar = solid[0];
        // Hover over the grip, at the toolbar's left end: its tooltip shows below.
        let grip = Pos2::new(bar.left() + 16.0, bar.center().y);
        for _ in 0..4 {
            let events = vec![egui::Event::PointerMoved(grip)];
            (.., solid) = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, events);
        }
        let below = bar.bottom() as usize + 2;
        let drawn: Vec<(usize, usize)> = (below..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha(&pixels, x, y) > 0)
            .collect();
        assert!(
            drawn.iter().any(|&(x, y)| alpha(&pixels, x, y) == 255),
            "the grip's tooltip is drawn below the toolbar"
        );
        // Not one of its pixels, nor its shadow's, takes the pointer: a press there draws.
        let taken: Vec<_> = drawn
            .iter()
            .filter(|&&(x, y)| takes(&solid, x, y))
            .collect();
        assert!(
            taken.is_empty(),
            "{} of the tooltip's pixels take it: {solid:?}, {:?}",
            taken.len(),
            taken.first()
        );
        // The toolbar does, to its edge, and a few pixels out it goes through.
        let mid = bar.center().y as usize;
        for x in [bar.left() as usize + 2, bar.right() as usize - 3] {
            assert!(takes(&solid, x, mid), "the toolbar at {x}");
        }
        for (x, y) in [
            (bar.left() as usize - 6, mid),
            (bar.right() as usize + 5, mid),
            (bar.center().x as usize, bar.bottom() as usize + 5),
        ] {
            assert!(!takes(&solid, x, y), "({x}, {y}), a few pixels out");
        }
        // With a strip open: it takes the pointer too, and below it nothing does.
        let events = vec![egui::Event::PointerGone];
        ui::set_open_group(&look.ctx, Some(ui::Group::Tool));
        let _ = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, events);
        for _ in 0..2 {
            (.., solid) = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, Vec::new());
        }
        assert_eq!(solid.len(), 2, "the toolbar and the strip: {solid:?}");
        let strip = solid[1];
        assert!(strip.top() > bar.bottom() - 2.0, "{strip:?} below {bar:?}");
        assert!(takes(
            &solid,
            strip.center().x as usize,
            strip.center().y as usize
        ));
        for y in [strip.bottom() as usize + 5, strip.bottom() as usize + 40] {
            assert!(
                !takes(&solid, strip.center().x as usize, y),
                "{y}, under the strip"
            );
        }
        // Every pixel the toolbar and its strip draw solid (not their shadows) takes it.
        for y in 0..h {
            for x in 0..w {
                if alpha(&pixels, x, y) == 255 {
                    assert!(
                        takes(&solid, x, y),
                        "({x}, {y}) is drawn but doesn't take it"
                    );
                }
            }
        }
        // The More menu takes it too.
        ui::set_open_group(&look.ctx, None);
        let mut tree = None;
        for _ in 0..2 {
            let f = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, Vec::new());
            tree = f.1.or(tree);
        }
        let more = tree
            .expect("an accessibility tree")
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("More"))
            .and_then(|(_, n)| n.bounds())
            .expect("More, with its bounds");
        let at = Pos2::new(
            ((more.x0 + more.x1) / 2.0) as f32,
            ((more.y0 + more.y1) / 2.0) as f32,
        );
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let events = vec![egui::Event::PointerMoved(at), click(true), click(false)];
        let _ = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, events);
        for _ in 0..2 {
            (.., solid) = run_frame(&mut look, &state, &mut pixels, w, h, 1.0, Vec::new());
        }
        assert_eq!(solid.len(), 2, "the toolbar and the menu: {solid:?}");
        for y in 0..h {
            for x in 0..w {
                if alpha(&pixels, x, y) == 255 {
                    assert!(
                        takes(&solid, x, y),
                        "({x}, {y}) of the menu doesn't take it"
                    );
                }
            }
        }
    }

    #[test]
    fn it_draws_its_text_and_frame_and_leaves_the_rest_clear() {
        overmark_core::i18n::init("en");
        let state = State {
            backdrop: overmark_core::backdrop::Backdrop::Frozen,
            guides: overmark_core::backdrop::Guides::Off,
            tool: overmark_core::tool::ToolKind::Pen,
            colour: overmark_core::settings::DEFAULT_PALETTE[0],
            palette: overmark_core::settings::DEFAULT_PALETTE.to_vec(),
            width: Some(4.0),
            can_undo: true,
            can_redo: false,
            collapsed: false,
            disappearing_ink: false,
            right_click_undoes: true,
        };
        let (w, h) = (1040, 360);
        // An area measures itself in its first frame and shows from the next ones.
        let rgba = render_offscreen(&state, w, h, 1.0, 3, None);
        let alpha = |x: usize, y: usize| rgba[(y * w + x) * 4 + 3];
        // The toolbar's middle is drawn, opaque; the window's bottom is clear (the pointer
        // goes through there).
        let most = (0..w).map(|x| alpha(x, 30)).max();
        assert_eq!(most, Some(255), "an opaque toolbar row");
        assert_eq!(alpha(w / 2, h - 5), 0);
        // Text: more than one colour in the toolbar's row (glyphs, not a flat fill).
        let row: std::collections::HashSet<[u8; 4]> = (0..w)
            .map(|x| {
                let i = (30 * w + x) * 4;
                [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
            })
            .collect();
        assert!(row.len() > 10, "{} colours", row.len());
    }
}
