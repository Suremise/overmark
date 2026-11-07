//! Recording a box on the screen to a small MP4: the engine both recording modes drive.
//!
//! [`Recording::start`] takes a box (virtual-desktop pixels, on one monitor), the settings and
//! the file to write, and records on a thread of its own until [`Recording::stop`] (which saves
//! and gives the file) or [`Recording::discard`] (which deletes it). [`Recording::pause`] and
//! [`Recording::resume`] leave no gap and no frozen stretch; [`Recording::elapsed`] is the time
//! recorded. A recording that ends by itself (the screen locked, the monitor unplugged, the
//! encoder failed) saves what it had, calls the `on_end` given at the start, and says why in
//! [`Recording::ended_by`]; [`Recording::stop`] then gives the file as usual.
//!
//! How it works (docs/ARCHITECTURE.md, Recording): Desktop Duplication gives the monitor's
//! image only when something changes; the box is cut from it on the GPU and read back. The
//! thread writes a frame each tick of the frame rate, from the latest image with the pointer
//! drawn in, turned into NV12, so a still screen repeats one buffer and the encoder spends
//! almost nothing on it. Media Foundation's H.264 encoder (the GPU's, else Microsoft's
//! software one) writes the MP4 through a sink writer, to `<file>.part` until it's finished.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use overmark_core::record::pointer::{self, Shape};
use overmark_core::record::{
    self as core_record, Clock, frame_duration, frame_time, frames_at_stop, frames_started, nv12,
};
pub use overmark_core::record::{FrameRate, PixelRect, Quality, RecordError, RecordSettings};

use crate::win32::Rect;
use crate::win32::capture::dd::{Duplication, PointerPosition};
use crate::win32::clock;
use crate::win32::dpi;
use crate::win32::mf::{self, EncoderInfo, FrameBuffer, MfError, Mp4Writer, VideoFormat};

/// Which H.264 encoder to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncoderChoice {
    /// The GPU's when it takes the video, else the software one.
    #[default]
    Auto,
    /// Microsoft's software encoder only.
    Software,
}

/// What to record, and where to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The box, in physical pixels on the virtual desktop. It must be on one monitor; it's cut
    /// to that monitor and its sides rounded down to even numbers.
    pub area: PixelRect,
    pub settings: RecordSettings,
    /// The MP4 to make. While recording it's `<path>.part`; if `path` is taken by the time the
    /// recording stops, the video gets the next free name (`… (2).mp4`).
    pub path: PathBuf,
    pub encoder: EncoderChoice,
}

impl Start {
    pub fn new(area: PixelRect, settings: RecordSettings, path: impl Into<PathBuf>) -> Start {
        Start {
            area,
            settings,
            path: path.into(),
            encoder: EncoderChoice::Auto,
        }
    }
}

/// A finished recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    /// The video's length (the time recorded, to the frame).
    pub duration: Duration,
    pub frames: u64,
    pub bytes: u64,
    /// What was recorded: the box as placed (on its monitor, even-sized).
    pub area: PixelRect,
    pub encoder: EncoderInfo,
    /// Why it ended before it was stopped, if it did.
    pub ended_by: Option<RecordError>,
}

/// Where a recording is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Recording,
    Paused,
    /// It ended by itself ([`Recording::ended_by`] says why); stop it to get the file.
    Ended,
}

/// What the caller asked the thread to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Save,
    Discard,
}

#[derive(Debug)]
struct State {
    clock: Clock,
    stop: Option<(Stop, i64)>,
    ended_by: Option<RecordError>,
    ended: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A recording in progress. Dropping it stops it and keeps the file, as [`Recording::stop`].
pub struct Recording {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<Saved, RecordError>>>,
    area: PixelRect,
    encoder: EncoderInfo,
}

/// The screen as the thread sees it: the box's latest image and the pointer, its position
/// relative to the box's top-left.
struct Scene {
    width: i32,
    height: i32,
    image: Vec<u8>,
    pointer: Option<PointerPosition>,
    shape: Option<Shape>,
}

/// Where frames come from: Desktop Duplication, or a test pattern.
trait Source {
    /// Waits up to `timeout_ms` for a change and puts it in `scene`; whether anything the
    /// video shows changed.
    fn next(&mut self, timeout_ms: u32, scene: &mut Scene) -> Result<bool, RecordError>;
}

type MakeSource = Box<dyn FnOnce() -> Result<Box<dyn Source>, RecordError> + Send>;

/// One monitor's Desktop Duplication, cut to the box.
struct ScreenSource {
    dup: Duplication,
    /// The box, relative to the monitor's top-left.
    area: Rect,
    pointer: bool,
}

impl Source for ScreenSource {
    fn next(&mut self, timeout_ms: u32, scene: &mut Scene) -> Result<bool, RecordError> {
        let change = self
            .dup
            .next_change(timeout_ms, self.area, &mut scene.image)
            .map_err(|e| RecordError::CaptureLost(e.message()))?;
        let mut changed = change.image;
        if self.pointer {
            if let Some(p) = change.pointer {
                let p = PointerPosition {
                    x: p.x - self.area.left,
                    y: p.y - self.area.top,
                    ..p
                };
                changed |= scene.pointer != Some(p);
                scene.pointer = Some(p);
            }
            if let Some(s) = change.shape {
                scene.shape = Some(s);
                changed = true;
            }
        }
        Ok(changed)
    }
}

impl Recording {
    /// Starts recording. Returns once the first image is in, so [`Recording::elapsed`] counts
    /// from what the video's first frame shows; or with why it can't record.
    pub fn start(
        start: Start,
        on_end: impl Fn() + Send + 'static,
    ) -> Result<Recording, RecordError> {
        let monitors = dpi::monitors();
        let rects: Vec<PixelRect> = monitors
            .iter()
            .map(|m| PixelRect::new(m.rect.left, m.rect.top, m.rect.right, m.rect.bottom))
            .collect();
        let placement = core_record::place(start.area, &rects)?;
        let monitor = monitors[placement.monitor].clone();
        let area = placement.area;
        let relative = Rect::new(
            area.left - monitor.rect.left,
            area.top - monitor.rect.top,
            area.right - monitor.rect.left,
            area.bottom - monitor.rect.top,
        );
        let pointer = start.settings.pointer;
        // A monitor handle is a plain number Windows checks; the thread looks it up again.
        let handle = monitor.handle.0 as isize;
        let make: MakeSource = Box::new(move || {
            let monitor = dpi::describe(windows::Win32::Graphics::Gdi::HMONITOR(handle as _))
                .ok_or_else(|| RecordError::CaptureUnavailable("the monitor went away".into()))?;
            let dup = Duplication::new(&monitor)
                .map_err(|e| RecordError::CaptureUnavailable(e.message()))?;
            Ok(Box::new(ScreenSource {
                dup,
                area: relative,
                pointer,
            }) as Box<dyn Source>)
        });
        Self::start_with(start, area, make, Box::new(on_end))
    }

    fn start_with(
        start: Start,
        area: PixelRect,
        make: MakeSource,
        on_end: Box<dyn Fn() + Send>,
    ) -> Result<Recording, RecordError> {
        if !mf::available() {
            return Err(RecordError::MediaFoundationMissing);
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                clock: Clock::new(clock::now_100ns()),
                stop: None,
                ended_by: None,
                ended: false,
            }),
            changed: Condvar::new(),
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("overmark-record".into())
            .spawn(move || run(thread_shared, start, area, make, on_end, ready_tx))
            .map_err(|e| RecordError::Encoding(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(encoder)) => Ok(Recording {
                shared,
                thread: Some(thread),
                area,
                encoder,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(match thread.join() {
                Ok(Err(e)) => e,
                _ => RecordError::Encoding("the recording thread stopped".into()),
            }),
        }
    }

    /// What is recorded: the box as placed.
    pub fn area(&self) -> PixelRect {
        self.area
    }

    /// The encoder in use.
    pub fn encoder(&self) -> &EncoderInfo {
        &self.encoder
    }

    /// The time recorded so far: real time less the pauses.
    pub fn elapsed(&self) -> Duration {
        let active = self.shared.lock().clock.active(clock::now_100ns());
        Duration::from_nanos(active as u64 * 100)
    }

    pub fn status(&self) -> Status {
        let state = self.shared.lock();
        if state.ended {
            Status::Ended
        } else if state.clock.is_paused() {
            Status::Paused
        } else {
            Status::Recording
        }
    }

    /// Why the recording ended by itself, if it did. What it had is saved all the same.
    pub fn ended_by(&self) -> Option<RecordError> {
        self.shared.lock().ended_by.clone()
    }

    /// Pauses: nothing is recorded until [`Recording::resume`], and the pause leaves no gap.
    /// A recording that has ended stays as it ended: pausing or resuming it changes nothing.
    pub fn pause(&self) {
        let mut state = self.shared.lock();
        if state.ended {
            return;
        }
        state.clock.pause(clock::now_100ns());
        self.shared.changed.notify_all();
    }

    pub fn resume(&self) {
        let mut state = self.shared.lock();
        if state.ended {
            return;
        }
        state.clock.resume(clock::now_100ns());
        self.shared.changed.notify_all();
    }

    /// Stops and saves: the MP4's path, length and size. If the recording ended by itself,
    /// this gives what it saved, with [`Saved::ended_by`] set.
    pub fn stop(mut self) -> Result<Saved, RecordError> {
        self.finish(Stop::Save)
            .unwrap_or_else(|| Err(RecordError::Encoding("already stopped".into())))
    }

    /// Stops and deletes the file.
    pub fn discard(mut self) {
        let _ = self.finish(Stop::Discard);
    }

    fn finish(&mut self, how: Stop) -> Option<Result<Saved, RecordError>> {
        let thread = self.thread.take()?;
        {
            let mut state = self.shared.lock();
            if state.stop.is_none() {
                state.stop = Some((how, clock::now_100ns()));
            }
            self.shared.changed.notify_all();
        }
        let result = thread.join().unwrap_or_else(|_| {
            Err(RecordError::Encoding(
                "the recording thread panicked".into(),
            ))
        });
        if how == Stop::Discard {
            // A recording that ended by itself has already saved its file: discarding deletes
            // that too.
            if let Ok(saved) = &result {
                let _ = std::fs::remove_file(&saved.path);
            }
        }
        Some(result)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.finish(Stop::Save);
    }
}

/// The `n`th name a recording of `path` may be written to until it's finished:
/// `<path>.part`, then `<path>.2.part`, `<path>.3.part` and so on.
fn part_path(path: &Path, n: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    if n > 1 {
        name.push(format!(".{n}"));
    }
    name.push(".part");
    PathBuf::from(name)
}

/// Makes the first free [`part_path`] for `path`, empty, with `create_new`, so it is this
/// recording's own: another recording's partial file, or one left from a failed finish, is
/// never replaced or deleted.
fn reserve_part(path: &Path) -> Result<PathBuf, RecordError> {
    for n in 1..=1000 {
        let part = part_path(path, n);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
        {
            Ok(_) => return Ok(part),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(file_error(&part, e)),
        }
    }
    Err(file_error(
        &part_path(path, 1),
        "no free name for the recording's partial file",
    ))
}

/// `path`, or the first of `… (2).ext`, `… (3).ext` and so on that's free.
fn free_path(path: &Path) -> PathBuf {
    if path.symlink_metadata().is_err() {
        return path.to_owned();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2u32..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| p.symlink_metadata().is_err())
        .expect("a free name")
}

fn file_error(path: &Path, e: impl std::fmt::Display) -> RecordError {
    RecordError::File {
        path: path.display().to_string(),
        detail: e.to_string(),
    }
}

/// Seconds from one key frame to the next.
const KEY_FRAME_SECS: u32 = 30;

/// The encoder's settings for a box `width` x `height`.
fn video_format(width: i32, height: i32, settings: &RecordSettings) -> VideoFormat {
    let average = core_record::bitrate(settings.quality, width, height, settings.frame_rate);
    let fps = settings.frame_rate.fps();
    VideoFormat {
        width: width as u32,
        height: height as u32,
        fps,
        bitrate: average,
        peak_bitrate: core_record::peak_bitrate(average),
        // A key frame every 30 s. Each is a whole picture, and on a still screen they are
        // nearly all the file: a minute of a still full screen is 8 MB with one every 10 s,
        // 3 MB at 30 s (docs/ARCHITECTURE.md, Recording). Players seek to the key frame before
        // and decode forward, which repeated frames make quick.
        gop: fps * KEY_FRAME_SECS,
        rate_control: mf::RateControl::PeakConstrainedVbr,
    }
}

/// The recording thread: set up, tell `ready`, record until stopped or lost, finish.
fn run(
    shared: Arc<Shared>,
    start: Start,
    area: PixelRect,
    make: MakeSource,
    on_end: Box<dyn Fn() + Send>,
    ready: std::sync::mpsc::Sender<Result<EncoderInfo, RecordError>>,
) -> Result<Saved, RecordError> {
    let (width, height) = (area.width(), area.height());
    let settings = start.settings;
    let part = match reserve_part(&start.path) {
        Ok(p) => p,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    // From here on `part` is this recording's own file: the writer replaces it (it's empty),
    // and a failure deletes it.
    let setup = || -> Result<(Box<dyn Source>, Mp4Writer, Scene), RecordError> {
        let mut source = make()?;
        let format = video_format(width, height, &settings);
        let writer = Mp4Writer::create(&part, &format, start.encoder == EncoderChoice::Auto)
            .map_err(|e| match e {
                MfError::Missing => RecordError::MediaFoundationMissing,
                MfError::Refused(e) => RecordError::EncoderRefused {
                    width,
                    height,
                    detail: e.message(),
                },
                MfError::Failed(e) => RecordError::Encoding(e.message()),
            })?;
        let mut scene = Scene {
            width,
            height,
            image: Vec::new(),
            pointer: None,
            shape: None,
        };
        // The first image: Desktop Duplication gives the screen as it is on its first frame.
        let deadline = clock::now_secs() + 2.0;
        while scene.image.len() != (width * height * 4) as usize {
            if clock::now_secs() > deadline {
                return Err(RecordError::CaptureUnavailable(
                    "no image from the screen".into(),
                ));
            }
            source.next(500, &mut scene)?;
        }
        Ok((source, writer, scene))
    };
    let (mut source, mut writer, mut scene) = match setup() {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let encoder = writer.encoder.clone();
    {
        // The clock starts with the first image.
        let mut state = shared.lock();
        state.clock = Clock::new(clock::now_100ns());
    }
    let _ = ready.send(Ok(encoder.clone()));

    let rate = settings.frame_rate;
    let fps = u64::from(rate.fps());
    let mut written = 0u64;
    let mut frame: Option<FrameBuffer> = None;
    let mut dirty = true;
    let mut work = Vec::new();
    let mut yuv = vec![0u8; nv12::len(width as usize, height as usize)];
    let mut ended_by: Option<RecordError> = None;

    let mut compose = |scene: &Scene, writer: &Mp4Writer| -> Result<FrameBuffer, RecordError> {
        work.clear();
        work.extend_from_slice(&scene.image);
        if settings.pointer
            && let (Some(p), Some(shape)) = (scene.pointer, &scene.shape)
            && p.visible
        {
            pointer::draw(shape, p.x, p.y, &mut work, scene.width, scene.height);
        }
        nv12::from_bgra(&work, width as usize, height as usize, &mut yuv);
        writer
            .frame(&yuv)
            .map_err(|e| RecordError::Encoding(e.message()))
    };

    // Writes frames `written..upto`, the current scene; more than a second behind (the PC
    // stalled), the gap is one long frame rather than a burst of repeats.
    let mut write_upto = |upto: u64,
                          written: &mut u64,
                          frame: &mut Option<FrameBuffer>,
                          dirty: &mut bool,
                          scene: &Scene,
                          writer: &mut Mp4Writer|
     -> Result<(), RecordError> {
        if upto <= *written {
            return Ok(());
        }
        if *dirty || frame.is_none() {
            *frame = Some(compose(scene, writer)?);
            *dirty = false;
        }
        let buffer = frame.as_ref().expect("a frame");
        let encode = |e: windows::core::Error| RecordError::Encoding(e.message());
        if upto - *written > fps {
            let t = frame_time(*written, rate);
            writer
                .write(buffer, t, frame_time(upto, rate) - t)
                .map_err(encode)?;
        } else {
            for n in *written..upto {
                writer
                    .write(buffer, frame_time(n, rate), frame_duration(n, rate))
                    .map_err(encode)?;
            }
        }
        *written = upto;
        Ok(())
    };

    let (how, stop_at) = loop {
        let active = {
            let mut state = shared.lock();
            while state.clock.is_paused() && state.stop.is_none() {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if let Some(stop) = state.stop {
                break stop;
            }
            state.clock.active(clock::now_100ns())
        };
        let due = frames_started(active, rate);
        if due > written {
            if let Err(e) = write_upto(
                due,
                &mut written,
                &mut frame,
                &mut dirty,
                &scene,
                &mut writer,
            ) {
                ended_by = Some(e);
                break (Stop::Save, clock::now_100ns());
            }
            continue;
        }
        let wait_100ns = frame_time(written, rate) - active;
        let timeout_ms = (wait_100ns.div_euclid(10_000) + 1).clamp(1, 1000) as u32;
        match source.next(timeout_ms, &mut scene) {
            Ok(changed) => dirty |= changed,
            Err(e) => {
                ended_by = Some(e);
                break (Stop::Save, clock::now_100ns());
            }
        }
    };

    // The last frames, up to the stop.
    let active = shared.lock().clock.active(stop_at);
    let total = frames_at_stop(active, rate);
    let mut finish_error = None;
    if how == Stop::Save
        && let Err(e) = write_upto(
            total,
            &mut written,
            &mut frame,
            &mut dirty,
            &scene,
            &mut writer,
        )
    {
        finish_error = Some(e);
    }
    drop(frame);
    let finished = writer.finish();
    drop(source);

    let result = if how == Stop::Discard {
        let _ = std::fs::remove_file(&part);
        Err(RecordError::Encoding("discarded".into()))
    } else if let Err(e) = finished {
        // The file can't play without its index: say so, keep the part for what it's worth.
        Err(RecordError::Encoding(e.message()))
    } else {
        let path = free_path(&start.path);
        match std::fs::rename(&part, &path) {
            Err(e) => Err(file_error(&path, e)),
            Ok(()) => {
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let duration_100ns = frame_time(written, rate);
                Ok(Saved {
                    path,
                    duration: Duration::from_nanos(duration_100ns as u64 * 100),
                    frames: written,
                    bytes,
                    area,
                    encoder,
                    ended_by: ended_by.clone().or(finish_error),
                })
            }
        }
    };

    if ended_by.is_some() {
        tracing::warn!(error = ?ended_by, "recording: ended by itself");
        let mut state = shared.lock();
        // The time recorded stops where the recording did.
        state.clock.pause(stop_at);
        state.ended = true;
        state.ended_by = ended_by;
        drop(state);
        on_end();
    }
    match &result {
        Ok(saved) => tracing::info!(
            path = %saved.path.display(),
            frames = saved.frames,
            bytes = saved.bytes,
            secs = saved.duration.as_secs_f64(),
            "recording: saved"
        ),
        Err(e) if how == Stop::Save => tracing::warn!(?e, "recording: not saved"),
        Err(_) => tracing::info!("recording: discarded"),
    }
    result
}

#[cfg(test)]
mod measure;
#[cfg(test)]
mod tests;
