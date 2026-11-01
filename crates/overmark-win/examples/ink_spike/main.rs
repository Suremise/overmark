//! The ink-layer spike: measures the two ways to show ink, checks what screen
//! capture sees, and lets a person draw over the screen.
//!
//! ```text
//! cargo run -p overmark-win --release --example ink_spike -- <mode>
//!
//!   draw [layered|dcomp] [--smoke]
//!                         draw over every monitor with the mouse or a pen (default: dcomp).
//!                         Keys: Esc quits, C clears, F freezes or unfreezes, D dims,
//!                         H switches pen and highlighter, S saves each monitor (the screen and
//!                         the ink, without the dim or toolbar) to %TEMP%\overmark-spike-save-N.bmp.
//!                         --smoke draws two strokes itself, presses S W F S W C W F Esc
//!                         and checks every save; --bench [seconds] times this mode's own
//!                         drawing path (draw, recompose, present) in Live and Frozen
//!   bench [seconds]       frame cost of both paths at the monitor's size and at 3840x2160,
//!                         and the time to the screen (default: 4 s per run)
//!   capture               what Windows Graphics Capture and Desktop Duplication see: the ink
//!                         layer, the excluded dim and toolbar, the capture border, clean grabs
//!   dpi                   one layer per monitor: its size in physical pixels and its DPI
//! ```
//!
//! The bench and the capture checks never take the mouse or keyboard: their windows are
//! click-through and never activate, and they draw nothing but faint strokes and small squares
//! near the top-left corner for a few seconds.

mod bench;
mod capture;
mod draw;
mod ink;

use overmark_win::win32::dpi;

fn main() {
    if let Err(e) = dpi::enable_per_monitor_v2() {
        eprintln!("per-monitor DPI awareness v2 wasn't set: {e}");
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("draw");
    let rest = &args[args.len().min(1)..];
    let result = match mode {
        "draw" if rest.iter().any(|a| a == "--bench") => draw::bench(
            rest.iter()
                .find_map(|a| a.parse::<f64>().ok())
                .unwrap_or(2.0),
        ),
        "draw" => draw::run(
            rest.iter()
                .map(String::as_str)
                .find(|a| !a.starts_with("--"))
                .unwrap_or("dcomp"),
            rest.iter().any(|a| a == "--smoke"),
        ),
        "bench" => bench::run(rest.first().and_then(|s| s.parse().ok()).unwrap_or(4.0)),
        "capture" => capture::run(),
        "dpi" => dpi_report(),
        other => {
            eprintln!("unknown mode {other:?}: draw, bench, capture or dpi");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("ink_spike {mode}: {e}");
        std::process::exit(1);
    }
}

/// Q5: one layer per monitor, in physical pixels.
fn dpi_report() -> windows::core::Result<()> {
    use overmark_win::win32::window::{Options, Window};

    let monitors = dpi::monitors();
    println!(
        "{} monitor(s); the process is per-monitor DPI aware v2",
        monitors.len()
    );
    let mut all_ok = true;
    for m in &monitors {
        let w = Window::new(
            m.rect,
            Options {
                layered: true,
                click_through: true,
                no_activate: true,
                ..Default::default()
            },
            Box::new(|_, _, _, _| None),
        )?;
        let (cw, ch) = w.client_size();
        let window_dpi = w.dpi();
        let ok = cw == m.rect.width() && ch == m.rect.height();
        all_ok &= ok;
        println!(
            "{}{}: {}x{} px at ({}, {}), {} dpi ({:.0} %); its layer's client area {}x{} px, window dpi {} {}",
            m.name,
            if m.primary { " (primary)" } else { "" },
            m.rect.width(),
            m.rect.height(),
            m.rect.left,
            m.rect.top,
            m.dpi,
            m.scale() * 100.0,
            cw,
            ch,
            window_dpi,
            if ok { "ok" } else { "MISMATCH" }
        );
    }
    println!("{}", if all_ok { "dpi: ok" } else { "dpi: FAILED" });
    Ok(())
}
