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

use overmark_core::i18n::tr;
use overmark_core::record::{Encoder, PixelRect, RecordError, place};
use overmark_core::tr;
use overmark_win::record::{EncoderChoice, Saved, Start, Status};
use overmark_win::win32::{Rect, shell};

use super::App;
use crate::record_bar::{BarAction, RecordBox};

/// The tray's wake code for a recording that ended by itself.
pub(super) const WAKE_RECORDING: u32 = 2;

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

    /// The box was chosen on monitor `screen` (`rect` in its pixels): its outline and bar.
    pub(super) fn record_box_chosen(&mut self, screen: usize, rect: Rect) {
        self.pending_record = false;
        self.pending_region = None;
        let Some(overlay) = self.overlay.as_ref() else {
            return;
        };
        let monitors = overlay.monitors();
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
                self.toast_text(screen, &error.message(), Vec::new(), None, None);
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
                    "a box to record"
                );
                self.record = Some(b);
            }
            Err(error) => {
                tracing::error!(%error, "couldn't show the record box");
                let text = tr!("toast-recording-failed", reason = error.message());
                self.toast_text(screen, &text, Vec::new(), None, None);
            }
        }
    }

    /// The bar's input, a frame when one is due, and what it asked for.
    pub(super) fn on_record(&mut self) {
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
                }
            }
        }
    }

    /// The bar's Record: the recording starts, into the captures folder.
    fn start_recording(&mut self) {
        let folder = self.captures_folder();
        let screen = self.record_screen();
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
            Ok(()) => tracing::info!("recording started"),
            Err(error) => {
                // Nothing was recorded: the box stays, so Record can be tried again.
                self.recording_failed(screen, &error);
            }
        }
    }

    /// The monitor the box is on, as the overlay counts them (for the notice).
    fn record_screen(&self) -> usize {
        let (Some(b), Some(o)) = (self.record.as_ref(), self.overlay.as_ref()) else {
            return 0;
        };
        let a = b.area();
        o.screen_at(a.left, a.top)
    }

    fn recording_failed(&mut self, screen: usize, error: &RecordError) {
        tracing::warn!(?error, "couldn't record");
        let text = tr!("toast-recording-failed", reason = error.message());
        self.toast_text(screen, &text, Vec::new(), None, None);
    }

    /// Esc or ✕ before recording: the box goes, the layer stays.
    pub(super) fn cancel_record_box(&mut self) {
        if self.record.take().is_some() {
            tracing::info!("record box cancelled");
        }
    }

    /// Whether Esc should cancel the box (it's up and nothing is being recorded).
    pub(super) fn esc_cancels_record_box(&self) -> bool {
        self.record.as_ref().is_some_and(|b| !b.is_recording())
    }

    /// Stop: saves, the box goes, and the notice says where the video is (or why it isn't).
    pub(super) fn finish_recording(&mut self) {
        let screen = self.record_screen();
        let Some(b) = self.record.take() else {
            return;
        };
        match b.stop() {
            None => tracing::info!("record box closed before recording"),
            Some(Ok(saved)) => self.recording_saved(screen, &saved),
            Some(Err(error)) => self.recording_failed(screen, &error),
        }
    }

    fn recording_saved(&mut self, screen: usize, saved: &Saved) {
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
        let folder = saved.path.parent().map(|p| p.to_path_buf());
        self.toast_text(screen, &title, lines, Some(saved.path.clone()), folder);
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
