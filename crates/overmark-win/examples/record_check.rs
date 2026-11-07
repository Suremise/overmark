//! Records the real screen through the engine, as the app will, and reads each MP4 back:
//!
//! ```powershell
//! cargo run -p overmark-win --release --example record_check [-- --seconds 10] [--keep]
//! ```
//!
//! On the primary monitor: a small box (800 x 600, Task Manager's size), the left half and the
//! whole screen, each for `--seconds` at 15 fps Balanced with the pointer, then a run with a
//! pause in it, then where the pointer is drawn against `GetCursorPos`. It makes no window and
//! sends no input: it only reads the screen, into a temp folder, and deletes the files after
//! (`--keep` leaves them, and prints the folder). What's on the screen meanwhile is whatever
//! you're doing, so the sizes vary with it.

use std::time::{Duration, Instant};

use overmark_win::record::{PixelRect, RecordSettings, Recording, Saved, Start, Status};
use overmark_win::win32::{dpi, mf, shell};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn record(path: std::path::PathBuf, area: PixelRect, settings: RecordSettings, secs: f64) -> Saved {
    let rec = Recording::start(Start::new(area, settings, path), || {}).expect("start");
    while rec.elapsed().as_secs_f64() < secs && rec.status() == Status::Recording {
        std::thread::sleep(Duration::from_millis(20));
    }
    rec.stop().expect("stop")
}

fn report(name: &str, saved: &Saved) {
    let info = mf::probe(&saved.path).expect("probe");
    let secs = saved.duration.as_secs_f64();
    println!(
        "{name}: {}x{} {} frames, {:.2} s (file {:.2} s), {:.0} KB, {:.2} MB/min, {}{}{}",
        info.width,
        info.height,
        info.frames,
        secs,
        info.duration_100ns as f64 / 1e7,
        saved.bytes as f64 / 1000.0,
        saved.bytes as f64 / 1e6 / secs * 60.0,
        if saved.encoder.hardware {
            "hardware "
        } else {
            "software"
        },
        saved.encoder.name,
        saved
            .ended_by
            .as_ref()
            .map(|e| format!(", ended by: {e:?}"))
            .unwrap_or_default(),
    );
    assert!(info.h264);
    assert_eq!(info.frames, saved.frames);
}

fn main() {
    dpi::enable_per_monitor_v2().expect("per-monitor DPI awareness");
    let secs: f64 = arg("--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);
    let keep = std::env::args().any(|a| a == "--keep");
    let dir = tempfile::Builder::new()
        .prefix("overmark-record-check-")
        .tempdir()
        .expect("temp folder");
    let monitor = dpi::monitors().into_iter().next().expect("a monitor");
    let m = monitor.rect;
    println!(
        "Primary monitor {}x{} at ({}, {}), {} Hz",
        m.width(),
        m.height(),
        m.left,
        m.top,
        monitor.refresh_hz()
    );
    let settings = RecordSettings::default();
    let small = PixelRect::new(m.left + 100, m.top + 100, m.left + 900, m.top + 700);
    let half = PixelRect::new(m.left, m.top, m.left + m.width() / 2, m.bottom);
    let full = PixelRect::new(m.left, m.top, m.right, m.bottom);
    for (name, area) in [("small", small), ("half", half), ("full", full)] {
        let saved = record(dir.path().join(format!("{name}.mp4")), area, settings, secs);
        report(name, &saved);
    }

    // A pause: the file is as long as the time recorded, not the time taken.
    let wall = Instant::now();
    let rec = Recording::start(
        Start::new(small, settings, dir.path().join("pause.mp4")),
        || {},
    )
    .expect("start");
    while rec.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    rec.pause();
    std::thread::sleep(Duration::from_secs(2));
    rec.resume();
    while rec.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = rec.stop().expect("stop");
    report("pause", &saved);
    println!(
        "  {:.2} s recorded in {:.2} s with a 2 s pause",
        saved.duration.as_secs_f64(),
        wall.elapsed().as_secs_f64()
    );

    // The pointer: the difference between a frame with it and one without, around the cursor.
    let (cx, cy) = shell::cursor();
    let around = PixelRect::new(cx - 64, cy - 64, cx + 64, cy + 64);
    let with = record(dir.path().join("pointer.mp4"), around, settings, 1.0);
    let without = record(
        dir.path().join("no-pointer.mp4"),
        around,
        RecordSettings {
            pointer: false,
            ..settings
        },
        1.0,
    );
    let (now_x, now_y) = shell::cursor();
    let (w, h, a) = mf::decode_frame_at(&with.path, 0).expect("decode");
    let (_, _, b) = mf::decode_frame_at(&without.path, 0).expect("decode");
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let differs = (0..3).any(|c| a[i + c].abs_diff(b[i + c]) > 40);
            if differs {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((l, t, r, btm)) => (l.min(x), t.min(y), r.max(x), btm.max(y)),
                });
            }
        }
    }
    let area = with.area;
    match bbox {
        Some((l, t, r, b)) => println!(
            "pointer: drawn at ({}, {})-({}, {}) on the screen; GetCursorPos ({cx}, {cy}) before, ({now_x}, {now_y}) after. An arrow's hot spot is its top-left.",
            area.left + l as i32,
            area.top + t as i32,
            area.left + r as i32,
            area.top + b as i32
        ),
        None => println!(
            "pointer: no difference with and without it (hidden, or off this monitor; GetCursorPos ({cx}, {cy}))"
        ),
    }

    if keep {
        println!("Kept in {}", dir.keep().display());
    }
}
