//! Q1: each path's frame cost while drawing continuously, at the monitor's size and at
//! 3840x2160, and the time from a present to the screen.
//!
//! Each frame waits for DWM's next composition (DwmFlush), as an app drawing at the refresh
//! rate does, then draws a fast scribble's next stretch (six segments, as a 1 kHz mouse or a
//! pen gives between frames) and presents only the rectangle it touched. Once a second it
//! clears the whole layer and presents all of it: the cost of an undo or a clear, the worst
//! frame. The strokes are faint (alpha 10) and the windows click-through, so the bench can
//! run while the screen is in use.

use overmark_win::win32::Rect;
use overmark_win::win32::capture::close;
use overmark_win::win32::capture::wgc::Observer;
use overmark_win::win32::clock;
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::window::{self, Options};

use crate::ink::{InkLayer, Path, bgra, fill, now, percentiles, segment};

/// 144 Hz's frame, and the spike's bar: a 4K frame in half of it.
const FRAME_144HZ_MS: f64 = 1000.0 / 144.0;
const BUDGET_MS: f64 = FRAME_144HZ_MS / 2.0;

struct Run {
    path: Path,
    size: (i32, i32),
    frames: usize,
    render: Vec<f64>,
    upload: Vec<f64>,
    present: Vec<f64>,
    total: Vec<f64>,
    full: Vec<f64>,
    over_period: usize,
}

fn draw_run(path: Path, rect: Rect, seconds: f64, period_ms: f64) -> windows::core::Result<Run> {
    let mut layer = InkLayer::new(
        path,
        rect,
        Options {
            click_through: true,
            no_activate: true,
            ..Default::default()
        },
        Box::new(|_, _, _, _| None),
    )?;
    layer.window().show(false);
    let (w, h) = (rect.width() as f32, rect.height() as f32);
    let color = bgra(255, 40, 40, 10);
    // A Lissajous scribble across the layer at about 4,000 px/s: a fast flick of the pen.
    let at = |t: f64| -> (f32, f32) {
        let t = t as f32;
        (
            w * 0.5 + w * 0.42 * (t * 1.9).sin(),
            h * 0.5 + h * 0.42 * (t * 2.7 + 0.6).sin(),
        )
    };
    let mut run = Run {
        path,
        size: (rect.width(), rect.height()),
        frames: 0,
        render: Vec::new(),
        upload: Vec::new(),
        present: Vec::new(),
        total: Vec::new(),
        full: Vec::new(),
        over_period: 0,
    };
    let start = now();
    let mut last_t = 0.0;
    let mut last_clear = start;
    while now() - start < seconds {
        window::pump();
        window::wait_for_composition();
        let frame_start = now();
        let t = frame_start - start;
        if frame_start - last_clear >= 1.0 {
            // The worst frame: clear everything and present all of it.
            layer.pixels().fill(0);
            let p = layer.present(None, true)?;
            run.full.push(now() - frame_start);
            let _ = p;
            last_clear = frame_start;
            last_t = t;
            continue;
        }
        let mut dirty = Rect::default();
        {
            let mut pm = layer.pixmap();
            for i in 1..=6 {
                let a = last_t + (t - last_t) * f64::from(i - 1) / 6.0;
                let b = last_t + (t - last_t) * f64::from(i) / 6.0;
                dirty = dirty.union(&segment(&mut pm, at(a), at(b), 6.0, color));
            }
        }
        let rendered = now();
        let p = layer.present(Some(dirty), true)?;
        let done = now();
        run.render.push(rendered - frame_start);
        run.upload.push(p.upload);
        run.present.push(p.present);
        let total = done - frame_start;
        run.total.push(total);
        if total * 1000.0 > period_ms {
            run.over_period += 1;
        }
        run.frames += 1;
        last_t = t;
    }
    window::pump();
    Ok(run)
}

/// The time from a present to the frame a capture sees it in: a small square switched on and
/// off, watched by Windows Graphics Capture (as a share sees the screen).
fn latency_run(path: Path, monitor: &Monitor, toggles: usize) -> windows::core::Result<Vec<f64>> {
    let mut layer = InkLayer::new(
        path,
        monitor.rect,
        Options {
            click_through: true,
            no_activate: true,
            ..Default::default()
        },
        Box::new(|_, _, _, _| None),
    )?;
    layer.window().show(false);
    let square = Rect::new(40, 40, 48, 48);
    let observer = Observer::start(monitor, vec![(44, 44)], false, false)?;
    let magenta = [255, 0, 255, 255];
    let mut out = Vec::new();
    for i in 0..toggles {
        let on = i % 2 == 0;
        let width = layer.width();
        fill(
            layer.pixels(),
            width,
            square,
            if on { magenta } else { [0; 4] },
        );
        observer.take_frames();
        let t0 = clock::now_100ns();
        layer.present(Some(square), true)?;
        let deadline = now() + 0.5;
        let mut seen = None;
        while seen.is_none() && now() < deadline {
            window::pump();
            for f in observer.take_frames() {
                if f.time_100ns >= t0 && close(f.probes[0], magenta, 40) == on {
                    seen = Some(f.time_100ns);
                    break;
                }
            }
            window::wait_for_message(1);
        }
        if let Some(t) = seen {
            out.push((t - t0) as f64 / 10_000_000.0);
        }
        // A pause of a varying length, so presents land at every point of the refresh cycle.
        let pause = 0.02 + 0.013 * (i % 7) as f64 / 7.0;
        let until = now() + pause;
        while now() < until {
            window::pump();
            window::wait_for_message(1);
        }
    }
    Ok(out)
}

/// The same measurement for an ordinary window painted with GDI, the baseline: what any app
/// drawing on screen gets.
fn baseline_run(monitor: &Monitor, toggles: usize) -> windows::core::Result<Vec<f64>> {
    let r = monitor.rect;
    let square = Rect::new(r.left + 40, r.top + 40, r.left + 48, r.top + 48);
    let window = overmark_win::win32::window::Window::new(
        square,
        Options {
            no_activate: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| {
            // Leave the background alone: the test paints it.
            (msg == windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND)
                .then_some(windows::Win32::Foundation::LRESULT(1))
        }),
    )?;
    window.fill_gdi(0, 0, 0);
    window.show(false);
    let observer = Observer::start(monitor, vec![(44, 44)], false, false)?;
    let magenta = [255, 0, 255, 255];
    let mut out = Vec::new();
    for i in 0..toggles {
        let on = i % 2 == 0;
        observer.take_frames();
        let t0 = clock::now_100ns();
        if on {
            window.fill_gdi(255, 0, 255);
        } else {
            window.fill_gdi(0, 0, 0);
        }
        let deadline = now() + 0.5;
        let mut seen = None;
        while seen.is_none() && now() < deadline {
            window::pump();
            for f in observer.take_frames() {
                if f.time_100ns >= t0 && close(f.probes[0], magenta, 40) == on {
                    seen = Some(f.time_100ns);
                    break;
                }
            }
            window::wait_for_message(1);
        }
        if let Some(t) = seen {
            out.push((t - t0) as f64 / 10_000_000.0);
        }
        let pause = 0.02 + 0.013 * (i % 7) as f64 / 7.0;
        let until = now() + pause;
        while now() < until {
            window::pump();
            window::wait_for_message(1);
        }
    }
    Ok(out)
}

fn row(label: &str, s: &[f64]) {
    let [p50, p95, p99, max] = percentiles(s);
    println!("    {label:<26} p50 {p50:6.3}  p95 {p95:6.3}  p99 {p99:6.3}  max {max:7.3} ms");
}

pub fn run(seconds: f64) -> windows::core::Result<()> {
    let monitors = dpi::monitors();
    let m = monitors.first().expect("a monitor").clone();
    let hz = m.refresh_hz();
    let period_ms = 1000.0 / f64::from(hz);
    println!(
        "Monitor {}: {}x{} px, {} Hz ({:.2} ms a frame), {} dpi. Bar: a 3840x2160 frame in {:.2} ms (half of 144 Hz's {:.2} ms).",
        m.name,
        m.rect.width(),
        m.rect.height(),
        hz,
        period_ms,
        m.dpi,
        BUDGET_MS,
        FRAME_144HZ_MS
    );
    let native = m.rect;
    // A 3840x2160 layer from the monitor's top-left: it runs past the monitor's edges, but
    // the renderer, the copy to DWM and DWM's surface are all 4K-sized.
    let uhd = Rect::new(
        m.rect.left,
        m.rect.top,
        m.rect.left + 3840,
        m.rect.top + 2160,
    );
    let mut summary = Vec::new();
    for path in [Path::Layered, Path::DComp] {
        for rect in [native, uhd] {
            let r = draw_run(path, rect, seconds, period_ms)?;
            println!(
                "\n{} at {}x{}: {} frames in {:.1} s ({:.0} fps), {} over one refresh",
                r.path.name(),
                r.size.0,
                r.size.1,
                r.frames,
                seconds,
                r.frames as f64 / seconds,
                r.over_period
            );
            row("draw (tiny-skia)", &r.render);
            if path == Path::DComp {
                row("upload dirty rect", &r.upload);
            }
            row("present", &r.present);
            row("frame total", &r.total);
            row("full clear + present", &r.full);
            summary.push(r);
        }
    }
    println!("\nTime from present to the screen, as Windows Graphics Capture sees it:");
    for path in [None, Some(Path::Layered), Some(Path::DComp)] {
        let l = match path {
            None => baseline_run(&m, 40)?,
            Some(p) => latency_run(p, &m, 40)?,
        };
        let [p50, p95, _, max] = percentiles(&l);
        println!(
            "    {:<26} {} of 40 seen; p50 {p50:6.2}  p95 {p95:6.2}  max {max:6.2} ms ({:.1} / {:.1} frames at {} Hz)",
            path.map_or("baseline: a GDI window", Path::name),
            l.len(),
            p50 / period_ms,
            p95 / period_ms,
            hz
        );
    }
    println!("\nSummary against the bar ({BUDGET_MS:.2} ms for a 4K frame):");
    for r in summary.iter().filter(|r| r.size == (3840, 2160)) {
        let typical = percentiles(&r.total);
        let worst_full = percentiles(&r.full);
        let scaled = percentiles(
            &summary
                .iter()
                .find(|n| n.path == r.path && n.size != (3840, 2160))
                .map(|n| n.total.clone())
                .unwrap_or_default(),
        );
        let area = (3840.0 * 2160.0) / f64::from(native.width() * native.height());
        println!(
            "    {:<20} drawing p99 {:.3} ms measured at 4K ({:.3} ms at {}x{}, x{:.2} scaled {:.3} ms); full clear p50 {:.2} ms; {}",
            r.path.name(),
            typical[2],
            scaled[2],
            native.width(),
            native.height(),
            area,
            scaled[2] * area,
            worst_full[0],
            if typical[2] <= BUDGET_MS {
                "PASS"
            } else {
                "FAIL"
            }
        );
    }
    Ok(())
}
