//! The resident app's loop: the tray, the hotkeys, and the overlay while it's open.

mod tray_ui;

use overmark_core::backdrop::Guides;
use overmark_core::hotkey::Hotkey;
use overmark_core::hotkey_status::{State, Status};
use overmark_core::i18n::tr;
use overmark_core::session::Session;
use overmark_core::settings::{PrintScreen, Settings};
use overmark_core::tr;
use overmark_win::hotkeys::{Action, Hotkeys, RegisterError};
use overmark_win::overlay::{
    Backdrop, Command, Look, Overlay, OverlayEvent, freeze_toggled, log_name,
};
use overmark_win::save;
use overmark_win::win32::shell;
use overmark_win::win32::toast::Toast;
use overmark_win::win32::tray::{self, BalloonKind, Instance, SingleInstance, Tray, TrayEvent};
use overmark_win::win32::{clock, dpi, language};

use crate::APP_ID;
use crate::config::Startup;
use crate::ink::{Ink, Outcome, Wake};
use crate::settings_window::Page;
use crate::toolbar::{self, Toolbar};
use overmark_win::logging;
use overmark_win::paths::{AppPaths, Mode};
use tray_ui::{BalloonAction, TrayUi};

/// The command line.
#[derive(Debug, Clone, Default)]
pub struct Args {
    /// Serve clean grabs for the app's overlay ([`overmark_win::grabber`]), then exit.
    pub grabber: bool,
    /// Open the layer Frozen (when another instance is running, ask it to).
    pub frozen: bool,
    /// Ask the running instance to quit, then exit.
    pub quit: bool,
    /// The dim's level for this run (0 to 0.8), instead of the settings': for comparing it by
    /// eye with Windows' snip dim (Settings changes the saved level).
    pub dim: Option<f32>,
    /// Draw two known strokes when the layer opens (for `capture_check`, exact by design): see
    /// [`TEST_PEN_Y`].
    pub test_drawing: bool,
    /// Make clean grabs without the grabber helper, by the GDI fallback (for checks of it).
    pub no_grabber: bool,
    /// Be the Settings window (`--settings`, on About with `--about`), not the tray.
    pub settings: Option<Page>,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Args {
        let mut out = Args::default();
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--dim" => match args.next().and_then(|v| v.parse::<f32>().ok()) {
                    Some(v) => out.dim = Some(v.clamp(0.0, 0.8)),
                    None => eprintln!("{}", tr("cli-dim-needs-number")),
                },
                "--frozen" => out.frozen = true,
                "--quit" => out.quit = true,
                "--test-drawing" => out.test_drawing = true,
                "--no-grabber" => out.no_grabber = true,
                "--settings" => out.settings = out.settings.or(Some(Page::General)),
                "--about" => out.settings = Some(Page::About),
                overmark_win::grabber::ARG => out.grabber = true,
                _ => eprintln!(
                    "{}",
                    tr!("cli-unknown-argument", argument = format!("{a:?}"))
                ),
            }
        }
        out
    }
}

/// A frame while something animates (a fade, the laser's trail).
const FRAME_MS: u32 = 16;
/// The hotkey-to-visible budget: logged as a warning when it's missed.
const OPEN_BUDGET_MS: f64 = 100.0;

struct App {
    settings: Settings,
    tray: Tray,
    hotkeys: Option<Hotkeys>,
    /// The hotkeys registered now, in the order Settings lists them, for the tooltip.
    registered: Vec<(Action, Hotkey)>,
    overlay: Option<Overlay>,
    /// The overlay has just opened: make what F needs on the next turn of the loop.
    prepare_pending: bool,
    /// The drawing on every monitor, and the tool, colour and widths.
    ink: Ink,
    /// When the drawing changes by itself (a fade, the laser's trail): come back then.
    wake: Wake,
    /// The user's real Pictures folder, for `{pictures}`.
    pictures: Option<std::path::PathBuf>,
    /// The notice after the last capture.
    toast: Option<Toast>,
    /// A capture waiting for its region.
    pending_region: Option<PendingRegion>,
    /// `--test-drawing`.
    test_drawing: bool,
    /// The guides (G), kept from one opening to the next while Overmark runs.
    guides: Guides,
    /// `--no-grabber`.
    no_grabber: bool,
    /// The tray's clicks, menu, balloons, Settings and update check.
    ui: TrayUi,
    /// The toolbar while the layer is open, and whether it's collapsed (kept while Overmark
    /// runs).
    toolbar: Option<Toolbar>,
    /// The toolbar's egui between openings (made and warmed once).
    toolbar_look: Option<toolbar::Look>,
    toolbar_collapsed: bool,
}

fn action_name(action: Action) -> String {
    match action {
        Action::Draw => tr("tray-draw"),
        Action::DrawFrozen => tr("tray-draw-frozen"),
        Action::DrawWhiteboard => Backdrop::Whiteboard.name(),
        Action::DrawBlackboard => Backdrop::Blackboard.name(),
    }
}

/// The most a tray tooltip holds, in UTF-16 units (`szTip` is 128 with its terminator).
const TOOLTIP_MAX: usize = 127;

/// The tray's tooltip: what Overmark is doing (or its name) on the first line, then a line
/// for each hotkey registered (`shortcuts`: its action and keys). Lines that don't fit in
/// [`TOOLTIP_MAX`] are left out from the end, whole; idle with none, it says to click.
fn tooltip_text(drawing: Option<Backdrop>, shortcuts: &[(String, String)]) -> String {
    let first = match drawing {
        Some(Backdrop::Live) => tr("tray-tooltip-live"),
        Some(Backdrop::Frozen) => tr("tray-tooltip-frozen"),
        Some(b @ (Backdrop::Whiteboard | Backdrop::Blackboard)) => {
            tr!("tray-tooltip-board", backdrop = b.name())
        }
        None if shortcuts.is_empty() => tr("tray-tooltip-no-hotkey"),
        None => tr("app-name"),
    };
    let lines = shortcuts.iter().map(|(action, hotkey)| {
        tr!(
            "tray-tooltip-shortcut",
            action = action.as_str(),
            hotkey = hotkey.as_str()
        )
    });
    fit_lines(first, &tr("app-name"), lines, TOOLTIP_MAX)
}

/// `first`, then each of `lines` on a line of its own while the whole stays within `max`
/// UTF-16 units. No line is ever cut: a first line too long on its own gives way to `name`
/// (or "Overmark", should that be too long as well), and lines are left out from the first
/// that doesn't fit.
fn fit_lines(
    first: String,
    name: &str,
    lines: impl IntoIterator<Item = String>,
    max: usize,
) -> String {
    let units = |s: &str| s.encode_utf16().count();
    let mut text = [first.as_str(), name]
        .into_iter()
        .find(|line| units(line) <= max)
        .unwrap_or("Overmark")
        .to_owned();
    for line in lines {
        if units(&text) + 1 + units(&line) > max {
            break;
        }
        text.push('\n');
        text.push_str(&line);
    }
    text
}

impl App {
    fn tooltip(&self) -> String {
        let shortcuts: Vec<_> = self
            .registered
            .iter()
            .map(|(action, hotkey)| (action_name(*action), hotkey.display()))
            .collect();
        tooltip_text(self.overlay.as_ref().map(Overlay::backdrop), &shortcuts)
    }

    /// Shows the layer's state in the tray: its tooltip and its icon.
    fn refresh_tooltip(&mut self) {
        let tip = self.tooltip();
        if let Err(error) = self.tray.set_tooltip(&tip) {
            tracing::warn!(%error, "couldn't update the tray tooltip");
        }
        self.refresh_icon();
    }

    /// Registers the hotkeys from the settings; each that can't be is logged and told in a
    /// tray notice (never silently), and what became of each is published for
    /// Settings.
    fn register_hotkeys(&mut self) {
        let mut status = Status::default();
        let mut hotkeys = match Hotkeys::new() {
            Ok(h) => h,
            Err(error) => {
                tracing::error!(%error, "no hotkeys: their window couldn't be made");
                return;
            }
        };
        let general = self.settings.general.clone();
        let boards = self.settings.backdrops.clone();
        let mut wanted = vec![
            (
                Action::Draw,
                "general.hotkey_draw",
                general.hotkey_draw.clone(),
            ),
            (
                Action::DrawFrozen,
                "general.hotkey_draw_frozen",
                general.hotkey_draw_frozen.clone(),
            ),
            (
                Action::DrawWhiteboard,
                "backdrops.hotkey_whiteboard",
                boards.hotkey_whiteboard.clone(),
            ),
            (
                Action::DrawBlackboard,
                "backdrops.hotkey_blackboard",
                boards.hotkey_blackboard.clone(),
            ),
        ];
        // Print Screen as an optional binding.
        match general.print_screen {
            PrintScreen::Off => status.set("general.print_screen", State::Unset),
            PrintScreen::Live => {
                wanted.push((Action::Draw, "general.print_screen", "Print Screen".into()))
            }
            PrintScreen::Frozen => wanted.push((
                Action::DrawFrozen,
                "general.print_screen",
                "Print Screen".into(),
            )),
        }
        for (action, key, text) in wanted {
            let hotkey = match Hotkey::parse(&text) {
                Ok(Some(h)) => h,
                Ok(None) => {
                    tracing::info!(?action, key, "no hotkey set");
                    status.set(key, State::Unset);
                    continue;
                }
                Err(error) => {
                    tracing::warn!(?action, %text, %error, "the hotkey setting can't be used");
                    status.set(key, State::Invalid);
                    self.notice_hotkey(key, action, &text);
                    continue;
                }
            };
            if hotkey.is_ctrl_alt() {
                tracing::warn!(?action, hotkey = %hotkey.display(), "a Ctrl+Alt hotkey collides with AltGr");
            }
            match hotkeys.register(action, &hotkey) {
                Ok(()) => {
                    tracing::info!(?action, key, hotkey = %hotkey.display(), "hotkey registered");
                    status.set(key, State::Registered);
                    self.registered.push((action, hotkey));
                }
                Err(RegisterError::Taken) => {
                    tracing::warn!(?action, key, hotkey = %hotkey.display(), "hotkey taken by another app");
                    status.set(key, State::Taken);
                    self.notice_hotkey(key, action, &hotkey.display());
                }
                Err(RegisterError::Failed(error)) => {
                    tracing::warn!(?action, key, hotkey = %hotkey.display(), %error, "hotkey not registered");
                    status.set(key, State::Failed);
                    self.notice_hotkey(key, action, &hotkey.display());
                }
            }
        }
        self.hotkeys = Some(hotkeys);
        self.ui.publish(status);
    }

    /// Tells the user a hotkey isn't working; clicked, the notice opens Settings. After a
    /// reload, only for the hotkeys that changed (the others were told already).
    fn notice_hotkey(&mut self, key: &str, action: Action, hotkey: &str) {
        if let Some(keys) = &self.ui.notify_keys
            && !keys.contains(&key)
        {
            return;
        }
        let body = tr!(
            "hotkey-notice-body",
            hotkey = hotkey,
            action = action_name(action)
        );
        self.balloon(
            &tr("hotkey-notice-title"),
            &body,
            BalloonKind::Warning,
            BalloonAction::Settings,
        );
    }

    /// Opens the overlay over `backdrop`, or, if it's open, switches to that backdrop.
    fn open(&mut self, backdrop: Backdrop, asked_100ns: i64) {
        if let Some(overlay) = self.overlay.as_mut() {
            match overlay.set_backdrop(backdrop) {
                Ok(ms) => tracing::info!(backdrop = log_name(backdrop), ms, "backdrop switched"),
                Err(error) => tracing::error!(%error, "couldn't switch the backdrop"),
            }
            self.ink.set_backdrop(overlay.backdrop());
            self.flush();
            self.refresh_tooltip();
            return;
        }
        let dim = self.settings.drawing.dim;
        let exe = std::env::current_exe().ok().filter(|_| !self.no_grabber);
        let look = Look {
            backdrops: self.settings.backdrops.clone(),
            guides: self.settings.guides.clone(),
        };
        match Overlay::open(
            backdrop,
            dim,
            asked_100ns,
            exe.as_deref(),
            look,
            self.guides,
        ) {
            Ok((overlay, t)) => {
                let backdrop = overlay.backdrop();
                self.ink.open(&overlay, &self.settings.drawing, backdrop);
                if self.test_drawing
                    && let Some(session) = self.ink.session_mut(overlay.drawing_screen())
                {
                    seed_test_drawing(session);
                }
                self.overlay = Some(overlay);
                self.prepare_pending = true;
                let since = |t: i64| (clock::now_100ns() - t) as f64 / 10_000.0;
                let flushed = clock::now_100ns();
                self.flush();
                let flush_ms = since(flushed);
                let toolbar_ms = self.open_toolbar();
                let ready_ms = since(asked_100ns);
                // The layers and the toolbar come in the same frame: hotkey to both on screen.
                let visible_ms = self
                    .overlay
                    .as_ref()
                    .map_or(t.layers_ms, |o| o.shown(asked_100ns));
                if visible_ms > OPEN_BUDGET_MS {
                    tracing::warn!(
                        backdrop = log_name(backdrop),
                        grab_ms = t.grab_ms,
                        layers_ms = t.layers_ms,
                        flush_ms,
                        toolbar_ms,
                        ready_ms,
                        visible_ms,
                        "opened, over the 100 ms budget"
                    );
                } else {
                    tracing::info!(
                        backdrop = log_name(backdrop),
                        grab_ms = t.grab_ms,
                        layers_ms = t.layers_ms,
                        flush_ms,
                        toolbar_ms,
                        ready_ms,
                        visible_ms,
                        "opened"
                    );
                }
            }
            Err(error) => tracing::error!(%error, "couldn't open the overlay"),
        }
        self.refresh_tooltip();
    }

    /// Shows the toolbar on the monitor under the cursor, where it was left there. Returns
    /// how long making it took, in milliseconds.
    fn open_toolbar(&mut self) -> f64 {
        let t = clock::now_secs();
        let Some(overlay) = self.overlay.as_ref() else {
            return 0.0;
        };
        let (x, y) = shell::cursor();
        let Some(monitor) = overlay.monitor(overlay.screen_at(x, y)).cloned() else {
            return 0.0;
        };
        let saved = self.settings.toolbar.positions.get(&monitor.name).copied();
        let state = self.toolbar_state();
        let look = self.toolbar_look.take().unwrap_or_default();
        match Toolbar::open(look, &monitor, saved, &state) {
            Ok((tb, _)) => self.toolbar = Some(tb),
            Err((look, error)) => {
                self.toolbar_look = Some(look);
                tracing::error!(%error, "couldn't show the toolbar");
            }
        }
        (clock::now_secs() - t) * 1000.0
    }

    /// Makes the toolbar's egui and draws it once off screen, at start-up: its fonts and
    /// glyphs are then ready, and an opening only makes its window and draws a frame.
    fn warm_toolbar(&mut self) {
        let scale = dpi::monitors()
            .into_iter()
            .find(|m| m.primary)
            .map_or(1.0, |m| m.scale());
        let state = self.toolbar_state();
        let mut look = toolbar::Look::new();
        look.warm(&state, scale);
        self.toolbar_look = Some(look);
    }

    /// What the toolbar shows now.
    fn toolbar_state(&self) -> toolbar::State {
        let (tool, colour, width, can_undo, can_redo) =
            self.ink.toolbar_view(&self.settings.drawing);
        toolbar::State {
            backdrop: self
                .overlay
                .as_ref()
                .map_or(Backdrop::Live, Overlay::backdrop),
            guides: self.guides,
            tool,
            colour,
            palette: self.settings.drawing.palette.clone(),
            width,
            can_undo,
            can_redo,
            collapsed: self.toolbar_collapsed,
            disappearing_ink: self.settings.drawing.disappearing_ink,
            right_click_undoes: self.settings.drawing.right_click_undoes,
        }
    }

    /// The toolbar's input, a frame when one is due, and what its controls asked for.
    fn on_toolbar(&mut self) {
        // A control can change what it shows (a tool, the backdrop): then it draws again.
        for _ in 0..3 {
            let state = self.toolbar_state();
            let Some(tb) = self.toolbar.as_mut() else {
                return;
            };
            tb.take_input();
            if !tb.due(&state) {
                break;
            }
            let actions = tb.frame(&state);
            let moved = tb.take_moved().map(|p| (tb.monitor().name.clone(), p));
            if let Some((monitor, position)) = moved {
                self.remember_toolbar(monitor, position);
            }
            for a in actions {
                self.toolbar_action(a);
            }
        }
    }

    /// Does what a toolbar control asks, the way its key does (the same events).
    fn toolbar_action(&mut self, action: toolbar::Action) {
        use crate::ink::Command as C;
        use toolbar::Action as A;
        tracing::info!(?action, "toolbar");
        // What its key does, as a command of its own: it acts while text is typed (where the
        // keys are text) and holds no key down.
        let command = match action {
            A::Tool(t) => Some(C::Tool(t)),
            A::Colour(i) => Some(C::Colour(i)),
            A::Undo => Some(C::Undo),
            A::Redo => Some(C::Redo),
            A::Clear => Some(C::Clear),
            A::Close => Some(C::Close),
            A::ToggleFreeze => Some(C::ToggleFreeze),
            _ => None,
        };
        if let Some(c) = command {
            let outcome = self.ink.command(c, &self.settings.drawing);
            self.outcome(outcome);
            self.flush();
            return;
        }
        let event = match action {
            A::Capture(c) => OverlayEvent::Command(c),
            A::Backdrop(b) => {
                self.switch_backdrop(b);
                return;
            }
            A::Guides(g) => {
                self.switch_guides(g);
                return;
            }
            A::Collapse(c) => {
                self.toolbar_collapsed = c;
                return;
            }
            A::Width(w) => {
                // As the wheel sets it (holding the tool's key): the next stroke and the
                // toolbar follow.
                self.ink.set_width(w, &self.settings.drawing);
                self.flush();
                return;
            }
            A::DisappearingInk(on) => {
                // Settings' own setting, taken up as a change from Settings is (the ink is
                // drawn again: a stroke half faded comes back whole when it's turned off),
                // and saved.
                let old = self.settings.drawing.clone();
                self.settings.drawing.disappearing_ink = on;
                self.drawing_changed(&old);
                self.save_setting(|s| s.drawing.disappearing_ink = on);
                self.flush();
                return;
            }
            _ => return,
        };
        self.overlay_event(event);
        self.flush();
    }

    /// Remembers where the toolbar was left on `monitor`, in Settings (saved when they can be).
    fn remember_toolbar(
        &mut self,
        monitor: String,
        position: overmark_core::settings::ToolbarPosition,
    ) {
        tracing::info!(monitor = %monitor, x = position.x, y = position.y, "toolbar moved");
        self.settings
            .toolbar
            .positions
            .insert(monitor.clone(), position);
        // Into the file as it is on disk (Settings may have changed it meanwhile).
        self.save_setting(|s| {
            s.toolbar.positions.insert(monitor, position);
        });
    }

    /// Closes the overlay: the strokes vanish: each opening starts clean.
    fn close(&mut self) {
        if let Some(tb) = self.toolbar.take() {
            self.toolbar_look = Some(tb.close());
        }
        if self.overlay.take().is_some() {
            self.pending_region = None;
            self.ink.close();
            self.wake = Wake::default();
            self.prepare_pending = false;
            trim_memory();
            tracing::info!("closed");
            self.refresh_tooltip();
        }
    }

    fn on_overlay(&mut self) {
        let events = self
            .overlay
            .as_mut()
            .map(Overlay::take_events)
            .unwrap_or_default();
        for e in events {
            if self.overlay.is_none() {
                break;
            }
            if self.width_slider_event(e) {
                continue;
            }
            self.overlay_event(e);
        }
        self.flush();
        self.on_toolbar();
    }

    /// While the toolbar's width strip is open, the arrows that come to the
    /// ink layer move its slider (the toolbar never takes the focus), unless text is being
    /// typed, where they move the caret. Esc stays the layer's. Returns whether the event was
    /// the slider's.
    fn width_slider_event(&mut self, e: OverlayEvent) -> bool {
        if self.ink.is_typing() {
            return false;
        }
        let Some(tb) = self.toolbar.as_mut().filter(|t| t.width_open()) else {
            return false;
        };
        let OverlayEvent::Key {
            vk,
            ctrl: false,
            alt: false,
            ..
        } = e
        else {
            return false;
        };
        let key = match vk {
            0x25 => eframe::egui::Key::ArrowLeft,
            0x26 => eframe::egui::Key::ArrowUp,
            0x27 => eframe::egui::Key::ArrowRight,
            0x28 => eframe::egui::Key::ArrowDown,
            _ => return false,
        };
        tb.key(key);
        true
    }

    /// One of the layer's events (or a toolbar control's, as the key it stands for).
    fn overlay_event(&mut self, e: OverlayEvent) {
        match e {
            OverlayEvent::Command(c) => {
                // What's drawn so far is in the capture.
                self.flush();
                self.capture(c);
            }
            OverlayEvent::RegionChosen { screen, rect } => {
                self.region_chosen(screen, rect);
                self.raise_toolbar();
            }
            OverlayEvent::RegionCancelled => {
                self.pending_region = None;
                tracing::info!("region cancelled");
                if std::mem::take(&mut self.ui.close_after_region) {
                    self.close();
                } else {
                    self.raise_toolbar();
                }
            }
            // A right-click on another monitor's layer while a region is chosen keeps that as it
            // was: it does nothing (Esc cancels).
            OverlayEvent::RightClick { .. }
                if self
                    .overlay
                    .as_ref()
                    .is_some_and(Overlay::is_choosing_region) => {}
            e => {
                let outcome = self.ink.handle(e, &self.settings.drawing);
                self.outcome(outcome);
            }
        }
    }

    /// Does what the drawing asked for after an event or a toolbar command.
    fn outcome(&mut self, outcome: Option<Outcome>) {
        match outcome {
            Some(Outcome::Close) => self.close(),
            Some(Outcome::ToggleFreeze) => self.toggle_freeze(),
            Some(Outcome::Backdrop { back }) => self.cycle_backdrop(back),
            Some(Outcome::Guides { back }) => self.cycle_guides(back),
            Some(Outcome::Notice(text)) => self.notice(&text),
            None => {}
        }
    }

    /// The ink layer came forward (it took the focus back after a region): the toolbar goes
    /// over it again.
    fn raise_toolbar(&self) {
        if let Some(tb) = &self.toolbar {
            tb.raise();
        }
    }

    /// Shows what the events changed.
    fn flush(&mut self) {
        let was = self.wake.animating;
        self.wake = match self.overlay.as_mut() {
            Some(o) => self.ink.flush(o, &self.settings.drawing),
            None => Wake::default(),
        };
        // Said so checks can tell when the loop stops coming back every frame.
        match (was, self.wake.animating) {
            (false, true) => tracing::info!("animating"),
            (true, false) => tracing::info!("still"),
            _ => {}
        }
    }

    /// F: Live and Frozen swap, the strokes staying.
    fn toggle_freeze(&mut self) {
        if let Some(overlay) = self.overlay.as_mut() {
            let to = freeze_toggled(overlay.backdrop());
            match overlay.set_backdrop(to) {
                Ok(ms) => tracing::info!(backdrop = log_name(to), ms, "F"),
                Err(error) => tracing::error!(%error, "couldn't switch the backdrop"),
            }
            self.ink.set_backdrop(overlay.backdrop());
        }
        self.refresh_tooltip();
    }

    /// W (Shift+W `back`): the next (previous) backdrop, the strokes staying.
    fn cycle_backdrop(&mut self, back: bool) {
        if let Some(overlay) = self.overlay.as_mut() {
            let from = overlay.backdrop();
            let to = if back { from.prev() } else { from.next() };
            match overlay.set_backdrop(to) {
                Ok(ms) => tracing::info!(backdrop = log_name(to), ms, back, "W"),
                Err(error) => tracing::error!(%error, "couldn't switch the backdrop"),
            }
            self.ink.set_backdrop(overlay.backdrop());
        }
        self.refresh_tooltip();
    }

    /// The toolbar's backdrop picker: straight to `to`, as W would get there.
    fn switch_backdrop(&mut self, to: Backdrop) {
        if let Some(overlay) = self.overlay.as_mut() {
            match overlay.set_backdrop(to) {
                Ok(ms) => tracing::info!(backdrop = log_name(to), ms, "backdrop picked"),
                Err(error) => tracing::error!(%error, "couldn't switch the backdrop"),
            }
            self.ink.set_backdrop(overlay.backdrop());
        }
        self.flush();
        self.refresh_tooltip();
    }

    /// The toolbar's guides picker: straight to `to`.
    fn switch_guides(&mut self, to: Guides) {
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.set_guides(to);
            self.guides = to;
            self.ink.redraw_all();
            tracing::info!(guides = guides_name(to), "guides picked");
        }
        self.flush();
    }

    /// G (Shift+G `back`): the next (previous) guides.
    fn cycle_guides(&mut self, back: bool) {
        if let Some(overlay) = self.overlay.as_mut() {
            let from = overlay.guides();
            let to = if back { from.prev() } else { from.next() };
            let t = clock::now_secs();
            overlay.set_guides(to);
            self.guides = to;
            // A blur on the frozen frame pixelates the guides too.
            self.ink.redraw_all();
            tracing::info!(
                guides = guides_name(to),
                ms = (clock::now_secs() - t) * 1000.0,
                back,
                "G"
            );
        }
    }

    /// Tells the user why something can't be done, on the monitor under the cursor.
    fn notice(&mut self, text: &str) {
        let (x, y) = shell::cursor();
        let monitor = self
            .overlay
            .as_ref()
            .and_then(|o| o.monitor(o.screen_at(x, y)).cloned());
        let Some(m) = monitor else {
            return;
        };
        self.toast = None;
        match Toast::show(m.work, m.scale(), text, Vec::new(), None, None) {
            Ok(t) => self.toast = Some(t),
            Err(error) => tracing::warn!(%error, "couldn't show the notice"),
        }
    }

    /// One turn of the loop: false when Overmark should quit.
    fn turn(&mut self, instance: &SingleInstance) -> bool {
        // With the overlay just open, come straight back to prepare it; otherwise wait for as
        // long as it takes.
        let timeout = if self.prepare_pending {
            Some(0)
        } else {
            let frame = self
                .overlay
                .as_ref()
                .and_then(|_| self.wake.timeout(FRAME_MS));
            let bar = self.toolbar.as_ref().and_then(Toolbar::wait_ms);
            // A click waiting for its double-click time comes back when that's up.
            let soonest = |a: Option<u32>, b: Option<u32>| match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            match (soonest(frame, bar), self.ui.wait_ms()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        for event in tray::wait(timeout, Some(instance)) {
            match event {
                TrayEvent::Activate => self.on_click(),
                TrayEvent::KeyActivate => self.click_action(),
                TrayEvent::DoubleClick => self.on_double_click(),
                TrayEvent::Menu { x, y } => {
                    if !self.menu(x, y) {
                        return false;
                    }
                }
                TrayEvent::BalloonClicked => self.on_balloon_clicked(),
                TrayEvent::ThemeChanged => {
                    if let Err(error) = self.tray.refresh() {
                        tracing::warn!(%error, "couldn't follow the taskbar's theme");
                    }
                }
                TrayEvent::Woken(code) => self.on_woken(code),
                TrayEvent::ReloadRequested => self.reload_settings(),
                TrayEvent::OpenRequested { frozen } => {
                    let backdrop = if frozen {
                        Backdrop::Frozen
                    } else {
                        Backdrop::Live
                    };
                    self.open(backdrop, clock::now_100ns());
                }
                TrayEvent::QuitRequested | TrayEvent::Quit => return false,
            }
        }
        self.click_if_due();
        let pressed = self
            .hotkeys
            .as_ref()
            .map(Hotkeys::take_pressed)
            .unwrap_or_default();
        for (action, at) in pressed {
            let backdrop = match action {
                Action::Draw => Backdrop::Live,
                Action::DrawFrozen => Backdrop::Frozen,
                Action::DrawWhiteboard => Backdrop::Whiteboard,
                Action::DrawBlackboard => Backdrop::Blackboard,
            };
            self.open(backdrop, at);
        }
        self.on_overlay();
        if self.prepare_pending {
            self.prepare_pending = false;
            if let Some(overlay) = self.overlay.as_mut() {
                let t = clock::now_secs();
                overlay.prepare();
                tracing::debug!(
                    ms = (clock::now_secs() - t) * 1000.0,
                    "prepared for a freeze"
                );
                // The Text tool's and the stamps' fonts, once: after the layer shows, so they
                // never slow its opening.
                let t = clock::now_secs();
                let fonts = overmark_win::fonts::ensure_loaded();
                tracing::debug!(fonts, ms = (clock::now_secs() - t) * 1000.0, "fonts");
            }
        }
        true
    }
}

/// The name the logs use for guides.
fn guides_name(g: Guides) -> &'static str {
    match g {
        Guides::Off => "Off",
        Guides::Grid => "Grid",
        Guides::Dots => "Dots",
        Guides::Lines => "Lines",
    }
}

/// Gives back the memory the overlay used: once it's closed, the pages it touched are returned
/// to Windows, so the idle footprint is the tray's again.
fn trim_memory() {
    overmark_win::win32::process::trim_working_set();
}

/// Runs Overmark until it's asked to quit. Returns the process's exit code.
pub fn run(args: Args) -> i32 {
    if args.grabber {
        return overmark_win::grabber::serve();
    }
    let _ = dpi::enable_per_monitor_v2();
    overmark_core::i18n::init(&language::ui_language());
    if let Some(page) = args.settings {
        // Settings logs to stderr only: the file is the tray's (one writer rolls it).
        logging::init(None);
        logging::log_panics();
        return crate::settings_window::run(page);
    }
    // The mouse comes as pointer messages, as pens and touch do: one path for all three.
    if let Err(error) = overmark_win::win32::pointer::enable_mouse_in_pointer() {
        eprintln!(
            "{}",
            tr!("cli-no-pointer-mouse", reason = error.to_string())
        );
    }
    if args.quit {
        return if tray::request_quit(APP_ID) { 0 } else { 1 };
    }
    // A level for this run can't be handed to an instance already running: it takes its place
    // (otherwise the new level is silently ignored, as it was at first).
    if args.dim.is_some() && !tray::replace_running(APP_ID, 5000) {
        eprintln!("{}", tr("cli-dim-running"));
        return 1;
    }
    // Only the one instance logs to the file (it rolls the file; two writers would fight).
    let instance = match SingleInstance::acquire(APP_ID, args.frozen) {
        Ok(Instance::First(i)) => i,
        Ok(Instance::Second) => {
            eprintln!("{}", tr("cli-already-running"));
            return 0;
        }
        Err(error) => {
            eprintln!(
                "{}",
                tr!("cli-instance-check-failed", reason = error.to_string())
            );
            return 1;
        }
    };
    let paths = AppPaths::detect(APP_ID);
    let log_dir = paths
        .as_ref()
        .ok()
        .filter(|p| p.is_writable())
        .map(AppPaths::logs_dir);
    let log_file = logging::init(log_dir.as_deref());
    logging::log_panics();
    let startup = Startup::load(paths);
    let about = startup.about();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = about.mode.map_or("unknown", Mode::as_str),
        settings = ?about.settings_file,
        can_save = about.can_save,
        created = startup.created,
        log = ?log_file,
        "starting"
    );
    for w in &startup.warnings {
        tracing::warn!(warning = %w, "settings");
    }
    let mut settings = startup.settings.clone();
    if let Some(dim) = args.dim {
        // For this run only: never saved.
        settings.drawing.dim = dim;
    }
    let tray = match Tray::new(tray_ui::ICON_IDLE, &tr("app-name")) {
        Ok(t) => t,
        Err(error) => {
            tracing::error!(%error, "couldn't add the tray icon");
            return 1;
        }
    };
    let mut app = App {
        settings,
        tray,
        hotkeys: None,
        registered: Vec::new(),
        overlay: None,
        prepare_pending: false,
        ink: Ink::default(),
        wake: Wake::default(),
        pictures: overmark_win::win32::known_folders::pictures(),
        toast: None,
        pending_region: None,
        test_drawing: args.test_drawing,
        guides: Guides::Off,
        no_grabber: args.no_grabber,
        ui: TrayUi::new(about.settings_file.clone(), startup.can_save),
        toolbar: None,
        toolbar_look: None,
        toolbar_collapsed: false,
    };
    for n in &startup.notices {
        tracing::warn!(title = %n.title, body = %n.body, "start-up notice");
    }
    // Windows shows one balloon at a time: the most important (the folder, then the file).
    if let Some(n) = startup.notices.first()
        && let Err(error) = app.tray.balloon(&n.title, &n.body, BalloonKind::Warning)
    {
        tracing::warn!(%error, "couldn't show the start-up notice");
    }
    app.register_hotkeys();
    app.refresh_tooltip();
    // Only when the user has turned it on.
    app.check_updates(false);
    app.warm_toolbar();
    if args.frozen {
        app.open(Backdrop::Frozen, clock::now_100ns());
    }
    trim_memory();
    tracing::info!(pid = std::process::id(), "resident in the tray");
    while app.turn(&instance) {}
    app.close();
    tracing::info!("quit");
    0
}

/// Where the `--test-drawing` strokes go on the drawing's monitor, for checks: a half-alpha
/// blue pen stroke along `y = TEST_PEN_Y` and a yellow highlighter along `y = TEST_MARKER_Y`,
/// both from `x = TEST_X0` to `TEST_X1`.
pub const TEST_PEN_Y: f64 = 600.0;
pub const TEST_MARKER_Y: f64 = 680.0;
pub const TEST_X0: f64 = 300.0;
pub const TEST_X1: f64 = 900.0;

/// Draws the `--test-drawing` strokes into `session` with core's own stroke calls (exactly
/// known strokes, the drawing a capture is proven with; `draw_check` draws with the tools).
fn seed_test_drawing(session: &mut Session) {
    use overmark_core::colour::Colour;
    use overmark_core::drawing::InkPoint;
    use overmark_core::tool::ToolKind;
    let (tool, colour, width) = (session.tool, session.colour, session.width);
    for (t, c, w, y) in [
        (
            ToolKind::Pen,
            Colour {
                r: 0,
                g: 0,
                b: 255,
                a: 128,
            },
            18.0,
            TEST_PEN_Y,
        ),
        (
            ToolKind::Highlighter,
            Colour::rgb(255, 255, 0),
            26.0,
            TEST_MARKER_Y,
        ),
    ] {
        session.tool = t;
        session.colour = c;
        session.width = w;
        session.stroke_begin(InkPoint::mouse(TEST_X0, y, 0));
        let steps = 60u32;
        for i in 1..=steps {
            let x = TEST_X0 + (TEST_X1 - TEST_X0) * f64::from(i) / f64::from(steps);
            session.stroke_extend(InkPoint::mouse(x, y, u64::from(i) * 16));
        }
        session.stroke_end();
    }
    session.tool = tool;
    session.colour = colour;
    session.width = width;
}

/// A capture waiting for its region to be chosen: the composed image of monitor `screen`.
struct PendingRegion {
    screen: usize,
    image: overmark_win::save::Image,
}

impl App {
    /// The drawing's clock now, in milliseconds (the clock its samples will use).
    fn now_ms(&self) -> u64 {
        (clock::now_secs() * 1000.0) as u64
    }

    fn disappearing_ink_ms(&self) -> Option<u64> {
        let d = &self.settings.drawing;
        d.disappearing_ink
            .then(|| (d.disappearing_ink_seconds.max(0.0) * 1000.0) as u64)
    }

    /// The captures folder: the setting, `{pictures}` being the real Pictures folder.
    fn captures_folder(&self) -> std::path::PathBuf {
        let pictures = self.pictures.clone().unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join("Pictures")
        });
        self.settings.capture.resolve_save_folder(&pictures)
    }

    /// Runs a capture command on the monitor under the cursor.
    fn capture(&mut self, command: Command) {
        let started = clock::now_secs();
        let (x, y) = shell::cursor();
        let with_drawing =
            command != Command::SaveWithoutDrawing && self.settings.capture.include_drawing;
        let (now, fade) = (self.now_ms(), self.disappearing_ink_ms());
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        let screen = overlay.screen_at(x, y);
        let monitor = overlay.monitor(screen).cloned();
        let Some(frame) = overlay.capture_frame(screen) else {
            tracing::error!(screen, "no clean grab for a capture");
            self.toast_text(
                screen,
                &tr("toast-capture-no-image"),
                Vec::new(),
                None,
                None,
            );
            return;
        };
        let drawing = with_drawing
            .then(|| self.ink.session(screen).map(Session::drawing))
            .flatten();
        let guides = overlay.save_guides(screen);
        let Some(image) = save::compose(&frame, guides.as_ref(), drawing, now, fade) else {
            tracing::error!("the capture couldn't be composed");
            self.toast_text(
                screen,
                &tr("toast-capture-no-image"),
                Vec::new(),
                None,
                None,
            );
            return;
        };
        tracing::info!(
            ?command,
            screen,
            monitor = monitor.as_ref().map_or("", |m| m.name.as_str()),
            width = image.width,
            height = image.height,
            with_drawing,
            ms = (clock::now_secs() - started) * 1000.0,
            "captured"
        );
        if command == Command::SaveRegion {
            match overlay.begin_region(screen, &image) {
                Ok(()) => self.pending_region = Some(PendingRegion { screen, image }),
                Err(error) => {
                    tracing::error!(%error, "couldn't start choosing a region");
                    let text = tr!("toast-capture-failed", reason = error.message());
                    self.toast_text(screen, &text, Vec::new(), None, None);
                }
            }
            return;
        }
        self.deliver(screen, &image, command, with_drawing);
    }

    /// Copies and/or saves `image`, as `command` and the settings say, and tells the user.
    fn deliver(
        &mut self,
        screen: usize,
        image: &overmark_win::save::Image,
        command: Command,
        with_drawing: bool,
    ) {
        let after = self.settings.capture.after_capture;
        let copies = command == Command::Copy || after.copies();
        let saves = command != Command::Copy && after.saves();
        let copied = copies.then(|| save::copy(image));
        let folder = self.captures_folder();
        let saved = saves.then(|| {
            let c = &self.settings.capture;
            save::write_capture(
                &folder,
                &c.file_name_pattern,
                &shell::capture_time(),
                c.format,
                c.jpg_quality,
                image,
            )
        });
        let file = match &saved {
            Some(Ok(p)) => Some(p.clone()),
            _ => None,
        };
        tracing::info!(copied = ?copied.as_ref().map(|r| r.is_ok()), file = ?file, folder = %folder.display(), "delivered");
        let folder_text = folder.display().to_string();
        let title = match (&copied, &saved) {
            (Some(Ok(())), Some(Ok(p))) => {
                tr!("toast-copied-and-saved", file = p.display().to_string())
            }
            (None, Some(Ok(p))) => tr!("toast-saved", file = p.display().to_string()),
            (Some(Ok(())), None) => tr("toast-copied"),
            (Some(Ok(())), Some(Err(e))) => tr!(
                "toast-copied-not-saved",
                folder = folder_text.as_str(),
                reason = e.reason.to_string()
            ),
            (_, Some(Err(e))) => tr!(
                "toast-save-failed",
                folder = folder_text.as_str(),
                reason = e.reason.to_string()
            ),
            (Some(Err(e)), _) => tr!("toast-copy-failed", reason = e.as_str()),
            (None, None) => return,
        };
        let mut lines = Vec::new();
        if !with_drawing {
            lines.push(tr("toast-without-drawing"));
        }
        if file.is_some() {
            lines.push(tr("toast-hint"));
        }
        self.toast_text(screen, &title, lines, file, Some(folder));
    }

    fn toast_text(
        &mut self,
        screen: usize,
        title: &str,
        lines: Vec<String>,
        file: Option<std::path::PathBuf>,
        folder: Option<std::path::PathBuf>,
    ) {
        if !self.settings.capture.toast {
            return;
        }
        let monitor = self
            .overlay
            .as_ref()
            .and_then(|o| o.monitor(screen).cloned())
            .or_else(|| dpi::monitors().into_iter().next());
        let Some(m) = monitor else {
            return;
        };
        self.toast = None;
        match Toast::show(m.work, m.scale(), title, lines, file, folder) {
            Ok(t) => self.toast = Some(t),
            Err(error) => tracing::warn!(%error, "couldn't show the capture notice"),
        }
    }

    /// The region was chosen: crop the composed image to it and deliver it.
    fn region_chosen(&mut self, screen: usize, rect: overmark_win::win32::Rect) {
        let Some(p) = self.pending_region.take() else {
            return;
        };
        if p.screen != screen {
            tracing::warn!(screen, "a region on another monitor than the capture's");
        }
        match save::crop(&p.image, rect) {
            Some(img) => {
                tracing::info!(
                    left = rect.left,
                    top = rect.top,
                    width = img.width,
                    height = img.height,
                    "region chosen"
                );
                let with_drawing = self.settings.capture.include_drawing;
                self.deliver(p.screen, &img, Command::SaveRegion, with_drawing);
            }
            None => tracing::warn!("the region was empty"),
        }
        // Opened from the tray's "Capture a region": done with it.
        if std::mem::take(&mut self.ui.close_after_region) {
            self.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use crate::settings_window::Page;

    fn parse(a: &[&str]) -> Args {
        Args::parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn reads_the_command_line() {
        let a = parse(&["--frozen", "--dim", "0.45"]);
        assert!(a.frozen && !a.quit && !a.grabber);
        assert_eq!(a.dim, Some(0.45));
        assert_eq!(parse(&["--dim", "3"]).dim, Some(0.8));
        assert_eq!(parse(&["--dim"]).dim, None);
        assert!(parse(&["--quit"]).quit);
        assert!(parse(&["--test-drawing"]).test_drawing);
        assert!(parse(&["--no-grabber"]).no_grabber);
        assert_eq!(parse(&["--settings"]).settings, Some(Page::General));
        assert_eq!(
            parse(&["--settings", "--about"]).settings,
            Some(Page::About)
        );
        assert_eq!(parse(&[]).settings, None);
    }

    /// Every notice the capture can show is a string of its own in English (none written in
    /// the code), the failure without a clean image included.
    #[test]
    fn the_capture_notices_are_in_the_strings() {
        for key in [
            "toast-capture-no-image",
            "toast-copied-and-saved",
            "toast-saved",
            "toast-copied",
            "toast-save-failed",
            "toast-copy-failed",
            "toast-copied-not-saved",
            "toast-without-drawing",
            "toast-hint",
        ] {
            assert!(overmark_core::i18n::has(key), "{key}");
        }
        let text = overmark_core::i18n::tr("toast-capture-no-image");
        assert!(
            text.starts_with("The screen couldn't be captured"),
            "{text}"
        );
        assert!(parse(&[overmark_win::grabber::ARG]).grabber);
    }

    fn shortcuts(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, h)| ((*a).to_owned(), (*h).to_owned()))
            .collect()
    }

    /// Idle: the name, then each registered hotkey on its own line, as the menu names them.
    #[test]
    fn the_tooltip_lists_the_hotkeys() {
        overmark_core::i18n::init("en");
        let all = shortcuts(&[
            ("Draw", "Win+Shift+D"),
            ("Draw on a frozen screen", "Win+Shift+F"),
            ("Whiteboard", "Ctrl+Shift+W"),
        ]);
        assert_eq!(
            super::tooltip_text(None, &all),
            "Overmark\nDraw: Win+Shift+D\nDraw on a frozen screen: Win+Shift+F\n\
             Whiteboard: Ctrl+Shift+W"
        );
        // One that isn't set or couldn't be registered isn't in the list, so isn't shown.
        let one = shortcuts(&[("Draw on a frozen screen", "Win+Shift+F")]);
        assert_eq!(
            super::tooltip_text(None, &one),
            "Overmark\nDraw on a frozen screen: Win+Shift+F"
        );
        assert_eq!(super::tooltip_text(None, &[]), "Overmark: click to draw");
    }

    /// Drawing: the state first, the hotkeys under it.
    #[test]
    fn the_tooltip_says_its_drawing_then_the_hotkeys() {
        use overmark_win::overlay::Backdrop;
        overmark_core::i18n::init("en");
        let one = shortcuts(&[("Draw", "Win+Shift+D")]);
        assert_eq!(
            super::tooltip_text(Some(Backdrop::Frozen), &one),
            "Overmark: drawing (Frozen)\nDraw: Win+Shift+D"
        );
        assert_eq!(
            super::tooltip_text(Some(Backdrop::Live), &[]),
            "Overmark: drawing (Live)"
        );
    }

    /// A long translation: whole lines are left out from the end, never cut, and the whole
    /// stays within the 127 units `szTip` holds (counted in UTF-16, as it is).
    #[test]
    fn a_long_tooltip_drops_whole_lines() {
        overmark_core::i18n::init("en");
        let long = "Dessiner sur un écran figé, avec la souris, le stylet ou le doigt";
        let list = shortcuts(&[
            ("Dessiner", "Win+Maj+D"),
            (long, "Win+Maj+F"),
            ("Tableau blanc", "Ctrl+Maj+W"),
        ]);
        let text = super::tooltip_text(None, &list);
        assert!(text.encode_utf16().count() <= super::TOOLTIP_MAX, "{text}");
        assert_eq!(
            text,
            format!("Overmark\nDessiner: Win+Maj+D\n{long}: Win+Maj+F"),
            "the third line doesn't fit, so it's left out"
        );
        // Lines that fit all show; once one doesn't, none after it does, so the order holds.
        let fit = |first: &str, lines: Vec<String>| {
            super::fit_lines(first.into(), "Overmark", lines, 127)
        };
        let text = fit("A", vec!["B".repeat(120), "C".into()]);
        assert_eq!(text, format!("A\n{}\nC", "B".repeat(120)));
        assert_eq!(fit("A", vec!["B".repeat(130), "C".into()]), "A");
        // Exactly the limit fits; one over doesn't.
        let line = "x".repeat(125);
        assert_eq!(fit("A", vec![line.clone()]).len(), 127);
        assert_eq!(fit("AB", vec![line]), "AB");
    }

    /// A state line too long on its own (counted in UTF-16) isn't cut: the name stands in for
    /// it, with the hotkeys that fit under it; a name too long as well gives way to "Overmark".
    #[test]
    fn a_first_line_too_long_gives_way_to_the_name() {
        let state = format!(
            "Overmark : vous dessinez sur l'écran figé {}",
            "é".repeat(120)
        );
        assert!(state.encode_utf16().count() > 127);
        let text = super::fit_lines(
            state.clone(),
            "Overmark",
            ["Dessiner: Win+Maj+D".into()],
            127,
        );
        assert_eq!(text, "Overmark\nDessiner: Win+Maj+D");
        let text = super::fit_lines(state.clone(), &state, Vec::new(), 127);
        assert_eq!(text, "Overmark");
    }

    #[test]
    fn the_tooltip_names_a_board() {
        overmark_core::i18n::init("en");
        let text = overmark_core::tr!(
            "tray-tooltip-board",
            backdrop = overmark_win::overlay::Backdrop::Whiteboard.name()
        );
        assert!(text.contains("Whiteboard"), "{text}");
        assert!(!text.contains("tray-tooltip"), "{text}");
    }
}
