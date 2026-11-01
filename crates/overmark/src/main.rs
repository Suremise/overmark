//! Overmark's exe: wires the core and the Windows layer together (the app is in `lib.rs`).

// No console window in release builds; debug builds keep one for their output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    // `run` sets up the logs: `logs\overmark.log` beside the settings, and stderr (a console in
    // debug builds, or whoever started it).
    let args = overmark::Args::parse(std::env::args().skip(1));
    std::process::exit(overmark::run(args));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("{}", overmark_core::i18n::tr("app-windows-only"));
    std::process::exit(1);
}
