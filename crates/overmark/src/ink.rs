//! The drawing while the layer is open: one core session per monitor, fed by the ink
//! layers' pointer and key events, redrawn by dirty rectangles once per turn of the loop.
//!
//! - **Pointer:** a contact (the mouse's left button, a pen or a finger touching) draws with the
//!   tool on the monitor where it started; a pen's eraser end erases for that contact. The laser
//!   follows the pointer whether it touches or not.
//! - **History:** each monitor's session keeps its own steps; [`History`] keeps which monitors
//!   each action changed, so Ctrl+Z undoes the newest action wherever it was and Delete's Clear
//!   of every monitor undoes as one.
//! - **Animation:** only while something fades or the laser's trail is showing
//!   ([`Ink::flush`] says so); otherwise the loop waits with no timeout.
//! - **Timings:** each sample's age when its pixels were shown, and each redraw's time, are
//!   summarised in the log when the layer closes.

use std::collections::HashMap;

use kurbo::Rect;
use overmark_core::backdrop::Backdrop;
use overmark_core::colour::Colour;
use overmark_core::drawing::{InkPoint, Item, ItemId};
use overmark_core::render::{self, Highlight, LaserLook, RenderOptions};
use overmark_core::session::Session;
use overmark_core::session::ToolError;
use overmark_core::settings::{DrawingSettings, Widths};
use overmark_core::tool::ToolKind;
use overmark_win::overlay::{Overlay, OverlayEvent, Phase};
use overmark_win::win32::clock;
use overmark_win::win32::pointer::Sample;

use crate::input::{self, History, KeyAction, TextKey};

/// What the app must do after an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Esc: close the layer.
    Close,
    /// F: Live and Frozen swap.
    ToggleFreeze,
    /// W or Shift+W (`back`): the next or previous backdrop.
    Backdrop { back: bool },
    /// G or Shift+G: the next or previous guides.
    Guides { back: bool },
    /// Tell the user why something can't be done (Blur on Live).
    Notice(String),
}

/// A toolbar control's command: what its key does, but as itself, so it acts while
/// text is being typed (where the keys are text) and never counts as a key held down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Tool(ToolKind),
    Colour(usize),
    Undo,
    Redo,
    Clear,
    Close,
    ToggleFreeze,
}

/// A pointer touching down, until it lifts.
#[derive(Debug, Clone, Copy)]
struct Contact {
    /// The pointer touching (the pen, a finger, the mouse): another one's samples meanwhile
    /// (the mouse hovering while the pen draws) don't end it.
    pointer: u32,
    screen: usize,
    /// Undone, refused, or ended by a key while still down: the rest of it draws nothing.
    over: bool,
    /// The tool to go back to when it lifts (the pen's eraser end erases for one contact).
    restore: Option<ToolKind>,
    last: InkPoint,
    shift: bool,
}

/// Every monitor's drawing while the layer is open, and the tool, colour and widths, which stay
/// from one opening to the next.
#[derive(Default)]
pub struct Ink {
    sessions: Vec<Session>,
    origins: Vec<(i32, i32)>,
    scales: Vec<f32>,
    history: History,
    tool: Option<ToolKind>,
    colour: Option<Colour>,
    /// Widths changed with the wheel, by tool (settings' units: pixels at 100 %).
    widths: Vec<(ToolKind, f32)>,
    contact: Option<Contact>,
    /// What was drawn when the contact touched down (after anything the touch itself made):
    /// what its lift is compared with, as an eraser removes items while it moves.
    contact_marks: Marks,
    /// When each finished item the history or a contact refers to was drawn, by (monitor, id)
    /// (`None`: it never fades, a blur): to tell when the items of a step have all faded, as
    /// its session then drops the step. Kept only while something refers to the item.
    seen: HashMap<(usize, u64), Option<u64>>,
    /// The drawing's clock, fixed (tests); `None`: the performance counter.
    fixed_now: Option<u64>,
    /// The tool key held down, for "hold a tool key and scroll".
    held: Option<u32>,
    /// The monitor the laser is on.
    laser_on: Option<usize>,
    dirty: Vec<Option<Rect>>,
    full: Vec<bool>,
    /// The times of the samples not shown yet (100 ns), and the timings so far.
    pending: Vec<i64>,
    latency_ms: Vec<f32>,
    redraw_ms: Vec<f32>,
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// Each monitor's finished items, by id ([`Ink::marks`]).
type Marks = Vec<Vec<u64>>;

/// Disappearing ink's delay, if it's on.
fn fade_ms(settings: &DrawingSettings) -> Option<u64> {
    settings
        .disappearing_ink
        .then(|| (settings.disappearing_ink_seconds.max(0.0) * 1000.0) as u64)
}

/// When `item` starts to count towards fading (`None`: never), as core's `render::opacity`
/// judges it: a finished item from when it was last drawn; a blur never.
fn fades_from(item: &Item) -> Option<u64> {
    (item.finished && item.tool != ToolKind::Blur).then(|| item.drawn_at())
}

/// Whether an item drawn at `drawn` has faded to nothing at `now` with a `delay`: exactly when
/// core's `render::opacity` reaches 0 (the delay, then `FADE_MS`).
fn faded_by(drawn: u64, now: u64, delay: u64) -> bool {
    now >= drawn.saturating_add(delay).saturating_add(render::FADE_MS)
}

/// The ids of `s`'s finished items, in drawing order.
fn finished(s: &Session) -> Vec<u64> {
    s.drawing()
        .items()
        .filter(|item| item.finished)
        .map(|item| item.id.0)
        .collect()
}

/// Keeps the timings from growing without end on a very long opening.
const MAX_TIMINGS: usize = 100_000;

impl Ink {
    /// The layer opened over `overlay`'s monitors: a fresh session for each, with the tool and
    /// colour (the settings' defaults the first time).
    pub fn open(&mut self, overlay: &Overlay, settings: &DrawingSettings, backdrop: Backdrop) {
        let n = overlay.screen_count();
        self.origins = overlay
            .screen_rects()
            .iter()
            .map(|r| (r.left, r.top))
            .collect();
        self.scales = (0..n)
            .map(|i| overlay.monitor(i).map_or(1.0, |m| m.scale()))
            .collect();
        self.sessions = (0..n).map(|_| Session::new()).collect();
        self.tool.get_or_insert(settings.default_tool);
        self.colour.get_or_insert_with(|| {
            settings
                .palette
                .get(settings.default_colour)
                .or(settings.palette.first())
                .copied()
                .unwrap_or(Colour::rgb(255, 0, 0))
        });
        self.history.clear();
        self.seen.clear();
        self.contact = None;
        self.held = None;
        self.laser_on = None;
        self.dirty = vec![None; n];
        self.full = vec![false; n];
        self.pending.clear();
        self.latency_ms.clear();
        self.redraw_ms.clear();
        self.set_backdrop(backdrop);
        // The layers open empty and the drawing is: nothing to redraw (a full redraw of every
        // monitor would cost the opening some 20 ms).
        self.full.iter_mut().for_each(|f| *f = false);
        self.apply(settings);
    }

    /// The layer closed: the drawing and its history go, and the timings are
    /// logged.
    pub fn close(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        log_timings("input latency", &mut self.latency_ms);
        log_timings("redraw", &mut self.redraw_ms);
        self.sessions.clear();
        self.history.clear();
        self.seen.clear();
        self.contact = None;
        self.held = None;
        self.pending.clear();
    }

    /// Monitor `i`'s session.
    pub fn session(&self, i: usize) -> Option<&Session> {
        self.sessions.get(i)
    }

    /// Monitor `i`'s session, to change it outside the events (`--test-drawing`); it's redrawn
    /// in full at the next flush.
    pub fn session_mut(&mut self, i: usize) -> Option<&mut Session> {
        if let Some(f) = self.full.get_mut(i) {
            *f = true;
        }
        self.sessions.get_mut(i)
    }

    /// What the toolbar shows of the drawing: the tool, the colour, the tool's width (Settings'
    /// pixels, as the wheel changes it; none for the eraser and the laser), and whether
    /// there's something to undo and to redo.
    pub fn toolbar_view(
        &self,
        settings: &DrawingSettings,
    ) -> (ToolKind, Colour, Option<f32>, bool, bool) {
        let tool = self.tool();
        let width = settings
            .widths
            .get(tool)
            .map(|_| self.width_of(tool, &settings.widths));
        let colour = self.colour.unwrap_or(Colour::rgb(255, 0, 0));
        let drawing = self.contact.is_some_and(|c| !c.over) || self.typing().is_some();
        let can_undo = drawing || self.sessions.iter().any(Session::can_undo);
        let can_redo = !drawing && self.sessions.iter().any(Session::can_redo);
        (tool, colour, width, can_undo, can_redo)
    }

    /// Redraws every layer in full at the next flush: what's under the ink changed (the
    /// guides, which a blur pixelates).
    pub fn redraw_all(&mut self) {
        self.full.iter_mut().for_each(|f| *f = true);
    }

    /// Settings changed how the boards or the guides look (from `old` to `new`): a blur on the
    /// frozen frame was made over them, so every layer is drawn again, as G does. Returns
    /// whether anything showing changed (a board hotkey alone doesn't).
    pub fn look_changed(
        &mut self,
        old: &overmark_core::settings::Settings,
        new: &overmark_core::settings::Settings,
    ) -> bool {
        let (a, b) = (&old.backdrops, &new.backdrops);
        let changed = a.whiteboard_colour != b.whiteboard_colour
            || a.blackboard_colour != b.blackboard_colour
            || old.guides != new.guides;
        if changed {
            self.redraw_all();
        }
        changed
    }

    /// The backdrop changed (F): Blur's availability follows it, and every layer is redrawn
    /// (a blur shows only over a frozen frame).
    pub fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.end_contact();
        for s in &mut self.sessions {
            s.backdrop = backdrop;
        }
        self.full.iter_mut().for_each(|f| *f = true);
    }

    fn tool(&self) -> ToolKind {
        self.tool.unwrap_or(ToolKind::Pen)
    }

    fn width_of(&self, tool: ToolKind, widths: &Widths) -> f32 {
        self.widths
            .iter()
            .find(|(t, _)| *t == tool)
            .map(|(_, w)| *w)
            .or_else(|| widths.get(tool))
            .unwrap_or(4.0)
    }

    /// Settings changed the drawing settings (from `old` to `new`), whether the layer is open
    /// or not: what the user changed there takes effect now, the drawing stays.
    /// - a new starting tool or colour is picked now, and is what the next opening has;
    /// - a palette colour changed while it's the one in use follows the change;
    /// - a tool's width changed in Settings replaces one set with the wheel, for that tool.
    pub fn settings_changed(&mut self, old: &DrawingSettings, new: &DrawingSettings) {
        if new.default_tool != old.default_tool {
            self.end_contact();
            self.tool = Some(new.default_tool);
        }
        let start = |s: &DrawingSettings| s.palette.get(s.default_colour).copied();
        let current = self.colour;
        if new.default_colour != old.default_colour || start(new) != start(old) {
            self.colour = start(new).or(current);
        } else if let Some(c) = current
            && let Some(i) = old.palette.iter().position(|p| *p == c)
            && let Some(changed) = new.palette.get(i).filter(|n| **n != c)
        {
            self.colour = Some(*changed);
        }
        self.widths
            .retain(|(t, _)| old.widths.get(*t) == new.widths.get(*t));
        self.apply(new);
        self.full.iter_mut().for_each(|f| *f = true);
    }

    /// Gives every session the tool, colour and width (scaled to its monitor).
    fn apply(&mut self, settings: &DrawingSettings) {
        let tool = self.tool();
        let width = self.width_of(tool, &settings.widths);
        let colour = self.colour.unwrap_or(Colour::rgb(255, 0, 0));
        for (s, scale) in self.sessions.iter_mut().zip(&self.scales) {
            s.tool = tool;
            s.colour = colour;
            s.width = width * scale;
        }
    }

    fn mark(&mut self, i: usize, area: Option<Rect>) {
        if let Some(d) = self.dirty.get_mut(i) {
            *d = union(*d, area);
        }
    }

    /// What's drawn and finished on each monitor, by item: what a new undo step changes. (A
    /// step can't be told from the length of a session's history: at its 100 steps a new one
    /// pushes the oldest out, and the length stays.)
    fn marks(&self) -> Marks {
        self.sessions.iter().map(finished).collect()
    }

    /// Records the monitors whose finished items changed since `before` as one action (each
    /// action makes at most one step on a monitor: text being typed is finished first, as its
    /// own), and keeps the history in step with the sessions'.
    fn record_since(&mut self, before: &Marks) {
        let after = self.marks();
        let mut group = Vec::new();
        for (i, (after, before)) in after.iter().zip(before).enumerate() {
            if after == before {
                continue;
            }
            // The items the step concerns: those it added or removed.
            let mut ids: Vec<u64> = after
                .iter()
                .filter(|id| !before.contains(id))
                .chain(before.iter().filter(|id| !after.contains(id)))
                .copied()
                .collect();
            ids.sort_unstable();
            for &id in &ids {
                if let Some(item) = self.sessions[i].drawing().item(ItemId(id)) {
                    self.seen.entry((i, id)).or_insert_with(|| fades_from(item));
                }
            }
            group.push((i, ids));
        }
        self.history.record(group);
        self.sync();
        self.retire();
    }

    /// Forgets the drawn-at times of items nothing refers to any more: not drawn, nor in a step
    /// (dropped, faded, or past the redo side), nor a contact's, so a long opening with
    /// disappearing ink stays bounded. (A drawn item keeps its time: a later erase of it needs
    /// it, even once its own step is past the 100.)
    fn retire(&mut self) {
        let drawn = self.marks();
        let referred: std::collections::HashSet<(usize, u64)> = self
            .history
            .ids()
            .chain(
                drawn
                    .iter()
                    .enumerate()
                    .flat_map(|(s, ids)| ids.iter().map(move |&id| (s, id))),
            )
            .chain(
                self.contact_marks
                    .iter()
                    .enumerate()
                    .flat_map(|(s, ids)| ids.iter().map(move |&id| (s, id))),
            )
            .collect();
        self.seen.retain(|key, _| referred.contains(key));
    }

    /// The drawing's clock now (milliseconds of the performance counter).
    fn now(&self) -> u64 {
        self.fixed_now
            .unwrap_or_else(|| input::ms(clock::now_100ns()))
    }

    /// With disappearing ink, prunes every monitor at `now`: before Ctrl+Z or Ctrl+Y, so a
    /// step whose items have faded is gone (with its session's) before it could be undone,
    /// on monitors with nothing fading on screen too.
    fn prune_all(&mut self, settings: &DrawingSettings) {
        let Some(delay) = fade_ms(settings) else {
            return;
        };
        let now = self.now();
        for i in 0..self.sessions.len() {
            let gone = self.prune(i, now, delay);
            self.mark(i, gone);
        }
    }

    /// With disappearing ink, removes what has faded away from monitor `i` at `now` (core's
    /// `prune_faded`) and the history's parts for the steps its session dropped with it.
    /// Returns the area it covered.
    fn prune(&mut self, i: usize, now: u64, delay: u64) -> Option<Rect> {
        let before = finished(&self.sessions[i]);
        // It may drop steps about items no longer drawn (erased, cleared, undone), so the
        // history is checked each time, not only when something drawn went.
        let gone = self.sessions[i].prune_faded(now, delay);
        // What faded away isn't a contact's doing: its lift isn't compared with it.
        if let Some(m) = self.contact_marks.get_mut(i) {
            let after = finished(&self.sessions[i]);
            m.retain(|id| !before.contains(id) || after.contains(id));
        }
        // Only this monitor's parts: the others' sessions weren't pruned.
        let seen = &self.seen;
        let faded = |s: usize, id: u64| {
            s == i
                && seen
                    .get(&(s, id))
                    .copied()
                    .flatten()
                    .is_some_and(|drawn| faded_by(drawn, now, delay))
        };
        self.history.forget_faded(faded);
        self.sync();
        self.retire();
        gone
    }

    /// A session forgets its oldest step past 100 on its own: the history forgets that
    /// monitor's oldest too.
    fn sync(&mut self) {
        for (i, s) in self.sessions.iter().enumerate() {
            self.history.sync(i, s.undo_steps(), s.redo_steps());
        }
    }

    /// Whether text is being typed (its keys are its own: the arrows move its caret).
    pub fn is_typing(&self) -> bool {
        self.typing().is_some()
    }

    fn typing(&self) -> Option<usize> {
        self.sessions
            .iter()
            .position(|s| s.editing_text().is_some())
    }

    /// Where `sample` is on monitor `screen`'s layer.
    fn point(&self, screen: usize, s: &Sample) -> InkPoint {
        let origin = self.origins.get(screen).copied().unwrap_or_default();
        input::ink_point(s.x, s.y, origin, s.pressure, s.tilt, s.time_100ns)
    }

    /// Handles one of the layer's events; drawing events change the sessions, to be shown at
    /// the next [`Ink::flush`]. `None` for anything the app needn't act on.
    pub fn handle(&mut self, event: OverlayEvent, settings: &DrawingSettings) -> Option<Outcome> {
        if self.sessions.is_empty() {
            return None;
        }
        match event {
            OverlayEvent::Pointer {
                screen,
                phase,
                sample,
                shift,
            } => self.pointer(screen, phase, sample, shift),
            OverlayEvent::Key {
                vk,
                ctrl,
                shift,
                alt,
            } => self.key(vk, ctrl, shift, alt, settings),
            OverlayEvent::KeyUp(vk) => {
                if self.held == Some(vk) {
                    self.held = None;
                }
                None
            }
            OverlayEvent::Char(c) => {
                if let Some(i) = self.typing()
                    && !c.is_control()
                {
                    let mut buf = [0u8; 4];
                    let area = self.sessions[i].text_insert(c.encode_utf8(&mut buf));
                    self.mark(i, area);
                }
                None
            }
            OverlayEvent::Wheel { delta, .. } => {
                self.wheel(delta, settings);
                None
            }
            _ => None,
        }
    }

    fn pointer(
        &mut self,
        screen: usize,
        phase: Phase,
        sample: Sample,
        shift: bool,
    ) -> Option<Outcome> {
        if screen >= self.sessions.len() {
            return None;
        }
        let lifting = phase == Phase::Up || phase == Phase::Leave || !sample.in_contact;
        if self.contact.is_some_and(|c| c.pointer != sample.id) {
            // Another pointer while one touches: it waits for that one to lift.
            return None;
        }
        match self.contact {
            None if !lifting => return self.down(screen, sample, shift),
            Some(c) if lifting => self.up(c, sample, shift),
            Some(c) => {
                if !c.over {
                    let p = self.point(c.screen, &sample);
                    let area = self.sessions[c.screen].pointer_move(p, shift);
                    self.mark(c.screen, area);
                    self.pending.push(sample.time_100ns);
                    self.contact = Some(Contact {
                        last: p,
                        shift,
                        ..c
                    });
                }
            }
            None => self.hover(screen, phase, sample),
        }
        None
    }

    fn down(&mut self, screen: usize, sample: Sample, shift: bool) -> Option<Outcome> {
        let p = self.point(screen, &sample);
        // A click finishes the text being typed, here or on another monitor: its own step.
        for i in 0..self.sessions.len() {
            if self.sessions[i].editing_text().is_some() {
                let before = self.marks();
                let area = self.sessions[i].text_end(p.time_ms);
                self.mark(i, area);
                self.record_since(&before);
            }
        }
        self.hide_laser_except(Some(screen));
        let tool = self.tool();
        let restore = (sample.eraser && tool != ToolKind::Eraser).then(|| {
            self.sessions[screen].tool = ToolKind::Eraser;
            tool
        });
        let before = self.marks();
        let result = self.sessions[screen].pointer_down(p, shift);
        let mut contact = Contact {
            pointer: sample.id,
            screen,
            over: false,
            restore,
            last: p,
            shift,
        };
        self.pending.push(sample.time_100ns);
        let outcome = match result {
            Ok(area) => {
                self.mark(screen, area);
                // A stamp is its step as it touches down; an eraser's is the whole drag, its
                // lift compared with what was there before it touched.
                if self.sessions[screen].tool == ToolKind::Stamp {
                    self.record_since(&before);
                    self.contact_marks = self.marks();
                } else {
                    self.contact_marks = before;
                }
                if tool == ToolKind::Laser {
                    self.laser_on = Some(screen);
                }
                None
            }
            Err(e) => {
                contact.over = true;
                Some(refused(e))
            }
        };
        self.contact = Some(contact);
        outcome
    }

    fn up(&mut self, c: Contact, sample: Sample, shift: bool) {
        if !c.over {
            let p = self.point(c.screen, &sample);
            let area = self.sessions[c.screen].pointer_up(p, shift);
            self.mark(c.screen, area);
            let before = std::mem::take(&mut self.contact_marks);
            self.record_since(&before);
            self.pending.push(sample.time_100ns);
        }
        self.lifted(c);
    }

    fn lifted(&mut self, c: Contact) {
        if let Some(t) = c.restore {
            self.sessions[c.screen].tool = t;
        }
        self.contact = None;
    }

    /// Ends a contact still down (a key changes the tool, the backdrop or the drawing): as if
    /// it lifted where it last was; the rest of it draws nothing.
    fn end_contact(&mut self) {
        let Some(c) = self.contact else {
            return;
        };
        if !c.over {
            let area = self.sessions[c.screen].pointer_up(c.last, c.shift);
            self.mark(c.screen, area);
            let before = std::mem::take(&mut self.contact_marks);
            self.record_since(&before);
        }
        if let Some(t) = c.restore {
            self.sessions[c.screen].tool = t;
        }
        self.contact = Some(Contact {
            over: true,
            restore: None,
            ..c
        });
    }

    fn hover(&mut self, screen: usize, phase: Phase, sample: Sample) {
        if self.tool() != ToolKind::Laser {
            return;
        }
        if phase == Phase::Leave {
            self.sessions[screen].laser_hide();
            return;
        }
        self.hide_laser_except(Some(screen));
        let p = self.point(screen, &sample);
        self.sessions[screen].pointer_move(p, false);
        self.laser_on = Some(screen);
        self.pending.push(sample.time_100ns);
    }

    fn hide_laser_except(&mut self, keep: Option<usize>) {
        if let Some(on) = self.laser_on
            && Some(on) != keep
        {
            self.sessions[on].laser_hide();
            self.laser_on = None;
        }
    }

    fn key(
        &mut self,
        vk: u32,
        ctrl: bool,
        shift: bool,
        alt: bool,
        settings: &DrawingSettings,
    ) -> Option<Outcome> {
        let typing = self.typing();
        if !ctrl && !alt && typing.is_none() && input::tool_for_key(vk).is_some() {
            self.held = Some(vk);
        }
        let now = self.now();
        match input::key_action(vk, ctrl, shift, alt, typing.is_some()) {
            KeyAction::Tool(t) => return self.pick_tool(t, settings),
            KeyAction::Colour(i) => {
                if let Some(c) = settings.palette.get(i) {
                    self.colour = Some(*c);
                    self.apply(settings);
                    // Text being typed keeps its colour; the caret shows the next one.
                }
            }
            KeyAction::Undo => {
                self.prune_all(settings);
                self.undo();
            }
            KeyAction::Redo => {
                self.prune_all(settings);
                self.redo();
            }
            KeyAction::Clear => self.clear(now),
            KeyAction::ToggleFreeze => return Some(Outcome::ToggleFreeze),
            KeyAction::Backdrop { back } => return Some(Outcome::Backdrop { back }),
            KeyAction::Guides { back } => return Some(Outcome::Guides { back }),
            KeyAction::Close => return Some(Outcome::Close),
            KeyAction::Text(k) => {
                if let Some(i) = typing {
                    self.text_key(i, k, now);
                }
            }
            KeyAction::None => {}
        }
        None
    }

    /// A toolbar control's command. Text being typed is finished first where the command
    /// leaves it (a tool, Freeze, Clear), as a click elsewhere would; Undo drops it, as Ctrl+Z
    /// does; a colour is for what's drawn next. Close needs nothing: the drawing goes.
    pub fn command(&mut self, command: Command, settings: &DrawingSettings) -> Option<Outcome> {
        if self.sessions.is_empty() {
            return None;
        }
        let now = self.now();
        match command {
            Command::Tool(t) => {
                self.finish_typing(now);
                return self.pick_tool(t, settings);
            }
            Command::Colour(i) => {
                if let Some(c) = settings.palette.get(i) {
                    self.colour = Some(*c);
                    self.apply(settings);
                }
            }
            Command::Undo => {
                self.prune_all(settings);
                self.undo();
            }
            Command::Redo => {
                self.prune_all(settings);
                self.redo();
            }
            Command::Clear => self.clear(now),
            Command::Close => return Some(Outcome::Close),
            Command::ToggleFreeze => {
                self.finish_typing(now);
                return Some(Outcome::ToggleFreeze);
            }
        }
        None
    }

    /// Finishes the text being typed, if any, as its own step.
    fn finish_typing(&mut self, now: u64) {
        if let Some(i) = self.typing() {
            let before = self.marks();
            let area = self.sessions[i].text_end(now);
            self.mark(i, area);
            self.record_since(&before);
        }
    }

    fn pick_tool(&mut self, t: ToolKind, settings: &DrawingSettings) -> Option<Outcome> {
        let backdrop = self.sessions.first().map_or(Backdrop::Live, |s| s.backdrop);
        if t == ToolKind::Blur && !backdrop.allows_blur() {
            return Some(refused(ToolError::BlurNeedsFrozen));
        }
        if t == self.tool() {
            return None;
        }
        self.end_contact();
        if t != ToolKind::Laser {
            self.hide_laser_except(None);
        }
        self.tool = Some(t);
        self.apply(settings);
        tracing::info!(tool = t.as_str(), "tool");
        None
    }

    fn wheel(&mut self, delta: i32, settings: &DrawingSettings) {
        let Some(t) = self.held.and_then(input::tool_for_key) else {
            return;
        };
        let Some(w) = settings
            .widths
            .get(t)
            .map(|_| self.width_of(t, &settings.widths))
        else {
            return;
        };
        self.set_width_of(t, Widths::stepped(w, delta as f32 / 120.0), settings);
    }

    /// The current tool's width set from the toolbar's slider (Settings' pixels, kept in
    /// [`Widths::RANGE`]): as the wheel sets it, for this tool until Settings changes it.
    /// Nothing for a tool with no width.
    pub fn set_width(&mut self, width: f32, settings: &DrawingSettings) {
        let t = self.tool();
        if settings.widths.get(t).is_some() {
            self.set_width_of(t, Widths::stepped(width, 0.0), settings);
        }
    }

    fn set_width_of(&mut self, t: ToolKind, w: f32, settings: &DrawingSettings) {
        self.widths.retain(|(k, _)| *k != t);
        self.widths.push((t, w));
        self.apply(settings);
        tracing::info!(tool = t.as_str(), width = w, "width");
    }

    fn undo(&mut self) {
        // A stroke, shape or erase still under way is dropped instead.
        if let Some(c) = self.contact
            && !c.over
            && (self.sessions[c.screen].is_drawing()
                || self.sessions[c.screen].tool == ToolKind::Eraser)
        {
            let area = self.sessions[c.screen].undo();
            self.mark(c.screen, area);
            self.contact = Some(Contact { over: true, ..c });
            self.sync();
            return;
        }
        // So is text being typed.
        if let Some(i) = self.typing() {
            let area = self.sessions[i].undo();
            self.mark(i, area);
            return;
        }
        for i in self.history.undo().unwrap_or_default() {
            if let Some(s) = self.sessions.get_mut(i) {
                let area = s.undo();
                self.mark(i, area);
            }
        }
        self.sync();
    }

    fn redo(&mut self) {
        if self.contact.is_some_and(|c| !c.over) || self.typing().is_some() {
            return;
        }
        for i in self.history.redo().unwrap_or_default() {
            if let Some(s) = self.sessions.get_mut(i) {
                let area = s.redo();
                self.mark(i, area);
            }
        }
        self.sync();
    }

    fn clear(&mut self, now: u64) {
        self.end_contact();
        self.finish_typing(now);
        let before = self.marks();
        for i in 0..self.sessions.len() {
            let area = self.sessions[i].clear();
            self.mark(i, area);
        }
        self.record_since(&before);
    }

    fn text_key(&mut self, i: usize, k: TextKey, now: u64) {
        if k == TextKey::Finish {
            let before = self.marks();
            let area = self.sessions[i].text_end(now);
            self.mark(i, area);
            self.record_since(&before);
            return;
        }
        let s = &mut self.sessions[i];
        let area = match k {
            TextKey::Finish => None,
            TextKey::NewLine => s.text_newline(),
            TextKey::Backspace => s.text_backspace(),
            TextKey::Delete => s.text_delete(),
            TextKey::Left => s.text_left(),
            TextKey::Right => s.text_right(),
            TextKey::Home => s.text_home(),
            TextKey::End => s.text_end_of_line(),
        };
        self.mark(i, area);
    }

    /// Shows what changed since the last call on every monitor (and what animates on its own),
    /// redrawing only those rectangles. Returns when to call it again: in a frame while
    /// something fades or the laser's trail shows, when the next stroke starts to fade with
    /// disappearing ink, or `None`: nothing will change by itself.
    pub fn flush(&mut self, overlay: &mut Overlay, settings: &DrawingSettings) -> Wake {
        let now = self.now();
        let fade = fade_ms(settings);
        let trail = settings.laser_trail_ms;
        let laser_colour = self.colour.unwrap_or(Colour::rgb(255, 0, 0));
        let mut wake = Wake::default();
        let mut redrew = false;
        let started = clock::now_secs();
        for i in 0..self.sessions.len() {
            let full = std::mem::take(&mut self.full[i]);
            let dirty = self.dirty[i].take();
            let s = &self.sessions[i];
            let laser = s.laser();
            let options = RenderOptions {
                highlight: Highlight::Tint,
                now_ms: now,
                disappearing_ink_ms: fade,
                laser: Some(LaserLook {
                    laser,
                    colour: laser_colour,
                    trail_ms: trail,
                }),
                ..Default::default()
            };
            // What fades, and the laser while its trail shows or is left to clear (a dot
            // held still needs no frames).
            let fading = render::animated_area(
                s.drawing(),
                &RenderOptions {
                    laser: None,
                    ..options
                },
            );
            let trailing = (!laser.visible_trail(now, trail).is_empty()
                || laser.painted_bounds() != laser.bounds(now, trail))
            .then(|| laser.painted_bounds())
            .flatten();
            let moving = union(fading, trailing);
            if full {
                overlay.redraw(i, s.drawing(), &options, None);
                redrew = true;
            } else if let Some(area) = union(dirty, moving) {
                overlay.redraw(i, s.drawing(), &options, Some(area));
                redrew = true;
            }
            let caret = s.text_caret();
            let c = s.colour;
            overlay.set_caret(i, caret, [c.r, c.g, c.b, 255]);
            if let Some(delay) = fade {
                wake.next_ms = min_opt(wake.next_ms, next_fade(s, now, delay));
            }
            if moving.is_some() {
                wake.animating = true;
                if let Some(delay) = fade {
                    let gone = self.prune(i, now, delay);
                    self.dirty[i] = union(self.dirty[i], gone);
                }
                self.sessions[i].laser_prune(now, trail);
            }
        }
        if wake.animating {
            self.sync();
        }
        wake.animating |= self.dirty.iter().any(Option::is_some);
        if redrew {
            let shown = clock::now_100ns();
            if self.redraw_ms.len() < MAX_TIMINGS {
                self.redraw_ms
                    .push(((clock::now_secs() - started) * 1000.0) as f32);
            }
            for t in self.pending.drain(..) {
                if self.latency_ms.len() < MAX_TIMINGS {
                    self.latency_ms.push((shown - t) as f32 / 10_000.0);
                }
            }
        }
        wake
    }
}

/// When [`Ink::flush`] wants calling again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wake {
    /// Something moves: in a frame's time.
    pub animating: bool,
    /// Otherwise, in this many milliseconds (the next fade's start); `None`: only on input.
    pub next_ms: Option<u32>,
}

impl Wake {
    /// The loop's wait, given a frame's length.
    pub fn timeout(self, frame_ms: u32) -> Option<u32> {
        if self.animating {
            Some(frame_ms)
        } else {
            self.next_ms
        }
    }
}

fn min_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// How long until the next of `s`'s items starts to fade (`delay` after it was drawn), in
/// milliseconds: a little early, as [`render::animated_area`] starts a frame or so before.
fn next_fade(s: &Session, now: u64, delay: u64) -> Option<u32> {
    s.drawing()
        .items()
        .filter(|item| item.finished && item.tool != ToolKind::Blur)
        .map(|item| item.drawn_at() + delay)
        .filter(|&start| start > now + 50)
        .map(|start| (start - now - 40).min(u64::from(u32::MAX)) as u32)
        .min()
}

fn refused(e: ToolError) -> Outcome {
    tracing::info!(error = ?e, "tool refused");
    Outcome::Notice(e.to_string())
}

/// Logs the median, 95th percentile and worst of `ms`, and empties it.
fn log_timings(what: &str, ms: &mut Vec<f32>) {
    if ms.is_empty() {
        return;
    }
    ms.sort_by(f32::total_cmp);
    let at = |q: f32| ms[((ms.len() - 1) as f32 * q).round() as usize];
    tracing::info!(
        what,
        count = ms.len(),
        median_ms = at(0.5),
        p95_ms = at(0.95),
        max_ms = at(1.0),
        "timings"
    );
    ms.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use overmark_win::win32::pointer::PointerKind;

    /// `n` monitors side by side, 2000 px apart, without an overlay.
    fn ink(n: usize) -> Ink {
        let mut ink = Ink {
            sessions: (0..n).map(|_| Session::new()).collect(),
            origins: (0..n).map(|i| (i as i32 * 2000, 0)).collect(),
            scales: vec![1.0; n],
            dirty: vec![None; n],
            full: vec![false; n],
            ..Ink::default()
        };
        ink.apply(&DrawingSettings::default());
        ink
    }

    fn sample(x: i32, y: i32, contact: bool, eraser: bool) -> Sample {
        sample_at(x, y, contact, eraser, 0)
    }

    /// A sample at `t` ms on the drawing's clock.
    fn sample_at(x: i32, y: i32, contact: bool, eraser: bool, t: i64) -> Sample {
        Sample {
            id: 1,
            kind: if eraser {
                PointerKind::Pen
            } else {
                PointerKind::Mouse
            },
            x,
            y,
            pressure: eraser.then_some(0.5),
            tilt: None,
            eraser,
            in_contact: contact,
            time_100ns: t * 10_000,
        }
    }

    /// A drag along `points` on `screen` (layer pixels), at `t` ms.
    fn drag_at(ink: &mut Ink, screen: usize, points: &[(i32, i32)], eraser: bool, t: i64) {
        let x0 = screen as i32 * 2000;
        let n = points.len();
        for (k, &(x, y)) in points.iter().enumerate() {
            let phase = match k {
                0 => Phase::Down,
                _ if k + 1 == n => Phase::Up,
                _ => Phase::Move,
            };
            let event = OverlayEvent::Pointer {
                screen,
                phase,
                sample: sample_at(x0 + x, y, k + 1 < n, eraser, t),
                shift: false,
            };
            ink.handle(event, &DrawingSettings::default());
        }
    }

    fn drag(ink: &mut Ink, screen: usize, y: i32, eraser: bool) {
        let x0 = screen as i32 * 2000 + 100;
        let s = &DrawingSettings::default();
        for (k, (x, yy)) in [(x0, y), (x0 + 50, y), (x0 + 100, y)]
            .into_iter()
            .enumerate()
        {
            let phase = [Phase::Down, Phase::Move, Phase::Up][k];
            let event = OverlayEvent::Pointer {
                screen,
                phase,
                sample: sample(x, yy, k < 2, eraser),
                shift: false,
            };
            ink.handle(event, s);
        }
    }

    /// An eraser's drag across the strokes at x 150 on `screen`, from y0 to y1.
    fn erase(ink: &mut Ink, screen: usize, y0: i32, y1: i32) {
        let x = screen as i32 * 2000 + 150;
        let s = &DrawingSettings::default();
        let points = [
            (y0, Phase::Down, true),
            ((y0 + y1) / 2, Phase::Move, true),
            (y1, Phase::Up, false),
        ];
        for (y, phase, contact) in points {
            let event = OverlayEvent::Pointer {
                screen,
                phase,
                sample: sample(x, y, contact, true),
                shift: false,
            };
            ink.handle(event, s);
        }
    }

    fn key(ink: &mut Ink, vk: u32, ctrl: bool, shift: bool) {
        key_in(ink, vk, ctrl, shift, &DrawingSettings::default());
    }

    fn key_in(ink: &mut Ink, vk: u32, ctrl: bool, shift: bool, settings: &DrawingSettings) {
        let event = OverlayEvent::Key {
            vk,
            ctrl,
            shift,
            alt: false,
        };
        ink.handle(event, settings);
    }

    /// Disappearing ink after 1 s.
    fn fading() -> DrawingSettings {
        DrawingSettings {
            disappearing_ink: true,
            disappearing_ink_seconds: 1.0,
            ..DrawingSettings::default()
        }
    }

    /// What `flush` prunes at `now`: only the monitors with something fading on screen.
    fn frame(ink: &mut Ink, now: u64, delay: u64) {
        for i in 0..ink.sessions.len() {
            let options = RenderOptions {
                now_ms: now,
                disappearing_ink_ms: Some(delay),
                ..Default::default()
            };
            if render::animated_area(ink.sessions[i].drawing(), &options).is_some() {
                ink.prune(i, now, delay);
            }
        }
    }

    fn undo(ink: &mut Ink) {
        key(ink, 'Z' as u32, true, false);
    }

    fn counts(ink: &Ink) -> Vec<usize> {
        ink.sessions
            .iter()
            .map(|s| s.drawing().items().len())
            .collect()
    }

    #[test]
    fn another_pointer_hovering_meanwhile_does_not_end_a_stroke() {
        let mut ink = ink(1);
        let s = DrawingSettings::default();
        let at = |x, y, contact, id| {
            let mut sample = sample(x, y, contact, false);
            sample.id = id;
            sample
        };
        let ev = |phase, sample| OverlayEvent::Pointer {
            screen: 0,
            phase,
            sample,
            shift: false,
        };
        ink.handle(ev(Phase::Down, at(100, 100, true, 7)), &s);
        ink.handle(ev(Phase::Move, at(150, 100, true, 7)), &s);
        // The mouse (another pointer) moves over the layer, not touching.
        ink.handle(ev(Phase::Move, at(900, 900, false, 1)), &s);
        ink.handle(ev(Phase::Leave, at(0, 0, false, 1)), &s);
        ink.handle(ev(Phase::Move, at(200, 100, true, 7)), &s);
        ink.handle(ev(Phase::Up, at(250, 100, false, 7)), &s);
        assert_eq!(counts(&ink), vec![1]);
        assert_eq!(ink.sessions[0].undo_steps(), 1, "one stroke, not split");
        let b = ink.sessions[0]
            .drawing()
            .items()
            .next()
            .map(|i| i.points.len());
        assert_eq!(b, Some(4), "every sample of the pen's, none of the mouse's");
    }

    #[test]
    fn at_the_limit_ctrl_z_still_undoes_the_newest_stroke_wherever_it_was() {
        let mut ink = ink(2);
        for k in 0..100 {
            drag(&mut ink, 0, 10 + k * 5, false);
        }
        drag(&mut ink, 1, 10, false);
        drag(&mut ink, 0, 700, false);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![100, 1]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![100, 0]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![99, 0]);
        key(&mut ink, 'Y' as u32, true, false);
        key(&mut ink, 'Z' as u32, true, true);
        assert_eq!(counts(&ink), vec![100, 1]);
    }

    #[test]
    fn at_the_limit_a_clear_of_every_monitor_undoes_as_one() {
        let mut ink = ink(2);
        for k in 0..100 {
            drag(&mut ink, 0, 10 + k * 5, false);
        }
        drag(&mut ink, 1, 10, false);
        key(&mut ink, 0x2E, false, false); // Delete
        assert_eq!(counts(&ink), vec![0, 0]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![100, 1]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![100, 0]);
    }

    #[test]
    fn an_eraser_drag_and_the_pens_eraser_end_are_one_step_each() {
        let mut ink = ink(1);
        drag(&mut ink, 0, 100, false);
        drag(&mut ink, 0, 200, false);
        drag(&mut ink, 0, 300, false);
        // The pen's eraser end through the first two; the tool comes back after.
        erase(&mut ink, 0, 90, 210);
        assert_eq!(counts(&ink), vec![1]);
        assert_eq!(ink.sessions[0].tool, ToolKind::Pen);
        // The eraser tool through the last.
        key(&mut ink, 'X' as u32, false, false);
        let event = |phase, y, contact| OverlayEvent::Pointer {
            screen: 0,
            phase,
            sample: sample(150, y, contact, false),
            shift: false,
        };
        let s = DrawingSettings::default();
        ink.handle(event(Phase::Down, 290, true), &s);
        ink.handle(event(Phase::Move, 310, true), &s);
        ink.handle(event(Phase::Up, 310, false), &s);
        assert_eq!(counts(&ink), vec![0]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![1]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![3]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![2]);
    }

    #[test]
    fn text_finished_by_a_click_is_its_own_step_before_the_stamp() {
        let mut ink = ink(2);
        let s = DrawingSettings::default();
        key(&mut ink, 'T' as u32, false, false);
        let click = |ink: &mut Ink, screen: usize, x: i32| {
            for (phase, contact) in [(Phase::Down, true), (Phase::Up, false)] {
                let event = OverlayEvent::Pointer {
                    screen,
                    phase,
                    sample: sample(x, 100, contact, false),
                    shift: false,
                };
                ink.handle(event, &DrawingSettings::default());
            }
        };
        click(&mut ink, 0, 100);
        ink.handle(OverlayEvent::Char('a'), &s);
        // A stamp on the other monitor finishes the text first.
        ink.held = None;
        ink.tool = Some(ToolKind::Stamp);
        ink.apply(&s);
        click(&mut ink, 1, 2100);
        assert_eq!(counts(&ink), vec![1, 1]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![1, 0]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![0, 0]);
    }

    #[test]
    fn a_faded_old_item_and_its_newer_erase_leave_the_order_of_what_remains() {
        // A and C on monitor 0, B on monitor 1, then A erased; A fades
        // and its session drops both its add and the erase, from the middle of its history.
        let mut ink = ink(2);
        let flat = |y: i32| [(100, y), (150, y), (200, y)];
        drag_at(&mut ink, 0, &flat(100), false, 0); // A
        drag_at(&mut ink, 0, &flat(300), false, 1000); // C
        drag_at(&mut ink, 1, &flat(100), false, 1100); // B
        drag_at(
            &mut ink,
            0,
            &[(150, 90), (150, 100), (150, 110)],
            true,
            1200,
        ); // A erased
        assert_eq!(counts(&ink), vec![1, 1]);
        let delay = 1000;
        for i in 0..2 {
            ink.prune(i, 2200, delay);
        }
        assert_eq!(ink.sessions[0].undo_steps(), 1, "A's add and erase dropped");
        // C and B are still showing; B is the newer: Ctrl+Z undoes it first.
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![1, 0]);
        undo(&mut ink);
        assert_eq!(counts(&ink), vec![0, 0]);
        // And redo brings them back in their order: C, then B.
        key(&mut ink, 'Y' as u32, true, false);
        assert_eq!(counts(&ink), vec![1, 0]);
        key(&mut ink, 'Y' as u32, true, false);
        assert_eq!(counts(&ink), vec![1, 1]);
    }

    #[test]
    fn a_faded_undone_step_leaves_the_redo_order_of_the_rest() {
        let mut ink = ink(2);
        let flat = |y: i32| [(100, y), (150, y), (200, y)];
        drag_at(&mut ink, 0, &flat(100), false, 0); // A, monitor 0
        drag_at(&mut ink, 1, &flat(100), false, 1000); // B, monitor 1
        drag_at(&mut ink, 0, &flat(300), false, 1100); // C, monitor 0
        for _ in 0..3 {
            undo(&mut ink);
        }
        assert_eq!(counts(&ink), vec![0, 0]);
        // A (undone, oldest) fades away while held by the redo side.
        for i in 0..2 {
            ink.prune(i, 1700, 1000);
        }
        key(&mut ink, 'Y' as u32, true, false);
        assert_eq!(counts(&ink), vec![0, 1], "B first: A is gone");
        key(&mut ink, 'Y' as u32, true, false);
        assert_eq!(counts(&ink), vec![1, 1]);
    }

    #[test]
    fn pruning_one_monitor_leaves_another_monitors_history_alone() {
        // A on monitor 1, B on 0, C on 1, A erased on 1. At 1.7 s
        // B is fading on monitor 0, A has faded (held by the erase) and C shows on monitor 1:
        // a frame prunes monitor 0 only.
        let mut ink = ink(2);
        let flat = |y: i32| [(100, y), (150, y), (200, y)];
        drag_at(&mut ink, 1, &flat(100), false, 0); // A
        drag_at(&mut ink, 0, &flat(100), false, 500); // B
        drag_at(&mut ink, 1, &flat(300), false, 1000); // C
        drag_at(
            &mut ink,
            1,
            &[(150, 90), (150, 100), (150, 110)],
            true,
            1200,
        ); // A erased
        frame(&mut ink, 1700, 1000);
        assert_eq!(ink.sessions[1].undo_steps(), 3, "monitor 1 wasn't pruned");
        let parts_on_1 = ink.history.ids().filter(|(s, _)| *s == 1).count();
        assert_eq!(parts_on_1, 3, "nor were its parts of the history");
        // Ctrl+Z prunes first: A and its erase go from monitor 1, then C, the newest, is undone.
        ink.fixed_now = Some(1700);
        key_in(&mut ink, 'Z' as u32, true, false, &fading());
        assert_eq!(counts(&ink), vec![1, 0]);
        key_in(&mut ink, 'Z' as u32, true, false, &fading());
        assert_eq!(counts(&ink), vec![0, 0]);
        key_in(&mut ink, 'Y' as u32, true, false, &fading());
        assert_eq!(counts(&ink), vec![1, 0]);
        key_in(&mut ink, 'Y' as u32, true, false, &fading());
        assert_eq!(counts(&ink), vec![1, 1]);
    }

    #[test]
    fn what_has_faded_away_is_forgotten_not_kept() {
        let mut ink = ink(1);
        let delay = 1000;
        let gone_after = (delay + render::FADE_MS) as i64 + 1;
        for k in 0..200i64 {
            let t = k * 10_000;
            drag_at(&mut ink, 0, &[(100, 100), (150, 100), (200, 100)], false, t);
            frame(&mut ink, (t + gone_after) as u64, delay);
            assert!(
                ink.seen.len() <= 1,
                "{} kept after {k} strokes",
                ink.seen.len()
            );
        }
        assert_eq!(counts(&ink), vec![0]);
        assert_eq!(ink.sessions[0].undo_steps(), 0);
        assert!(ink.seen.is_empty());
        assert_eq!(ink.history.ids().count(), 0);
    }

    /// The last item drawn on monitor 0: its tool, colour and width.
    fn last_item(ink: &Ink) -> (ToolKind, Colour, f32) {
        let item = ink.sessions[0].drawing().items().last().expect("an item");
        (item.tool, item.colour, item.width)
    }

    /// Settings saved while the layer is open: the next stroke has the new width, even after
    /// the wheel set one, and what's drawn stays.
    #[test]
    fn a_width_changed_in_settings_is_the_next_strokes() {
        let mut old = DrawingSettings::default();
        old.widths.pen = 2.0;
        let mut ink = ink(1);
        ink.apply(&old);
        drag(&mut ink, 0, 100, false);
        assert_eq!(last_item(&ink).2, 2.0);
        let mut new = old.clone();
        new.widths.pen = 12.0;
        ink.settings_changed(&old, &new);
        drag(&mut ink, 0, 200, false);
        assert_eq!(last_item(&ink).2, 12.0);
        assert_eq!(
            ink.sessions[0].drawing().items().count(),
            2,
            "the drawing stays"
        );

        // A width set with the wheel stays until Settings changes that tool's width.
        ink.widths.push((ToolKind::Pen, 30.0));
        let mut other = new.clone();
        other.widths.highlighter = 40.0;
        ink.settings_changed(&new, &other);
        assert_eq!(ink.sessions[0].width, 30.0);
        let mut pen = other.clone();
        pen.widths.pen = 6.0;
        ink.settings_changed(&other, &pen);
        assert_eq!(ink.sessions[0].width, 6.0);
    }

    /// A new starting tool or colour is picked at once and kept for the next opening; a
    /// palette colour changed while in use follows the change; other changes leave the tool
    /// and colour picked meanwhile as they are.
    #[test]
    fn starting_tool_and_colour_follow_settings() {
        let old = DrawingSettings::default();
        let mut ink = ink(1);
        ink.tool = Some(ToolKind::Pen);
        ink.colour = Some(old.palette[2]);
        ink.apply(&old);

        let mut new = old.clone();
        new.disappearing_ink = true;
        ink.settings_changed(&old, &new);
        assert_eq!(
            (ink.tool, ink.colour),
            (Some(ToolKind::Pen), Some(old.palette[2])),
            "nothing of theirs changed"
        );

        let mut recoloured = new.clone();
        recoloured.palette[2] = Colour::rgb(1, 2, 3);
        ink.settings_changed(&new, &recoloured);
        assert_eq!(ink.colour, Some(Colour::rgb(1, 2, 3)));

        let mut start = recoloured.clone();
        start.default_tool = ToolKind::Highlighter;
        start.default_colour = 4;
        ink.settings_changed(&recoloured, &start);
        assert_eq!(ink.tool, Some(ToolKind::Highlighter));
        assert_eq!(ink.colour, Some(start.palette[4]));
        drag(&mut ink, 0, 100, false);
        let (tool, colour, _) = last_item(&ink);
        assert_eq!((tool, colour), (ToolKind::Highlighter, start.palette[4]));
    }

    /// A finished blur stays still, the drawing settings don't
    /// change, and only the guides' spacing (or whether shares see them) does: every layer is
    /// drawn again, so the blur is made over the new guides. A board hotkey alone draws
    /// nothing again.
    #[test]
    fn a_guide_change_alone_redraws_a_still_blur() {
        use overmark_core::settings::Settings;
        let mut ink = ink(1);
        ink.sessions[0].backdrop = Backdrop::Frozen;
        let old = Settings::default();
        let mut blur = old.drawing.clone();
        blur.default_tool = ToolKind::Blur;
        ink.settings_changed(&old.drawing, &blur);
        // A blur covers the box its drag spans.
        drag_at(&mut ink, 0, &[(100, 100), (150, 150), (200, 200)], false, 0);
        let item = ink.sessions[0].drawing().items().last().expect("the blur");
        assert_eq!(item.tool, ToolKind::Blur);
        ink.full.iter_mut().for_each(|f| *f = false);
        ink.dirty.iter_mut().for_each(|d| *d = None);

        let mut hotkey = old.clone();
        hotkey.backdrops.hotkey_whiteboard = "Win+Shift+W".into();
        assert!(!ink.look_changed(&old, &hotkey));
        assert_eq!(ink.full, [false], "nothing showing changed");

        let mut spacing = old.clone();
        spacing.guides.spacing = 64;
        assert!(ink.look_changed(&old, &spacing));
        assert_eq!(ink.full, [true], "the still blur is drawn again");

        ink.full[0] = false;
        let mut shares = old.clone();
        shares.guides.show_on_shares = false;
        assert!(ink.look_changed(&old, &shares));
        assert_eq!(ink.full, [true]);

        ink.full[0] = false;
        let mut board = old.clone();
        board.backdrops.blackboard_colour = Colour::rgb(1, 2, 3);
        assert!(ink.look_changed(&old, &board));
        assert_eq!(ink.full, [true]);
    }

    /// Text being typed, "hello", on monitor 0.
    fn typing_hello() -> Ink {
        let mut ink = ink(1);
        let s = DrawingSettings::default();
        key(&mut ink, 'T' as u32, false, false);
        drag_at(&mut ink, 0, &[(100, 100), (100, 100)], false, 0);
        for c in "hello".chars() {
            ink.handle(OverlayEvent::Char(c), &s);
        }
        assert!(ink.typing().is_some());
        ink
    }

    #[test]
    fn toolbar_commands_act_while_text_is_typed() {
        let s = DrawingSettings::default();
        // A tool: the text is finished (a step of its own), and the tool is picked.
        let mut ink = typing_hello();
        assert_eq!(ink.command(Command::Tool(ToolKind::Pen), &s), None);
        assert_eq!(ink.tool(), ToolKind::Pen);
        assert!(ink.typing().is_none());
        assert_eq!(ink.sessions[0].undo_steps(), 1);
        // Freeze: asked for (the text finished first).
        let mut ink = typing_hello();
        assert_eq!(
            ink.command(Command::ToggleFreeze, &s),
            Some(Outcome::ToggleFreeze)
        );
        assert!(ink.typing().is_none());
        // Close: the layer closes, not just the text.
        let mut ink = typing_hello();
        assert_eq!(ink.command(Command::Close, &s), Some(Outcome::Close));
        // Clear: the drawing is cleared (the text with it), not a character deleted.
        let mut ink = typing_hello();
        ink.command(Command::Clear, &s);
        assert_eq!(counts(&ink), vec![0]);
        ink.command(Command::Undo, &s);
        assert_eq!(counts(&ink), vec![1], "Clear undoes, the text back");
    }

    #[test]
    fn a_tool_from_the_toolbar_leaves_no_key_held() {
        let s = DrawingSettings::default();
        let mut ink = ink(1);
        ink.command(Command::Tool(ToolKind::Pen), &s);
        let before = ink.sessions[0].width;
        // A plain wheel turn, no key held: the width stays.
        ink.handle(
            OverlayEvent::Wheel {
                screen: 0,
                delta: 120,
            },
            &s,
        );
        assert_eq!(ink.sessions[0].width, before);
        assert_eq!(ink.held, None);
    }

    /// The toolbar's slider sets the width as holding the tool's key and scrolling does: the
    /// same width for the next stroke and the toolbar, kept in the same range, for that tool.
    #[test]
    fn the_toolbars_slider_sets_the_width_as_the_wheel_does() {
        let s = DrawingSettings::default();
        let pen = s.widths.pen;
        let mut wheel = ink(1);
        let p = ToolKind::Pen.key() as u32;
        wheel.handle(
            OverlayEvent::Key {
                vk: p,
                ctrl: false,
                shift: false,
                alt: false,
            },
            &s,
        );
        wheel.handle(
            OverlayEvent::Wheel {
                screen: 0,
                delta: 120,
            },
            &s,
        );
        wheel.handle(OverlayEvent::KeyUp(p), &s);
        let mut slider = ink(1);
        slider.set_width(Widths::stepped(pen, 1.0), &s);
        assert_eq!(wheel.sessions[0].width, slider.sessions[0].width);
        assert_eq!(wheel.toolbar_view(&s).2, slider.toolbar_view(&s).2);
        assert!((slider.toolbar_view(&s).2.unwrap() - pen * Widths::STEP).abs() < 1e-4);
        // Kept in the range; the highlighter keeps its own.
        slider.set_width(10_000.0, &s);
        assert_eq!(slider.toolbar_view(&s).2, Some(Widths::RANGE.1));
        slider.command(Command::Tool(ToolKind::Highlighter), &s);
        assert_eq!(
            slider.toolbar_view(&s).2,
            s.widths.get(ToolKind::Highlighter)
        );
        // The eraser has none: nothing changes.
        slider.command(Command::Tool(ToolKind::Eraser), &s);
        slider.set_width(50.0, &s);
        assert_eq!(slider.toolbar_view(&s).2, None);
        assert!(slider.widths.iter().all(|(t, _)| *t != ToolKind::Eraser));
    }

    #[test]
    fn turning_disappearing_ink_off_draws_a_half_faded_stroke_whole_again() {
        let fading = fading();
        let mut ink = ink(1);
        drag_at(&mut ink, 0, &[(100, 100), (150, 100), (200, 100)], false, 0);
        // Halfway through its fade (1 s delay, then 600 ms): drawn at half alpha.
        let now = 1300;
        let item = ink.sessions[0].drawing().items().next().unwrap().clone();
        assert!((0.3..0.7).contains(&render::opacity(&item, now, fade_ms(&fading))));
        ink.full = vec![false];
        // Off, as the toolbar's toggle takes it up (Settings' path): every layer is drawn
        // again, so the stroke shows whole, not left at half.
        let off = DrawingSettings {
            disappearing_ink: false,
            ..fading.clone()
        };
        ink.settings_changed(&fading, &off);
        assert_eq!(ink.full, vec![true]);
        assert_eq!(render::opacity(&item, now, fade_ms(&off)), 1.0);
    }
}
