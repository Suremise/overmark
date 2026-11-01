//! Q2-Q4, and the input half of Q1: what capture sees, and where clicks go.
//!
//! Small test windows near the monitor's top-left: the ink layer (a magenta square, visible to
//! capture), a dim veil (black at 63 %, excluded) and a toolbar (cyan, excluded). Each check
//! reads probe pixels from Windows Graphics Capture (as a Teams or Zoom share sees the screen)
//! and from Desktop Duplication, and compares them with the screen before the windows came.

use overmark_win::win32::Rect;
use overmark_win::win32::affinity;
use overmark_win::win32::capture::dd::Duplication;
use overmark_win::win32::capture::wgc::Observer;
use overmark_win::win32::capture::{Frame, close};
use overmark_win::win32::clock;
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::magnifier::Magnifier;
use overmark_win::win32::window::{self, Options, Window};

use crate::ink::{InkLayer, Path, fill, now};

const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const CYAN: [u8; 4] = [255, 255, 0, 255];
/// How far a probe may drift from the screen before (anti-aliasing, a changing desktop).
const TOLERANCE: u8 = 24;

/// Pumps messages for `seconds`.
fn settle(seconds: f64) {
    let until = now() + seconds;
    while now() < until {
        window::pump();
        window::wait_for_message(2);
    }
}

fn center(r: Rect) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

fn on_monitor(m: &Monitor, r: Rect) -> Rect {
    Rect::new(
        m.rect.left + r.left,
        m.rect.top + r.top,
        m.rect.left + r.right,
        m.rect.top + r.bottom,
    )
}

fn verdict(ok: bool) -> &'static str {
    if ok { "ok" } else { "FAILED" }
}

/// Counts the capture border's yellow along the frame's edges (4 px deep).
fn yellow_on_edges(f: &Frame) -> usize {
    let is_yellow = |p: [u8; 4]| p[2] > 200 && p[1] > 120 && p[1] < 235 && p[0] < 90;
    let mut n = 0;
    for y in 0..f.height {
        for x in 0..f.width {
            let edge = x < 4 || y < 4 || x >= f.width - 4 || y >= f.height - 4;
            if edge && is_yellow(f.pixel(x, y)) {
                n += 1;
            }
        }
    }
    n
}

pub fn run() -> windows::core::Result<()> {
    let m = dpi::monitors().first().expect("a monitor").clone();
    println!(
        "Monitor {}: {}x{} px. Windows {}.",
        m.name,
        m.rect.width(),
        m.rect.height(),
        windows_version()
    );
    let mut dd = Duplication::new(&m)?;
    let ink_sq = Rect::new(100, 100, 164, 164);
    let dim_sq = Rect::new(200, 100, 264, 164);
    let bar_sq = Rect::new(300, 100, 364, 164);
    let probes = vec![center(ink_sq), center(dim_sq), center(bar_sq)];

    // The ink layer: the whole monitor, transparent for now.
    let mut ink = InkLayer::new(
        Path::DComp,
        m.rect,
        Options {
            click_through: true,
            no_activate: true,
            ..Default::default()
        },
        Box::new(|_, _, _, _| None),
    )?;
    let w = ink.width();
    ink.window().show(false);
    settle(0.2);
    // The screen before our squares: a tiny change of our own away from the probes gives
    // Desktop Duplication a fresh frame (its first one can be empty).
    let marker = Rect::new(100, 190, 102, 192);
    let t_base = clock::now_100ns();
    fill(ink.pixels(), w, marker, MAGENTA);
    ink.present(Some(marker), true)?;
    let base = dd.grab_settled(t_base, 25, 1000)?.frame;
    let base_px: Vec<[u8; 4]> = probes.iter().map(|&(x, y)| base.pixel(x, y)).collect();
    println!(
        "    the screen at the probes before: {:?} (Desktop Duplication)",
        base_px
    );
    fill(ink.pixels(), w, ink_sq, MAGENTA);
    ink.present(Some(ink_sq), true)?;
    let quiet = Options {
        click_through: true,
        no_activate: true,
        ..Default::default()
    };
    let no_erase: fn() -> overmark_win::win32::window::Handler = || {
        Box::new(|_, msg, _, _| {
            (msg == windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND)
                .then_some(windows::Win32::Foundation::LRESULT(1))
        })
    };
    let dim = Window::new(on_monitor(&m, dim_sq), quiet, no_erase())?;
    dim.fill_gdi(0, 0, 0);
    dim.set_opacity(160);
    let bar = Window::new(on_monitor(&m, bar_sq), quiet, no_erase())?;
    bar.fill_gdi(0, 255, 255);
    affinity::set_excluded(dim.hwnd(), true)?;
    affinity::set_excluded(bar.hwnd(), true)?;
    dim.show(false);
    bar.show(false);
    dim.fill_gdi(0, 0, 0);
    bar.fill_gdi(0, 255, 255);
    settle(0.3);

    // Q2: visibility to capture.
    println!("\nQ2. What capture sees (ink layer visible; dim veil and toolbar excluded):");
    println!(
        "    dim veil excluded: {}, toolbar excluded: {}, ink layer excluded: {}",
        affinity::is_excluded(dim.hwnd()),
        affinity::is_excluded(bar.hwnd()),
        affinity::is_excluded(ink.hwnd())
    );
    let observer = Observer::start(&m, probes.clone(), true, false)?;
    // A change of our own, so both captures have a fresh frame to give.
    let t_setup = clock::now_100ns();
    fill(ink.pixels(), w, Rect::new(100, 170, 104, 174), MAGENTA);
    ink.present(Some(Rect::new(100, 170, 104, 174)), true)?;
    let mut wgc_frame = None;
    let deadline = now() + 2.0;
    while wgc_frame.is_none() && now() < deadline {
        settle(0.02);
        if let Some(f) = observer
            .take_frames()
            .into_iter()
            .rev()
            .find(|f| f.time_100ns >= t_setup)
        {
            wgc_frame = Some(f.probes);
        }
    }
    let dd_frame = dd.grab_settled(t_setup, 25, 1000)?.frame;
    let dd_px: Vec<[u8; 4]> = probes.iter().map(|&(x, y)| dd_frame.pixel(x, y)).collect();
    let mut q2_ok = true;
    for (name, px) in [
        ("Windows Graphics Capture", wgc_frame.unwrap_or_default()),
        ("Desktop Duplication", dd_px),
    ] {
        if px.len() < 3 {
            println!("    {name}: no frame");
            q2_ok = false;
            continue;
        }
        let ink_seen = close(px[0], MAGENTA, TOLERANCE);
        let dim_hidden = close(px[1], base_px[1], TOLERANCE);
        let bar_hidden = !close(px[2], CYAN, TOLERANCE) && close(px[2], base_px[2], TOLERANCE);
        q2_ok &= ink_seen && dim_hidden && bar_hidden;
        println!(
            "    {name:<25} ink {:?} (shown: {}), under the dim {:?} vs {:?} before (dim hidden: {}), under the toolbar {:?} (toolbar hidden: {})",
            px[0],
            verdict(ink_seen),
            px[1],
            base_px[1],
            verdict(dim_hidden),
            px[2],
            verdict(bar_hidden)
        );
    }
    println!("    Q2: {}", verdict(q2_ok));
    drop(observer);

    // Q4: a clean grab for a save, the ink layer excluded only while grabbing, watched by a
    // share's capture to see whether the ink flickers for its audience.
    println!(
        "\nQ4. Saving without our windows: exclude the ink layer only while Desktop Duplication grabs:"
    );
    let watcher = Observer::start(&m, vec![center(ink_sq)], false, false)?;
    settle(0.3);
    let mut clean = 0;
    let mut gaps = Vec::new();
    let mut grab_ms = Vec::new();
    let tries = 10;
    for _ in 0..tries {
        watcher.take_frames();
        let t0 = clock::now_100ns();
        affinity::set_excluded(ink.hwnd(), true)?;
        let grab = dd.grab_after(t0, 500);
        affinity::set_excluded(ink.hwnd(), false)?;
        let t1 = clock::now_100ns();
        grab_ms.push((t1 - t0) as f64 / 10_000.0);
        if let Ok(g) = grab {
            let p = g.frame.pixel(center(ink_sq).0, center(ink_sq).1);
            let others_ok = close(
                g.frame.pixel(center(dim_sq).0, center(dim_sq).1),
                base_px[1],
                TOLERANCE,
            ) && close(
                g.frame.pixel(center(bar_sq).0, center(bar_sq).1),
                base_px[2],
                TOLERANCE,
            );
            if close(p, base_px[0], TOLERANCE) && !close(p, MAGENTA, TOLERANCE) && others_ok {
                clean += 1;
            }
        }
        settle(0.25);
        // Frames the share saw without the ink, and for how long.
        let frames = watcher.take_frames();
        let missing: Vec<i64> = frames
            .iter()
            .filter(|f| f.time_100ns >= t0 && !close(f.probes[0], MAGENTA, TOLERANCE))
            .map(|f| f.time_100ns)
            .collect();
        let back = frames
            .iter()
            .find(|f| {
                missing.first().is_some_and(|&s| f.time_100ns > s)
                    && close(f.probes[0], MAGENTA, TOLERANCE)
            })
            .map(|f| f.time_100ns);
        let span = match (missing.first(), back) {
            (Some(&s), Some(e)) => (e - s) as f64 / 10_000.0,
            _ => 0.0,
        };
        gaps.push((missing.len(), span));
    }
    let flickers = gaps.iter().filter(|g| g.0 > 0).count();
    let worst = gaps.iter().map(|g| g.1).fold(0.0, f64::max);
    println!(
        "    clean grabs (no ink, dim or toolbar in them): {clean} of {tries}; a grab took {:.1}-{:.1} ms",
        grab_ms.iter().copied().fold(f64::MAX, f64::min),
        grab_ms.iter().copied().fold(0.0, f64::max)
    );
    println!(
        "    the share saw the ink vanish in {flickers} of {tries} saves (frames without it: {:?}); longest gap {worst:.1} ms",
        gaps.iter().map(|g| g.0).collect::<Vec<_>>()
    );
    drop(watcher);

    // Q4, the clean way: the Magnification API renders the monitor without the windows we name,
    // while they stay on screen. The ink is opaque here, over a backdrop of a known colour: the
    // grab must show the backdrop under it, the share must never lose the ink, and outside our
    // windows the grab must match Desktop Duplication's at the same moment.
    println!(
        "\nQ4c. Saving without our windows, none of them leaving the screen (the Magnification API):"
    );
    let patch = Rect::new(100, 200, 420, 260);
    let backdrop = Window::new(on_monitor(&m, patch), quiet, no_erase())?;
    backdrop.fill_gdi(200, 100, 50);
    backdrop.show(false);
    backdrop.place_below(ink.hwnd());
    backdrop.fill_gdi(200, 100, 50);
    let opaque = Rect::new(120, 210, 200, 250);
    fill(ink.pixels(), w, opaque, MAGENTA);
    ink.present(Some(opaque), true)?;
    settle(0.3);
    let magnifier = Magnifier::new(m.rect, &[ink.hwnd(), dim.hwnd(), bar.hwnd()])?;
    let watcher = Observer::start(&m, vec![center(opaque)], false, false)?;
    settle(0.2);
    let backdrop_bgra = [50u8, 100, 200, 255];
    let mut clean = 0;
    let mut grab_ms = Vec::new();
    let tries = 10;
    watcher.take_frames();
    let t0 = clock::now_100ns();
    let mut last_clean = None;
    for _ in 0..tries {
        let t = now();
        let g = magnifier.grab(500);
        grab_ms.push((now() - t) * 1000.0);
        if let Some(g) = g {
            let under_ink = g.pixel(center(opaque).0, center(opaque).1);
            let under_dim = g.pixel(center(dim_sq).0, center(dim_sq).1);
            let under_bar = g.pixel(center(bar_sq).0, center(bar_sq).1);
            if close(under_ink, backdrop_bgra, 2)
                && close(under_dim, base_px[1], TOLERANCE)
                && close(under_bar, base_px[2], TOLERANCE)
            {
                clean += 1;
            }
            last_clean = Some(g);
        }
        settle(0.05);
    }
    settle(0.2);
    let vanished = watcher
        .take_frames()
        .iter()
        .filter(|f| f.time_100ns >= t0 && !close(f.probes[0], MAGENTA, TOLERANCE))
        .count();
    println!(
        "    clean grabs (the backdrop under opaque ink, no dim, no toolbar): {clean} of {tries}; a grab took {:.1}-{:.1} ms",
        grab_ms.iter().copied().fold(f64::MAX, f64::min),
        grab_ms.iter().copied().fold(0.0, f64::max)
    );
    println!("    frames the share saw without the ink: {vanished}");
    drop(watcher);
    // Fidelity: against Desktop Duplication's frame of the same moment, outside our windows.
    let t = clock::now_100ns();
    let nudge = Rect::new(100, 196, 102, 198);
    fill(ink.pixels(), w, nudge, [0; 4]);
    ink.present(Some(nudge), true)?;
    let d = dd.grab_settled(t, 25, 1000)?.frame;
    if let Some(g) = magnifier.grab(500).or(last_clean) {
        let ours = [
            opaque,
            ink_sq,
            dim_sq,
            bar_sq,
            nudge,
            Rect::new(100, 170, 104, 184),
        ];
        let (mut differ, mut total, mut bbox) = (0usize, 0usize, Rect::default());
        for y in 0..d.height.min(g.height) {
            for x in 0..d.width.min(g.width) {
                if ours.iter().any(|r| {
                    x >= r.left - 2 && x < r.right + 2 && y >= r.top - 2 && y < r.bottom + 2
                }) {
                    continue;
                }
                total += 1;
                let (a, b) = (d.pixel(x, y), g.pixel(x, y));
                if (0..3).any(|n| a[n].abs_diff(b[n]) > 2) {
                    differ += 1;
                    bbox = bbox.union(&Rect::new(x, y, x + 1, y + 1));
                }
            }
        }
        println!(
            "    against Desktop Duplication, outside our windows: {differ} of {total} px differ by more than 2 (all within {bbox:?}: what changed between the two grabs)"
        );
    }
    drop(magnifier);
    drop(backdrop);
    fill(ink.pixels(), w, opaque, [0; 4]);
    ink.present(Some(opaque), true)?;

    // Q6: a highlighter that darkens what's behind it, against the live screen.
    println!("\nQ6. The highlighter: DirectComposition's MIN_BLEND against the screen behind it:");
    // A known background of our own (an ordinary window, visible to capture), and over it a
    // MIN_BLEND layer in thirds: highlighter yellow, white, and transparent.
    let hl_area = Rect::new(600, 100, 792, 164);
    let backdrop = Window::new(on_monitor(&m, hl_area), quiet, no_erase())?;
    backdrop.fill_gdi(200, 100, 50);
    backdrop.show(false);
    backdrop.fill_gdi(200, 100, 50);
    settle(0.2);
    let mut hl = InkLayer::new(
        Path::DComp,
        on_monitor(&m, hl_area),
        quiet,
        Box::new(|_, _, _, _| None),
    )?;
    let hw = hl.width();
    fill(hl.pixels(), hw, Rect::new(0, 0, 64, 64), [0, 255, 255, 255]);
    fill(
        hl.pixels(),
        hw,
        Rect::new(64, 0, 128, 64),
        [255, 255, 255, 255],
    );
    hl.present(None, true)?;
    let mode = hl.set_min_blend(true);
    hl.window().show(false);
    backdrop.place_below(hl.hwnd());
    settle(0.3);
    let t = clock::now_100ns();
    let nudge = Rect::new(100, 196, 102, 198);
    fill(ink.pixels(), w, nudge, MAGENTA);
    ink.present(Some(nudge), true)?;
    let g = dd.grab_settled(t, 25, 1000)?.frame;
    let at = |k: i32| g.pixel(hl_area.left + 32 + 64 * k, hl_area.top + 32);
    // The backdrop is RGB (200, 100, 50): BGRA [50, 100, 200].
    let backdrop_px = [50, 100, 200, 255];
    let darken = [0, 100, 200, 255];
    let over = [0, 255, 255, 255];
    println!(
        "    composite mode set: {}",
        if mode.is_ok() {
            "ok".to_string()
        } else {
            format!("{mode:?}")
        }
    );
    println!(
        "    yellow over the backdrop: {:?} (darken gives {:?}, plain alpha-over {:?}): {}",
        at(0),
        darken,
        over,
        verdict(close(at(0), darken, 8))
    );
    println!(
        "    white over the backdrop:  {:?} (darken leaves the backdrop {:?}): {}",
        at(1),
        backdrop_px,
        verdict(close(at(1), backdrop_px, 8))
    );
    println!(
        "    transparent pixels:       {:?} (the backdrop is {:?}; black would mean the layer must be white where it has no ink)",
        at(2),
        backdrop_px
    );
    drop(hl);
    drop(backdrop);
    window::pump();

    // Q3: the capture border. Desktop Duplication shows what's on screen, border included
    // (it's drawn by DWM), so it can look for the border a capture session asks for.
    println!(
        "\nQ3. Windows Graphics Capture's yellow border, as Desktop Duplication sees the screen:"
    );
    let mut flip_on = false;
    for border in [None, Some(false), Some(true), Some(false)] {
        let session = border
            .map(|b| Observer::start(&m, Vec::new(), false, b))
            .transpose()?;
        settle(0.6);
        flip_on = !flip_on;
        let t = clock::now_100ns();
        let flip = Rect::new(100, 180, 104, 184);
        fill(
            ink.pixels(),
            w,
            flip,
            if flip_on { MAGENTA } else { [0; 4] },
        );
        ink.present(Some(flip), true)?;
        let seen = dd
            .grab_settled(t, 25, 1000)
            .map(|g| yellow_on_edges(&g.frame));
        println!(
            "    {:<22} setting applied {:<5}; yellow edge pixels {}",
            match border {
                None => "no capture running:",
                Some(true) => "a capture, border on:",
                Some(false) => "a capture, border off:",
            },
            session.as_ref().is_none_or(|s| s.border_setting_applied),
            match seen {
                Ok(n) => n.to_string(),
                Err(e) => format!("(no frame: {e})"),
            }
        );
        drop(session);
        settle(0.2);
    }
    let session = Observer::start(&m, Vec::new(), false, false)?;
    settle(0.3);
    println!(
        "    a capture asking for no border: accepted {}",
        session.border_setting_applied
    );
    drop(session);

    // Q1, input: where a click on a transparent pixel goes.
    println!(
        "\nQ1 (input). Where a click on a transparent part of each layer goes (WindowFromPoint, no input sent):"
    );
    let area = Rect::new(500, 100, 700, 300);
    for path in [Path::Layered, Path::DComp] {
        let mut layer = InkLayer::new(
            path,
            on_monitor(&m, area),
            Options {
                no_activate: true,
                ..Default::default()
            },
            Box::new(|_, _, _, _| None),
        )?;
        let lw = layer.width();
        fill(layer.pixels(), lw, Rect::new(0, 0, 32, 32), MAGENTA);
        layer.present(None, true)?;
        layer.window().show(false);
        settle(0.15);
        let (ox, oy) = (m.rect.left + area.left, m.rect.top + area.top);
        let on_ink = window::window_at(ox + 16, oy + 16) == layer.hwnd();
        let on_clear = window::window_at(ox + 150, oy + 150) == layer.hwnd();
        println!(
            "    {:<20} on ink: {}; on a transparent pixel: {}",
            path.name(),
            if on_ink {
                "the layer"
            } else {
                "the window below"
            },
            if on_clear {
                "the layer"
            } else {
                "the window below (click-through)"
            }
        );
        drop(layer);
        window::pump();
    }
    drop(dim);
    drop(bar);
    drop(ink);
    window::pump();
    Ok(())
}

fn windows_version() -> String {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let read = |name: &str| {
        std::process::Command::new("reg")
            .args(["query", &format!(r"HKLM\{key}"), "/v", name])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.split_whitespace().last().map(str::to_string))
            .unwrap_or_default()
    };
    format!(
        "{} build {}.{}",
        read("DisplayVersion"),
        read("CurrentBuildNumber"),
        u32::from_str_radix(read("UBR").trim_start_matches("0x"), 16).unwrap_or(0)
    )
}
