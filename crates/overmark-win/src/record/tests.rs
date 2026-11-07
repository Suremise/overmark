//! The engine with a test pattern for a screen: the real thread, clock, NV12 and Media
//! Foundation encoder, read back with Media Foundation. Nothing on screen, no input.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use super::*;
use overmark_core::record::UNITS_PER_SEC;

/// The pattern's left half, BGRA: (R 200, G 100, B 50).
const LEFT: [u8; 4] = [50, 100, 200, 255];

/// A test pattern: the left half [`LEFT`], the right half black with a white bar that moves
/// every 100 ms. After `fail_after`, if set, it fails as a lost screen would.
struct Pattern {
    width: i32,
    height: i32,
    /// Its first frame, when it was asked for one: the times below count from there.
    started: Option<Instant>,
    step: Option<u64>,
    fail_after: Option<Duration>,
}

impl Pattern {
    fn new(width: i32, height: i32) -> Pattern {
        Pattern {
            width,
            height,
            started: None,
            step: None,
            fail_after: None,
        }
    }

    fn draw(&self, step: u64, out: &mut Vec<u8>) {
        let (w, h) = (self.width as usize, self.height as usize);
        out.clear();
        out.reserve(w * h * 4);
        let half = w / 2;
        let bar = half + (step as usize * 4) % half.max(1);
        for _ in 0..h {
            for x in 0..w {
                let px = if x < half {
                    LEFT
                } else if (bar..bar + 4).contains(&x) {
                    [255, 255, 255, 255]
                } else {
                    [0, 0, 0, 255]
                };
                out.extend_from_slice(&px);
            }
        }
    }
}

impl Source for Pattern {
    fn next(&mut self, timeout_ms: u32, scene: &mut Scene) -> Result<bool, RecordError> {
        let since = self.started.get_or_insert_with(Instant::now).elapsed();
        if self.fail_after.is_some_and(|f| since >= f) {
            return Err(RecordError::CaptureLost("the test screen went away".into()));
        }
        let step = since.as_millis() as u64 / 100;
        if self.step != Some(step) {
            self.step = Some(step);
            self.draw(step, &mut scene.image);
            return Ok(true);
        }
        // Nothing changes until the next step: wait for it, or the timeout.
        let next = Duration::from_millis((step + 1) * 100);
        let wait = next
            .saturating_sub(since)
            .min(Duration::from_millis(u64::from(timeout_ms)));
        std::thread::sleep(wait);
        Ok(false)
    }
}

fn start_pattern(
    dir: &Path,
    width: i32,
    height: i32,
    settings: RecordSettings,
    fail_after: Option<Duration>,
    on_end: Box<dyn Fn() + Send>,
) -> Result<Recording, RecordError> {
    let area = PixelRect::new(0, 0, width, height);
    let mut start = Start::new(area, settings, dir.join("clip.mp4"));
    start.encoder = EncoderChoice::Software;
    let make: MakeSource = Box::new(move || {
        let mut p = Pattern::new(width, height);
        p.fail_after = fail_after;
        Ok(Box::new(p) as Box<dyn Source>)
    });
    Recording::start_with(start, area, make, on_end)
}

/// Waits until `rec` has recorded `at_least`.
fn record_for(rec: &Recording, at_least: Duration) {
    while rec.elapsed() < at_least {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn secs(units: i64) -> f64 {
    units as f64 / UNITS_PER_SEC as f64
}

#[test]
fn records_a_pattern_to_an_mp4_that_plays_back() {
    let dir = tempfile::tempdir().unwrap();
    let rec = start_pattern(
        dir.path(),
        64,
        48,
        RecordSettings::default(),
        None,
        Box::new(|| {}),
    )
    .unwrap();
    assert_eq!(rec.status(), Status::Recording);
    assert!(!rec.encoder().hardware);
    record_for(&rec, Duration::from_millis(1200));
    let elapsed = rec.elapsed();
    let saved = rec.stop().unwrap();

    assert_eq!(saved.path, dir.path().join("clip.mp4"));
    assert!(!part_path(&saved.path, 1).exists(), "the .part was left");
    assert_eq!(saved.ended_by, None);
    assert_eq!(saved.bytes, std::fs::metadata(&saved.path).unwrap().len());
    // Every frame of 15 a second, up to the stop.
    let period = 1.0 / 15.0;
    assert_eq!(
        saved.frames,
        (saved.duration.as_secs_f64() / period).round() as u64
    );
    let late = saved.duration.as_secs_f64() - elapsed.as_secs_f64();
    assert!(
        (-0.001..0.5).contains(&late),
        "{:?} recorded, {elapsed:?} elapsed",
        saved.duration
    );

    let info = mf::probe(&saved.path).unwrap();
    assert!(info.h264);
    assert_eq!((info.width, info.height), (64, 48));
    assert_eq!(info.frames, saved.frames);
    assert!(
        (secs(info.duration_100ns) - saved.duration.as_secs_f64()).abs() < 0.002,
        "the file says {} s, the recording {:?}",
        secs(info.duration_100ns),
        saved.duration
    );
    assert!(info.key_frames >= 1);

    // The colours come back as they went in: BT.709, limited range, both ways.
    let (w, h, bgra) = mf::decode_frame_at(&saved.path, UNITS_PER_SEC / 2).unwrap();
    assert_eq!((w, h), (64, 48));
    let at = |x: usize, y: usize| {
        let i = (y * w as usize + x) * 4;
        [bgra[i], bgra[i + 1], bgra[i + 2]]
    };
    for (x, y) in [(4, 4), (16, 24), (28, 44)] {
        let p = at(x, y);
        let near = (0..3).all(|c| p[c].abs_diff(LEFT[c]) <= 8);
        assert!(near, "({x}, {y}) is {p:?}, not {:?}", &LEFT[..3]);
    }
}

#[test]
fn a_pause_leaves_no_gap() {
    let dir = tempfile::tempdir().unwrap();
    let wall = Instant::now();
    let rec = start_pattern(
        dir.path(),
        48,
        48,
        RecordSettings {
            frame_rate: FrameRate::Fps30,
            ..RecordSettings::default()
        },
        None,
        Box::new(|| {}),
    )
    .unwrap();
    record_for(&rec, Duration::from_millis(400));
    rec.pause();
    assert_eq!(rec.status(), Status::Paused);
    let paused_at = rec.elapsed();
    // The pause itself is what's tested: real time has to pass while it lasts.
    let pause = Instant::now();
    while pause.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rec.elapsed(), paused_at, "the clock moved while paused");
    rec.resume();
    assert_eq!(rec.status(), Status::Recording);
    record_for(&rec, Duration::from_millis(800));
    let saved = rec.stop().unwrap();
    let wall = wall.elapsed().as_secs_f64();

    let recorded = saved.duration.as_secs_f64();
    assert!(
        (0.8..1.3).contains(&recorded),
        "{recorded} s recorded: the pause wasn't left out"
    );
    assert!(
        wall >= recorded + 0.45,
        "{wall} s wall, {recorded} s recorded"
    );
    let info = mf::probe(&saved.path).unwrap();
    assert_eq!(info.frames, saved.frames);
    assert_eq!(
        saved.frames,
        (recorded * 30.0).round() as u64,
        "frames should be evenly spaced at 30 fps"
    );
}

#[test]
fn discard_deletes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let rec = start_pattern(
        dir.path(),
        48,
        48,
        RecordSettings::default(),
        None,
        Box::new(|| {}),
    )
    .unwrap();
    record_for(&rec, Duration::from_millis(200));
    rec.discard();
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn a_lost_screen_ends_the_recording_and_keeps_what_was_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (ended_tx, ended_rx) = std::sync::mpsc::channel();
    let ended_tx = Mutex::new(ended_tx);
    let rec = start_pattern(
        dir.path(),
        48,
        48,
        RecordSettings::default(),
        Some(Duration::from_millis(600)),
        Box::new(move || {
            let _ = ended_tx.lock().unwrap().send(());
        }),
    )
    .unwrap();
    // on_end is the signal: by the time it's called, the recording says it has ended.
    ended_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("on_end wasn't called");
    assert_eq!(rec.status(), Status::Ended);
    assert!(matches!(rec.ended_by(), Some(RecordError::CaptureLost(_))));
    // Ended, the time recorded stands still, and is what the file holds.
    let at_end = rec.elapsed();
    let later = Instant::now();
    while later.elapsed() < Duration::from_millis(100) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(rec.elapsed(), at_end);
    // Ended is final: a resume (or pause) racing the loss changes nothing.
    rec.resume();
    rec.pause();
    rec.resume();
    let later = Instant::now();
    while later.elapsed() < Duration::from_millis(100) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        rec.elapsed(),
        at_end,
        "resume restarted an ended recording's clock"
    );
    assert_eq!(rec.status(), Status::Ended);
    let saved = rec.stop().unwrap();
    assert!(
        saved.duration.abs_diff(at_end) <= Duration::from_millis(67),
        "{saved:?} vs {at_end:?}"
    );
    assert!(matches!(saved.ended_by, Some(RecordError::CaptureLost(_))));
    // The screen went 600 ms after the first frame; the clock starts just after that frame.
    assert!(saved.duration >= Duration::from_millis(300), "{saved:?}");
    let info = mf::probe(&saved.path).unwrap();
    assert_eq!(info.frames, saved.frames);
}

#[test]
fn a_size_the_encoder_refuses_is_said_plainly() {
    let dir = tempfile::tempdir().unwrap();
    // Far past H.264's largest level: the software encoder refuses it.
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let err = start_pattern(
        dir.path(),
        16_384,
        16_384,
        RecordSettings::default(),
        None,
        Box::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .err()
    .expect("refused");
    assert!(
        matches!(
            err,
            RecordError::EncoderRefused {
                width: 16_384,
                height: 16_384,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "on_end is for a running recording"
    );
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn a_box_off_every_monitor_is_refused_before_anything_starts() {
    let dir = tempfile::tempdir().unwrap();
    let far = PixelRect::new(-1_000_000, -1_000_000, -999_000, -999_000);
    let err = Recording::start(
        Start::new(far, RecordSettings::default(), dir.path().join("x.mp4")),
        || {},
    )
    .err()
    .expect("refused");
    assert_eq!(
        err,
        RecordError::Place(overmark_core::record::PlaceError::OffScreen)
    );
}

#[test]
fn a_taken_name_gets_the_next_free_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-11-07 10.00.00.mp4");
    assert_eq!(free_path(&path), path);
    std::fs::write(&path, b"").unwrap();
    let second = dir.path().join("2026-11-07 10.00.00 (2).mp4");
    assert_eq!(free_path(&path), second);
    std::fs::write(&second, b"").unwrap();
    assert_eq!(
        free_path(&path),
        dir.path().join("2026-11-07 10.00.00 (3).mp4")
    );
    assert_eq!(
        part_path(&path, 1),
        dir.path().join("2026-11-07 10.00.00.mp4.part")
    );
    assert_eq!(
        part_path(&path, 2),
        dir.path().join("2026-11-07 10.00.00.mp4.2.part")
    );
}

#[test]
fn the_video_format_follows_the_settings() {
    let f = video_format(
        640,
        480,
        &RecordSettings {
            frame_rate: FrameRate::Fps24,
            quality: Quality::Sharp,
            pointer: false,
        },
    );
    assert_eq!((f.width, f.height, f.fps, f.gop), (640, 480, 24, 720));
    assert_eq!(
        f.bitrate,
        core_record::bitrate(Quality::Sharp, 640, 480, FrameRate::Fps24)
    );
    assert_eq!(f.peak_bitrate, 2 * f.bitrate);
}

/// A start whose source can't start, for a failed setup.
fn failing_start(dir: &Path) -> Result<Recording, RecordError> {
    let area = PixelRect::new(0, 0, 48, 48);
    let mut start = Start::new(area, RecordSettings::default(), dir.join("clip.mp4"));
    start.encoder = EncoderChoice::Software;
    let make: MakeSource = Box::new(|| {
        Err(RecordError::CaptureUnavailable(
            "no screen in this test".into(),
        ))
    });
    Recording::start_with(start, area, make, Box::new(|| {}))
}

const THEIRS: &[u8] = b"another recording";

#[test]
fn a_failed_start_leaves_another_recordings_partial_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let theirs = dir.path().join("clip.mp4.part");
    std::fs::write(&theirs, THEIRS).unwrap();
    let err = failing_start(dir.path()).err().expect("it can't start");
    assert!(matches!(err, RecordError::CaptureUnavailable(_)), "{err:?}");
    assert_eq!(std::fs::read(&theirs).unwrap(), THEIRS);
    // Its own partial file, made for this attempt, is gone.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["clip.mp4.part"], "left behind: {names:?}");
}

#[test]
fn a_recording_beside_a_partial_file_writes_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let theirs = dir.path().join("clip.mp4.part");
    let ours = dir.path().join("clip.mp4.2.part");
    std::fs::write(&theirs, THEIRS).unwrap();
    let rec = start_pattern(
        dir.path(),
        48,
        48,
        RecordSettings::default(),
        None,
        Box::new(|| {}),
    )
    .unwrap();
    // While recording, it writes beside the other file, not over it.
    assert!(ours.exists());
    assert_eq!(std::fs::read(&theirs).unwrap(), THEIRS);
    record_for(&rec, Duration::from_millis(200));
    let saved = rec.stop().unwrap();
    assert_eq!(saved.path, dir.path().join("clip.mp4"));
    assert_eq!(mf::probe(&saved.path).unwrap().frames, saved.frames);
    assert_eq!(std::fs::read(&theirs).unwrap(), THEIRS);
    assert!(!ours.exists());

    // Discarding deletes only its own partial file too.
    let rec = start_pattern(
        dir.path(),
        48,
        48,
        RecordSettings::default(),
        None,
        Box::new(|| {}),
    )
    .unwrap();
    assert!(ours.exists());
    rec.discard();
    assert_eq!(std::fs::read(&theirs).unwrap(), THEIRS);
    assert!(!ours.exists());
}

/// A clip of the test pattern for checking playback in other players (a browser, Windows'
/// Media Player): `OVERMARK_SAMPLE_CLIP=<path.mp4>`, then
/// `cargo nextest run -p overmark-win --run-ignored only -E 'test(write_a_sample_clip)'`.
/// Three seconds of 640 x 360 at 15 fps through the engine as it records, with the hardware
/// encoder if there is one. The left half is (R 200, G 100, B 50).
#[test]
#[ignore = "measure: writes a clip for playback checks, to OVERMARK_SAMPLE_CLIP"]
fn write_a_sample_clip() {
    let path = PathBuf::from(std::env::var("OVERMARK_SAMPLE_CLIP").expect("OVERMARK_SAMPLE_CLIP"));
    let area = PixelRect::new(0, 0, 640, 360);
    let start = Start::new(area, RecordSettings::default(), &path);
    let make: MakeSource = Box::new(|| Ok(Box::new(Pattern::new(640, 360)) as Box<dyn Source>));
    let rec = Recording::start_with(start, area, make, Box::new(|| {})).unwrap();
    record_for(&rec, Duration::from_secs(3));
    let saved = rec.stop().unwrap();
    println!("{saved:?}");
}

/// A width that isn't a multiple of 16 (the decoder pads its rows): the colours still come
/// back where they were.
#[test]
fn an_unaligned_width_decodes_back_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let rec = start_pattern(
        dir.path(),
        680,
        162,
        RecordSettings::default(),
        None,
        Box::new(|| {}),
    )
    .unwrap();
    record_for(&rec, Duration::from_millis(300));
    let saved = rec.stop().unwrap();
    let (w, h, bgra) = mf::decode_frame_at(&saved.path, 0).unwrap();
    assert_eq!((w, h), (680, 162));
    let at = |x: usize, y: usize| {
        let i = (y * w as usize + x) * 4;
        [bgra[i], bgra[i + 1], bgra[i + 2]]
    };
    // The left half is the pattern's colour on every row, the far right black.
    for y in [0, 40, 100, 161] {
        let p = at(100, y);
        assert!(
            (0..3).all(|c| p[c].abs_diff(LEFT[c]) <= 8),
            "row {y}: {p:?}"
        );
        let q = at(679, y);
        assert!(q.iter().all(|&v| v <= 24), "row {y}, right edge: {q:?}");
    }
}
