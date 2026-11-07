//! Recording a region of the screen to an MP4: the platform-free part.
//!
//! The Windows layer (`overmark_win::record`) grabs the screen with Desktop Duplication and
//! encodes it with Media Foundation's H.264 encoder. Everything that can be worked out without
//! Windows is here, with its tests: where a box may be recorded and at what size
//! ([`place`]), the frame rates and qualities and the bitrate each gives ([`bitrate`]), the
//! recording's clock with its pauses ([`Clock`]), turning BGRA pixels into the encoder's NV12
//! ([`nv12`]) and drawing the mouse pointer into a frame ([`pointer`]).
//!
//! Times are in 100 ns units, Media Foundation's, from the performance counter.

pub mod nv12;
pub mod pointer;

use serde::{Deserialize, Serialize};

use crate::tr;

/// 100 ns units in a second.
pub const UNITS_PER_SEC: i64 = 10_000_000;

/// A rectangle in physical pixels on the virtual desktop: `left`/`top` inclusive,
/// `right`/`bottom` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// Where both overlap, or `None`.
    pub fn intersect(&self, other: &PixelRect) -> Option<PixelRect> {
        let r = PixelRect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!r.is_empty()).then_some(r)
    }

    /// This rectangle moved by (`dx`, `dy`).
    pub const fn offset(&self, dx: i32, dy: i32) -> PixelRect {
        PixelRect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }
}

/// The smallest side a recording may have, in pixels: Microsoft's software H.264 encoder
/// refuses a side of 32 or less (measured on Windows 11), and the GPUs' encoders want more.
pub const MIN_SIDE: i32 = 34;

/// Where a box is recorded: on which monitor, and the part of it that is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The index of the monitor in the list [`place`] was given.
    pub monitor: usize,
    /// What is recorded, on the virtual desktop: the box cut to its monitor, its width and
    /// height rounded down to even numbers (H.264's 4:2:0 needs them), keeping its top-left.
    pub area: PixelRect,
}

/// Why a box can't be recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceError {
    /// The box is on two or more monitors: a recording is of one monitor.
    SpansMonitors,
    /// The box is on no monitor.
    OffScreen,
    /// Cut to its monitor and made even, a side is under [`MIN_SIDE`].
    TooSmall,
}

/// Where `area` (virtual-desktop pixels) is recorded, given the monitors' rectangles.
pub fn place(area: PixelRect, monitors: &[PixelRect]) -> Result<Placement, PlaceError> {
    let mut on = monitors
        .iter()
        .enumerate()
        .filter_map(|(i, m)| area.intersect(m).map(|r| (i, r)));
    let (monitor, cut) = on.next().ok_or(PlaceError::OffScreen)?;
    if on.next().is_some() {
        return Err(PlaceError::SpansMonitors);
    }
    let (w, h) = (even_down(cut.width()), even_down(cut.height()));
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(PlaceError::TooSmall);
    }
    Ok(Placement {
        monitor,
        area: PixelRect::new(cut.left, cut.top, cut.left + w, cut.top + h),
    })
}

/// `n` rounded down to an even number.
pub const fn even_down(n: i32) -> i32 {
    n & !1
}

/// Frames a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FrameRate {
    #[default]
    #[serde(rename = "15")]
    Fps15,
    #[serde(rename = "24")]
    Fps24,
    #[serde(rename = "30")]
    Fps30,
}

impl FrameRate {
    pub const ALL: [FrameRate; 3] = [FrameRate::Fps15, FrameRate::Fps24, FrameRate::Fps30];

    pub const fn fps(self) -> u32 {
        match self {
            FrameRate::Fps15 => 15,
            FrameRate::Fps24 => 24,
            FrameRate::Fps30 => 30,
        }
    }

    /// The rate nearest `fps` (the lower of two as near).
    pub fn nearest(fps: u32) -> FrameRate {
        FrameRate::ALL
            .into_iter()
            .min_by_key(|r| r.fps().abs_diff(fps))
            .unwrap_or_default()
    }
}

/// Which H.264 encoder records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoder {
    /// The graphics card's when it takes the video, otherwise Windows' software one.
    #[default]
    Automatic,
    /// Windows' software encoder only.
    Software,
}

/// How sharp the video is, against how big the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Small,
    #[default]
    Balanced,
    Sharp,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Small, Quality::Balanced, Quality::Sharp];

    /// Bits for each pixel of each frame at the encoder's average: screen content (text, flat
    /// colours, little motion) needs far less than camera video's 0.1 to 0.2.
    const fn bits_per_pixel_milli(self) -> u64 {
        match self {
            Quality::Small => 40,
            Quality::Balanced => 70,
            Quality::Sharp => 120,
        }
    }
}

/// How a recording is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSettings {
    pub frame_rate: FrameRate,
    pub quality: Quality,
    /// Draw the mouse pointer into the video.
    pub pointer: bool,
}

impl Default for RecordSettings {
    fn default() -> Self {
        Self {
            frame_rate: FrameRate::Fps15,
            quality: Quality::Balanced,
            pointer: true,
        }
    }
}

/// The lowest and highest average bitrates [`bitrate`] gives, in bits a second. The floor keeps
/// a tiny box legible; the ceiling is well inside H.264 level 5.1's limit.
pub const MIN_BITRATE: u32 = 250_000;
pub const MAX_BITRATE: u32 = 40_000_000;

/// The encoder's average bitrate, in bits a second, for a `width` x `height` video at `rate`:
/// it grows with the area and the frame rate. It's an average for a capped variable bitrate
/// (peak twice this), not a constant one: a still screen costs almost nothing, and the bits go
/// to the moments that change (docs/ARCHITECTURE.md, Recording).
pub fn bitrate(quality: Quality, width: i32, height: i32, rate: FrameRate) -> u32 {
    let pixels = width.max(0) as u64 * height.max(0) as u64;
    let bits = pixels * u64::from(rate.fps()) * quality.bits_per_pixel_milli() / 1000;
    bits.clamp(u64::from(MIN_BITRATE), u64::from(MAX_BITRATE)) as u32
}

/// Where the record controls go (their top-left), for a bar `bar` (width, height) over `area`
/// on `monitor`, `gap` pixels from the box's edge: always on the box's top edge, **just above
/// it when it fits on the monitor, just inside it when it doesn't** (a full-screen box, a box
/// at the monitor's top). Never anywhere else. It starts at the box's left edge; a box
/// narrower than the bar has it overhang to the right, and it moves left only as far as it
/// must to stay on the monitor.
pub fn bar_origin(area: PixelRect, bar: (i32, i32), monitor: PixelRect, gap: i32) -> (i32, i32) {
    let (w, h) = bar;
    let above = area.top - gap - h;
    let y = if above >= monitor.top {
        above
    } else {
        // Inside, kept on the monitor even for a box too short to hold it.
        (area.top + gap).min(monitor.bottom - h).max(monitor.top)
    };
    let x = area.left.min(monitor.right - w).max(monitor.left);
    (x, y)
}

/// A recording's elapsed time as its timer shows it: `0:07`, `12:34`, `1:02:03`. Whole
/// seconds, rounded down, so it ticks over as each second is done.
pub fn timer_text(elapsed: std::time::Duration) -> String {
    let s = elapsed.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The highest bitrate the encoder may reach for a moment, for a given average.
pub const fn peak_bitrate(average: u32) -> u32 {
    average.saturating_mul(2)
}

/// The start time of frame `n` at `rate`, in 100 ns units from the recording's start. Exact for
/// every rate: 15 fps isn't a whole number of 100 ns units a frame.
pub fn frame_time(n: u64, rate: FrameRate) -> i64 {
    (n as i128 * UNITS_PER_SEC as i128 / rate.fps() as i128) as i64
}

/// How long frame `n` lasts: the gap to the next one's start.
pub fn frame_duration(n: u64, rate: FrameRate) -> i64 {
    frame_time(n + 1, rate) - frame_time(n, rate)
}

/// How many frames have started by `active` (100 ns of recording): every `n` whose
/// [`frame_time`] is at or before it.
pub fn frames_started(active: i64, rate: FrameRate) -> u64 {
    if active < 0 {
        return 0;
    }
    // frame_time(n) <= active  <=>  n * UNITS / fps < active + 1, as frame_time floors.
    starting_before(active + 1, rate)
}

/// How many frames start before `t`: every `n` with `n * UNITS / fps < t`.
fn starting_before(t: i64, rate: FrameRate) -> u64 {
    (t.max(0) as u128 * u128::from(rate.fps())).div_ceil(UNITS_PER_SEC as u128) as u64
}

/// How many frames a recording `active` long holds when it stops: every frame that starts
/// before its end, and at least one.
pub fn frames_at_stop(active: i64, rate: FrameRate) -> u64 {
    let n = starting_before(active, rate);
    if n == 0 { 1 } else { n }
}

/// A recording's clock: real time (the performance counter, in 100 ns) less the time spent
/// paused, so a pause leaves neither a gap nor a frozen stretch in the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    start: i64,
    paused_total: i64,
    paused_at: Option<i64>,
}

impl Clock {
    /// A clock started at `now`.
    pub const fn new(now: i64) -> Clock {
        Clock {
            start: now,
            paused_total: 0,
            paused_at: None,
        }
    }

    /// The recorded time at `now`: the time since the start, less every pause. It stands still
    /// while paused.
    pub fn active(&self, now: i64) -> i64 {
        let end = self.paused_at.unwrap_or(now);
        (end - self.start - self.paused_total).max(0)
    }

    pub const fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Pauses at `now`. Pausing a paused clock changes nothing.
    pub fn pause(&mut self, now: i64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    /// Resumes at `now`: the pause's length is taken off every later time. Resuming a running
    /// clock changes nothing.
    pub fn resume(&mut self, now: i64) {
        if let Some(at) = self.paused_at.take() {
            self.paused_total += (now - at).max(0);
        }
    }
}

/// Why a recording didn't start, or ended by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// See [`PlaceError`].
    Place(PlaceError),
    /// Media Foundation isn't installed: Windows N and KN editions without the Media Feature
    /// Pack.
    MediaFoundationMissing,
    /// The H.264 encoder won't take a video this size.
    EncoderRefused {
        width: i32,
        height: i32,
        detail: String,
    },
    /// The screen can't be captured (the secure desktop is up, say).
    CaptureUnavailable(String),
    /// The screen stopped being captured while recording: the screen was locked, the display
    /// mode changed, or the monitor was unplugged. What was recorded is saved.
    CaptureLost(String),
    /// The encoder or the file writer failed.
    Encoding(String),
    /// The file couldn't be made, written or moved into place.
    File { path: String, detail: String },
}

impl From<PlaceError> for RecordError {
    fn from(e: PlaceError) -> Self {
        RecordError::Place(e)
    }
}

impl RecordError {
    /// What went wrong, in the user's language.
    pub fn message(&self) -> String {
        match self {
            RecordError::Place(PlaceError::SpansMonitors) => tr("record-error-two-monitors"),
            RecordError::Place(PlaceError::OffScreen) => tr("record-error-off-screen"),
            RecordError::Place(PlaceError::TooSmall) => {
                tr!("record-error-too-small", min = MIN_SIDE)
            }
            RecordError::MediaFoundationMissing => tr("record-error-no-media-foundation"),
            RecordError::EncoderRefused {
                width,
                height,
                detail,
            } => tr!(
                "record-error-encoder-refused",
                width = *width,
                height = *height,
                reason = detail.as_str()
            ),
            RecordError::CaptureUnavailable(detail) => {
                tr!("record-error-capture-unavailable", reason = detail.as_str())
            }
            RecordError::CaptureLost(detail) => {
                tr!("record-error-capture-lost", reason = detail.as_str())
            }
            RecordError::Encoding(detail) => {
                tr!("record-error-encoding", reason = detail.as_str())
            }
            RecordError::File { path, detail } => tr!(
                "record-error-file",
                path = path.as_str(),
                reason = detail.as_str()
            ),
        }
    }
}

fn tr(key: &str) -> String {
    crate::i18n::tr(key)
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for RecordError {}

#[cfg(test)]
mod tests;
