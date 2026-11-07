//! Recording while annotating: the toolbar's Record, the box chosen as a region is, then its
//! outline and bar ([`crate::record_bar`]) while the drawing goes on.
//!
//! - Record (the toolbar) → choose a box, exactly as Save a region does → the outline and the
//!   bar on its top edge, ready.
//! - The bar's Record starts the recording; Pause and Resume; Stop saves the MP4 in the captures
//!   folder (named as captures are, `.mp4`) and shows the notice, whose click opens it and
//!   right-click its folder; ✕ cancels the box before recording, and on a recording asks
//!   before it's thrown away.
//! - Esc before recording cancels the box (the layer stays). Closing the layer while recording
//!   (Esc, the toolbar's Close, a hotkey, the tray) stops and saves first: closing never loses
//!   a recording.
//! - A recording that ends by itself (the screen locked, the monitor unplugged) wakes the loop
//!   ([`WAKE_RECORDING`]), and is saved and told as Stop's is, with the reason.
//!
//! **Screen capture** (the tray's Record a region…, or its hotkey): the same box and bar with no
//! drawing layer, over the apps, which go on being used. The box is chosen on still grabs of the
//! monitors (`region::Picker`), then the keyboard goes back to the app that had it; the bar
//! never takes it (so Esc can't reach it): **the hotkey pressed again ends the box**, saving a
//! recording. Twice a second the bar and outline go back on top (`core::record::Reassert`), as
//! another always-on-top window can cover them.

use overmark_core::i18n::tr;
use overmark_core::record::{Encoder, PixelRect, RecordError, place};
use overmark_core::tr;
use overmark_win::record::{EncoderChoice, Saved, Start, Status};
use overmark_win::region::{Picked, Picker};
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::toast::Toast;
use overmark_win::win32::{Rect, capture, clock, shell, window};

use super::App;
use crate::record_bar::{BarAction, RecordBox};

/// What Record a region (the tray's item, or its hotkey) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HotkeyDoes {
    /// A recording (either mode's, paused or asking whether to discard too): stop and save it.
    StopAndSave,
    /// A box not yet recording: close it.
    CloseBox,
    /// A screen recording's box is being chosen: stop choosing.
    CancelChoosing,
    /// The drawing layer is open: screen capture is without it, so say to close it (or record
    /// from its toolbar). Nothing is closed: the drawing is never lost to a key.
    SayCloseTheLayer,
    /// Choose a screen recording's box.
    Choose,
}

/// What Record a region does, given the box (`Some(recording)`, if one is open), whether a
/// screen recording's box is being chosen, and whether the drawing layer is open (or choosing
/// its own box).
pub(super) fn hotkey_does(boxed: Option<bool>, choosing: bool, layer: bool) -> HotkeyDoes {
    match (boxed, choosing, layer) {
        (Some(true), _, _) => HotkeyDoes::StopAndSave,
        (Some(false), _, _) => HotkeyDoes::CloseBox,
        (None, true, _) => HotkeyDoes::CancelChoosing,
        (None, false, true) => HotkeyDoes::SayCloseTheLayer,
        (None, false, false) => HotkeyDoes::Choose,
    }
}

/// What a recording's notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Notice {
    /// A recording saved as asked: the capture notice, which Settings may turn off.
    Saved,
    /// Something went wrong, a recording stopped by itself, or what to do instead (the layer
    /// is open): the only way the user hears of it, so it always shows.
    Problem,
}

/// Whether a notice of `kind` shows, Settings' "Show a notice after each capture" being
/// `capture_notices`.
pub(super) fn notice_shows(kind: Notice, capture_notices: bool) -> bool {
    match kind {
        Notice::Saved => capture_notices,
        Notice::Problem => true,
    }
}

/// The tray's wake code for a recording that ended by itself.
pub(super) const WAKE_RECORDING: u32 = 2;

/// Choosing a screen recording's box: the selection, the monitors it covers, and the window
/// that had the keyboard before it (given it back when choosing ends).
pub(super) struct ScreenPick {
    picker: Picker,
    monitors: Vec<Monitor>,
    had_keyboard: Option<windows::Win32::Foundation::HWND>,
}

/// A monitor's screen as it is now, through GDI (none of our windows is on it yet), as the
/// selection's still image.
fn grab(m: &Monitor) -> Option<overmark_win::save::Image> {
    let frame = capture::gdi_screen(m.rect)?;
    let mut rgba = frame.bgra;
    for p in rgba.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    Some(overmark_win::save::Image {
        width: frame.width as u32,
        height: frame.height as u32,
        rgba,
    })
}

impl App {
    /// The toolbar's Record: choose the box, as Save a region does.
    pub(super) fn begin_record(&mut self) {
        if self.record.is_some() || self.pending_record || self.overlay.is_none() {
            return;
        }
        self.pending_record = true;
        self.capture(overmark_win::overlay::Command::SaveRegion);
        if self.pending_region.is_none() {
            // It couldn't start (the notice said why).
            self.pending_record = false;
        }
    }

    /// The box was chosen on the drawing layer's monitor `screen` (`rect` in its pixels).
    pub(super) fn record_box_chosen(&mut self, screen: usize, rect: Rect) {
        self.pending_record = false;
        self.pending_region = None;
        let Some(overlay) = self.overlay.as_ref() else {
            return;
        };
        let monitors = overlay.monitors();
        self.open_record_box(&monitors, screen, rect, false);
    }

    /// Opens the box chosen on `monitors[screen]` (`rect` in its pixels): its outline and bar.
    /// `screen_mode`: a screen recording, with no drawing layer.
    fn open_record_box(
        &mut self,
        monitors: &[Monitor],
        screen: usize,
        rect: Rect,
        screen_mode: bool,
    ) {
        let Some(monitor) = monitors.get(screen).cloned() else {
            return;
        };
        let m = monitor.rect;
        let area = PixelRect::new(
            m.left + rect.left,
            m.top + rect.top,
            m.left + rect.right,
            m.top + rect.bottom,
        );
        let rects: Vec<PixelRect> = monitors
            .iter()
            .map(|m| PixelRect::new(m.rect.left, m.rect.top, m.rect.right, m.rect.bottom))
            .collect();
        // What will be recorded: the box on its monitor, its sides even. Refused plainly.
        let placed = match place(area, &rects) {
            Ok(p) => p.area,
            Err(e) => {
                let error = RecordError::from(e);
                tracing::info!(?error, "the box can't be recorded");
                self.toast_on(
                    Notice::Problem,
                    Some(monitor),
                    &error.message(),
                    Vec::new(),
                    None,
                );
                return;
            }
        };
        let area = Rect::new(placed.left, placed.top, placed.right, placed.bottom);
        match RecordBox::open(area, &monitor) {
            Ok(b) => {
                tracing::info!(
                    left = area.left,
                    top = area.top,
                    width = area.width(),
                    height = area.height(),
                    screen,
                    screen_mode,
                    "a box to record"
                );
                self.record = Some(b);
                self.record_screen_mode = screen_mode;
                self.reassert =
                    overmark_core::record::Reassert::new(overmark_core::record::REASSERT_MS);
                self.refresh_tooltip();
            }
            Err(error) => {
                tracing::error!(%error, "couldn't show the record box");
                let text = tr!("toast-recording-failed", reason = error.message());
                self.toast_on(Notice::Problem, Some(monitor), &text, Vec::new(), None);
            }
        }
    }

    /// The tray's Record a region…, or its hotkey: what [`hotkey_does`] says for where things
    /// are.
    pub(super) fn record_region_hotkey(&mut self) {
        let does = hotkey_does(
            self.record.as_ref().map(|b| b.is_recording()),
            self.screen_pick.is_some(),
            self.overlay.is_some() || self.pending_record,
        );
        tracing::info!(?does, "record hotkey");
        match does {
            HotkeyDoes::StopAndSave => self.finish_recording(),
            HotkeyDoes::CloseBox => self.cancel_record_box(),
            HotkeyDoes::CancelChoosing => {
                if let Some(pick) = self.screen_pick.take() {
                    drop(pick.picker);
                    if let Some(h) = pick.had_keyboard {
                        window::give_focus_back(h);
                    }
                    tracing::info!("choosing a box to record cancelled");
                }
            }
            HotkeyDoes::SayCloseTheLayer => {
                let (x, y) = shell::cursor();
                let monitor = dpi::monitors().into_iter().find(|m| {
                    x >= m.rect.left && x < m.rect.right && y >= m.rect.top && y < m.rect.bottom
                });
                self.toast_on(
                    Notice::Problem,
                    monitor,
                    &tr("toast-record-layer-open"),
                    Vec::new(),
                    None,
                );
            }
            HotkeyDoes::Choose => self.choose_screen_box(),
        }
    }

    /// A screen recording's box: chosen over still grabs of the monitors.
    fn choose_screen_box(&mut self) {
        let monitors = dpi::monitors();
        let images: Option<Vec<_>> = monitors.iter().map(grab).collect();
        let Some(images) = images else {
            let first = monitors.first().cloned();
            self.toast_on(
                Notice::Problem,
                first,
                &tr("toast-capture-no-image"),
                Vec::new(),
                None,
            );
            return;
        };
        let (x, y) = shell::cursor();
        let focus = monitors
            .iter()
            .position(|m| {
                x >= m.rect.left && x < m.rect.right && y >= m.rect.top && y < m.rect.bottom
            })
            .unwrap_or(0);
        let had_keyboard = window::foreground();
        match Picker::open(&monitors, &images, focus) {
            Ok(picker) => {
                tracing::info!(screens = monitors.len(), "choosing a box to record");
                self.screen_pick = Some(ScreenPick {
                    picker,
                    monitors,
                    had_keyboard,
                });
            }
            Err(error) => {
                tracing::error!(%error, "couldn't start choosing a box");
                let text = tr!("toast-recording-failed", reason = error.message());
                self.toast_on(
                    Notice::Problem,
                    monitors.get(focus).cloned(),
                    &text,
                    Vec::new(),
                    None,
                );
            }
        }
    }

    /// A screen recording's box being chosen: how it ended, if it has.
    fn on_screen_pick(&mut self) {
        let Some(pick) = self.screen_pick.as_mut() else {
            return;
        };
        let Some(picked) = pick.picker.poll() else {
            return;
        };
        let pick = self.screen_pick.take().expect("a pick");
        let ScreenPick {
            picker,
            monitors,
            had_keyboard,
        } = pick;
        // The selection goes, and the keyboard goes back to the app that had it.
        drop(picker);
        if let Some(h) = had_keyboard {
            window::give_focus_back(h);
        }
        match picked {
            Picked::Chosen { screen, rect } => self.open_record_box(&monitors, screen, rect, true),
            Picked::Cancelled => tracing::info!("choosing a box to record cancelled"),
        }
    }

    /// The bar's input, a frame when one is due, and what it asked for.
    pub(super) fn on_record(&mut self) {
        self.on_screen_pick();
        // A screen recording's bar and outline go back on top now and then: another
        // always-on-top window may have covered them.
        if self.record_screen_mode
            && let Some(b) = self.record.as_ref()
            && self.reassert.due((clock::now_secs() * 1000.0) as u64)
        {
            b.raise();
        }
        let Some(b) = self.record.as_mut() else {
            return;
        };
        let actions = b.take_actions();
        for action in actions {
            self.record_action(action);
        }
    }

    fn record_action(&mut self, action: BarAction) {
        tracing::info!(?action, "record bar");
        let Some(b) = self.record.as_mut() else {
            return;
        };
        match action {
            BarAction::Record if b.is_recording() => b.resume(),
            BarAction::Record => self.start_recording(),
            BarAction::Pause => b.pause(),
            BarAction::Stop => self.finish_recording(),
            BarAction::Discard if b.is_recording() => b.set_confirm_discard(true),
            BarAction::Discard => self.cancel_record_box(),
            BarAction::Keep => b.set_confirm_discard(false),
            BarAction::ConfirmDiscard => {
                if let Some(b) = self.record.take() {
                    b.discard();
                    tracing::info!("recording discarded");
                    self.refresh_tooltip();
                }
            }
        }
    }

    /// The bar's Record: the recording starts, into the captures folder.
    fn start_recording(&mut self) {
        let folder = self.captures_folder();
        let screen = self.record_monitor();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            let error = RecordError::File {
                path: folder.display().to_string(),
                detail: error.to_string(),
            };
            self.recording_failed(screen, &error);
            return;
        }
        let pattern = &self.settings.capture.file_name_pattern;
        let path = match overmark_core::settings::file_name::capture_path(
            &folder,
            pattern,
            &shell::capture_time(),
            "mp4",
        ) {
            Ok(p) => p,
            Err(error) => {
                let error = RecordError::File {
                    path: folder.display().to_string(),
                    detail: error.to_string(),
                };
                self.recording_failed(screen, &error);
                return;
            }
        };
        let r = &self.settings.recording;
        let mut start = Start::new(PixelRect::default(), r.record_settings(), path);
        start.encoder = match r.encoder {
            Encoder::Automatic => EncoderChoice::Auto,
            Encoder::Software => EncoderChoice::Software,
        };
        let waker = self.tray.waker();
        let Some(b) = self.record.as_mut() else {
            return;
        };
        match b.record(start, move || {
            waker.wake(WAKE_RECORDING);
        }) {
            Ok(()) => {
                tracing::info!("recording started");
                self.refresh_tooltip();
            }
            Err(error) => {
                // Nothing was recorded: the box stays, so Record can be tried again.
                self.recording_failed(screen, &error);
            }
        }
    }

    /// The monitor the box is on (for the notice).
    fn record_monitor(&self) -> Option<Monitor> {
        self.record.as_ref().map(|b| b.monitor().clone())
    }

    /// The notice on `monitor` (or the primary), as captures' is. A plain "saved" follows
    /// Settings' "Show a notice after each capture"; a problem or an instruction always shows
    /// ([`notice_shows`]).
    fn toast_on(
        &mut self,
        kind: Notice,
        monitor: Option<Monitor>,
        title: &str,
        lines: Vec<String>,
        file: Option<std::path::PathBuf>,
    ) {
        if !notice_shows(kind, self.settings.capture.toast) {
            tracing::debug!(?kind, "the recording's notice is off in Settings");
            return;
        }
        let Some(m) = monitor.or_else(|| dpi::monitors().into_iter().next()) else {
            return;
        };
        let folder = file
            .as_ref()
            .and_then(|f| f.parent())
            .map(|p| p.to_path_buf());
        self.toast = None;
        match Toast::show(m.work, m.scale(), title, lines, file, folder) {
            Ok(t) => self.toast = Some(t),
            Err(error) => tracing::warn!(%error, "couldn't show the recording's notice"),
        }
    }

    fn recording_failed(&mut self, monitor: Option<Monitor>, error: &RecordError) {
        tracing::warn!(?error, "couldn't record");
        let text = tr!("toast-recording-failed", reason = error.message());
        self.toast_on(Notice::Problem, monitor, &text, Vec::new(), None);
    }

    /// Esc or ✕ before recording: the box goes, the layer stays.
    pub(super) fn cancel_record_box(&mut self) {
        if self.record.take().is_some() {
            tracing::info!("record box cancelled");
            self.refresh_tooltip();
        }
    }

    /// Whether Esc should cancel the box (it's up and nothing is being recorded).
    pub(super) fn esc_cancels_record_box(&self) -> bool {
        !self.record_screen_mode && self.record.as_ref().is_some_and(|b| !b.is_recording())
    }

    /// How long the loop may wait for the recording's sake: the bar's timer and egui, and a
    /// screen recording's next going back on top.
    pub(super) fn record_wait_ms(&self) -> Option<u32> {
        let bar = self
            .record
            .as_ref()
            .and_then(crate::record_bar::RecordBox::wait_ms);
        let on_top = (self.record_screen_mode && self.record.is_some())
            .then(|| self.reassert.wait_ms((clock::now_secs() * 1000.0) as u64) as u32);
        match (bar, on_top) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Stop: saves, the box goes, and the notice says where the video is (or why it isn't).
    pub(super) fn finish_recording(&mut self) {
        let screen = self.record_monitor();
        let Some(b) = self.record.take() else {
            return;
        };
        self.refresh_tooltip();
        match b.stop() {
            None => tracing::info!("record box closed before recording"),
            Some(Ok(saved)) => self.recording_saved(screen, &saved),
            Some(Err(error)) => self.recording_failed(screen, &error),
        }
    }

    fn recording_saved(&mut self, screen: Option<Monitor>, saved: &Saved) {
        tracing::info!(
            path = %saved.path.display(),
            secs = saved.duration.as_secs_f64(),
            bytes = saved.bytes,
            ended_by = ?saved.ended_by,
            "recording saved"
        );
        let file = saved.path.display().to_string();
        let saved_text = tr!("toast-recording-saved", file = file.as_str());
        let (title, mut lines) = match &saved.ended_by {
            Some(why) => (why.message(), vec![saved_text]),
            None => (saved_text, Vec::new()),
        };
        lines.push(tr("toast-hint"));
        // Saved after it stopped by itself, the notice says why: that always shows.
        let kind = if saved.ended_by.is_some() {
            Notice::Problem
        } else {
            Notice::Saved
        };
        self.toast_on(kind, screen, &title, lines, Some(saved.path.clone()));
    }

    /// The tray's loop was woken for a recording: one that ended by itself is saved and told.
    pub(super) fn on_recording_woken(&mut self) {
        let ended = self
            .record
            .as_ref()
            .and_then(|b| b.recording())
            .is_some_and(|r| r.status() == Status::Ended);
        if ended {
            self.finish_recording();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HotkeyDoes, Notice, hotkey_does, notice_shows};

    /// "Show a notice after each capture" turned off hides only a plain "saved": a problem, a
    /// recording that stopped by itself and the drawing layer's instruction always show.
    #[test]
    fn only_a_plain_save_follows_the_notice_setting() {
        assert!(notice_shows(Notice::Saved, true));
        assert!(!notice_shows(Notice::Saved, false));
        assert!(notice_shows(Notice::Problem, true));
        assert!(notice_shows(Notice::Problem, false));
    }

    /// Pressed again, it ends whatever box is open (a recording, paused or asking whether to
    /// discard, is saved: it's "recording" all the same); it cancels a box being chosen; with
    /// the drawing layer open it only says so; otherwise it starts choosing.
    #[test]
    fn record_a_region_ends_what_is_open_and_never_starts_over_the_layer() {
        for choosing in [false, true] {
            for layer in [false, true] {
                assert_eq!(
                    hotkey_does(Some(true), choosing, layer),
                    HotkeyDoes::StopAndSave
                );
                assert_eq!(
                    hotkey_does(Some(false), choosing, layer),
                    HotkeyDoes::CloseBox
                );
            }
            assert_eq!(
                hotkey_does(None, true, choosing),
                HotkeyDoes::CancelChoosing
            );
        }
        assert_eq!(hotkey_does(None, false, true), HotkeyDoes::SayCloseTheLayer);
        assert_eq!(hotkey_does(None, false, false), HotkeyDoes::Choose);
    }
}
