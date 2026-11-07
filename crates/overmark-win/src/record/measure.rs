//! How big a minute of recording is, for docs/ARCHITECTURE.md (Recording, Sizes). Ignored in
//! the normal run; run it in release:
//!
//! ```powershell
//! cargo nextest run -p overmark-win --release --run-ignored only -E 'test(measure_sizes)' --no-capture
//! ```
//!
//! A minute of a mostly still screen like Task Manager's, made frame by frame (no screen, no
//! real-time wait): text-like detail that never changes, and once a second a CPU graph that
//! scrolls and numbers that change. Each box is encoded as the engine does it (the same
//! settings, NV12 and frame repeats).

use super::*;
use crate::win32::mf::RateControl;

/// A fixed pseudo-random number for a few coordinates.
fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

/// Whether text covers (`x`, `y`): lines 20 pixels apart, words of 6-pixel letters, each a
/// 5 x 8 glyph of strokes from a fixed hash.
fn text(x: usize, y: usize, salt: u32) -> bool {
    let (band, row) = (y / 20, y % 20);
    if !(6..14).contains(&row)
        || x % 48 >= 40
        || hash((x / 48) as u32, band as u32, salt).is_multiple_of(4)
    {
        return false;
    }
    let (cell, cx) = (x / 6, x % 6);
    let glyph = hash(cell as u32, band as u32, salt + 1);
    cx < 5
        && (cx == 0 || row == 6 || row == 13 || (glyph >> ((row - 6) * 4 + cx % 4)) & 1 == 1)
        && (glyph >> (cx * 3)) & 3 != 0
}

/// The scene at `second`, `w` x `h`: a desktop of text (scrolled up `scroll` pixels), and a
/// Task Manager window (up to 800 x 600) at the top-left whose graph and numbers change each
/// second.
fn scene(w: usize, h: usize, second: u32, scroll: usize) -> Vec<u8> {
    let mut px = vec![0u8; w * h * 4];
    let mut put = |x: usize, y: usize, rgb: [u8; 3]| {
        let i = (y * w + x) * 4;
        px[i..i + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
    };
    for y in 0..h {
        for x in 0..w {
            put(
                x,
                y,
                if text(x, y + scroll, 1) {
                    [40, 40, 40]
                } else {
                    [243, 243, 243]
                },
            );
        }
    }
    // The window: its graph (scrolling a column a second) and four numbers.
    let (tw, th) = (w.min(800), h.min(600));
    let (gx, gy, gw, gh) = (tw / 10, th / 4, tw * 6 / 10, th / 2);
    for y in gy..gy + gh {
        for x in gx..gx + gw {
            put(x, y, [255, 255, 255]);
            if (x - gx) % 40 == 0 || (y - gy) % 30 == 0 {
                put(x, y, [220, 230, 240]);
            }
        }
    }
    let columns = 60;
    let step = gw / columns;
    for c in 0..columns {
        // The value in column c at `second`: the samples scroll left.
        let load = 10 + hash(second + c as u32, 0, 3) % 30;
        let top = gy + gh - gh * load as usize / 100;
        for x in gx + c * step..gx + (c + 1) * step {
            for y in top..gy + gh {
                put(x, y, [200, 225, 245]);
            }
            put(x, top, [17, 125, 187]);
        }
    }
    for n in 0..4 {
        let (nx, ny) = (gx + n * 120, gy + gh + 20);
        for y in ny..(ny + 20).min(th) {
            for x in nx..(nx + 48).min(tw) {
                let lit = text(x - nx, y - ny + 20, 100 + second * 4 + n as u32);
                put(x, y, if lit { [30, 30, 30] } else { [243, 243, 243] });
            }
        }
    }
    px
}

/// How a minute is encoded.
#[derive(Debug, Clone, Copy)]
struct Run {
    w: usize,
    h: usize,
    hardware: bool,
    quality: Quality,
    rate_control: RateControl,
    gop_secs: u32,
    /// The screen never changes after the first frame.
    still: bool,
    /// The whole screen changes every frame (the text scrolls 3 pixels a frame, the graph
    /// moves), not once a second.
    busy: bool,
}

/// The bytes a minute takes, and the encoder that made it.
fn minute(dir: &Path, run: Run) -> (u64, String) {
    let Run { w, h, .. } = run;
    let settings = RecordSettings {
        quality: run.quality,
        ..RecordSettings::default()
    };
    let mut format = video_format(w as i32, h as i32, &settings);
    format.rate_control = run.rate_control;
    format.gop = run.gop_secs * settings.frame_rate.fps();
    let path = dir.join(format!("{run:?}.mp4").replace([' ', '{', '}', ':', ','], ""));
    let mut writer = Mp4Writer::create(&path, &format, run.hardware).unwrap();
    let encoder = if writer.encoder.hardware {
        writer.encoder.name.clone()
    } else {
        "software".to_owned()
    };
    let rate = settings.frame_rate;
    let fps = u64::from(rate.fps());
    let mut yuv = vec![0u8; nv12::len(w, h)];
    let mut buffer = None;
    for n in 0..60 * fps {
        let due = if run.busy { true } else { n % fps == 0 };
        if due && (buffer.is_none() || !run.still) {
            let (second, scroll) = if run.busy {
                (n as u32, n as usize * 3)
            } else {
                ((n / fps) as u32, 0)
            };
            nv12::from_bgra(&scene(w, h, second, scroll), w, h, &mut yuv);
            buffer = Some(writer.frame(&yuv).unwrap());
        }
        let b = buffer.as_ref().unwrap();
        writer
            .write(b, frame_time(n, rate), frame_duration(n, rate))
            .unwrap();
    }
    drop(buffer);
    writer.finish().unwrap();
    let info = mf::probe(&path).unwrap();
    assert_eq!(info.frames, 60 * fps);
    (std::fs::metadata(&path).unwrap().len(), encoder)
}

/// One line of a table: the output of a measurement for the docs, not text of the app.
fn out(line: &str) {
    println!("{}", line);
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

#[test]
#[ignore = "measure: a minute of video per cell, for the docs; run in release"]
fn measure_sizes() {
    let dir = tempfile::tempdir().unwrap();
    out("MB a minute at 15 fps (Small / Balanced / Sharp, capped VBR):");
    out("| Box | Encoder | Small | Balanced | Sharp | Balanced, still | Balanced, CBR |");
    out("|---|---|---|---|---|---|---|");
    for (w, h) in [(800, 600), (1280, 1440), (2560, 1440)] {
        for hardware in [false, true] {
            let base = Run {
                w,
                h,
                hardware,
                quality: Quality::Balanced,
                rate_control: RateControl::PeakConstrainedVbr,
                gop_secs: KEY_FRAME_SECS,
                still: false,
                busy: false,
            };
            let mut cells = Vec::new();
            let mut encoder = String::new();
            for run in [
                Run {
                    quality: Quality::Small,
                    ..base
                },
                base,
                Run {
                    quality: Quality::Sharp,
                    ..base
                },
                Run {
                    still: true,
                    ..base
                },
                Run {
                    rate_control: RateControl::Cbr,
                    ..base
                },
            ] {
                let (bytes, e) = minute(dir.path(), run);
                encoder = e;
                cells.push(format!("{:.2}", mb(bytes)));
            }
            out(&format!(
                "| {w} x {h} | {encoder} | {} |",
                cells.join(" | ")
            ));
        }
    }
    out("");
    out("Busy (all of it scrolling every frame), 1280 x 1440:");
    out("| Encoder | Small | Balanced | Sharp |");
    out("|---|---|---|---|");
    for hardware in [false, true] {
        let mut cells = Vec::new();
        let mut encoder = String::new();
        for quality in Quality::ALL {
            let run = Run {
                w: 1280,
                h: 1440,
                hardware,
                quality,
                rate_control: RateControl::PeakConstrainedVbr,
                gop_secs: KEY_FRAME_SECS,
                still: false,
                busy: true,
            };
            let (bytes, e) = minute(dir.path(), run);
            encoder = e;
            cells.push(format!("{:.2}", mb(bytes)));
        }
        out(&format!("| {encoder} | {} |", cells.join(" | ")));
    }
}

/// A still full screen with a key frame every 10, 30 and 60 s, capped VBR and quality-based
/// (docs/ARCHITECTURE.md, Recording: rate control):
/// `cargo nextest run -p overmark-win --release --run-ignored only -E 'test(measure_key_frames)' --no-capture`
#[test]
#[ignore = "measure: six minutes of video, for the docs; run in release"]
fn measure_key_frames() {
    let dir = tempfile::tempdir().unwrap();
    out("A still 2560 x 1440 minute, software encoder, MB:");
    out("| Key frame every | Capped VBR, Balanced | Quality 50 |");
    out("|---|---|---|");
    for gop_secs in [10, 30, 60] {
        let cells: Vec<String> = [RateControl::PeakConstrainedVbr, RateControl::Quality(50)]
            .into_iter()
            .map(|rate_control| {
                let run = Run {
                    w: 2560,
                    h: 1440,
                    hardware: false,
                    quality: Quality::Balanced,
                    rate_control,
                    gop_secs,
                    still: true,
                    busy: false,
                };
                format!("{:.2}", mb(minute(dir.path(), run).0))
            })
            .collect();
        out(&format!("| {gop_secs} s | {} |", cells.join(" | ")));
    }
}
