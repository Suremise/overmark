//! Overmark, the app: resident in the tray, it opens the overlay on a hotkey.
//! `main.rs` only calls [`run`]; the loop lives here so checks can build on it.
//!
//! The loop waits in `MsgWaitForMultipleObjectsEx` with no timeout while Overmark is idle, so
//! it uses no CPU in the tray; nothing of the overlay exists until a hotkey asks for it
//! (docs/ARCHITECTURE.md, The resident app).

#[cfg(windows)]
mod app;
#[cfg(windows)]
pub mod config;
#[cfg(windows)]
mod ink;
pub mod input;
/// The record box's outline and control bar.
#[cfg(windows)]
pub mod record_bar;
/// The Settings window, a process of its own (`--settings`).
#[cfg(windows)]
pub mod settings_window;
#[cfg(windows)]
pub mod toolbar;
/// The egui windows' shared look; Windows-only, as it loads Windows' own fonts.
#[cfg(windows)]
pub mod ui;

#[cfg(windows)]
pub use app::{Args, TEST_MARKER_Y, TEST_PEN_Y, TEST_X0, TEST_X1, run};

/// The app's name for its single-instance objects and its windows.
pub const APP_ID: &str = "Overmark";
