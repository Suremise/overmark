//! Overmark's Windows layer: overlay windows, capture, hotkeys, pointer input, display
//! affinity, the tray, clipboard, notifications, autostart and portable/installed detection.
//!
//! Everything is Windows only: on other platforms the crate is empty, so the workspace still
//! builds there.

/// The core this layer drives.
#[cfg(windows)]
pub use overmark_core as core;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod fonts;
#[cfg(windows)]
pub mod grabber;
#[cfg(windows)]
pub mod hotkeys;
#[cfg(windows)]
pub mod logging;
#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod paths;
#[cfg(windows)]
pub mod record;
#[cfg(windows)]
pub mod region;
#[cfg(windows)]
pub mod save;
#[cfg(windows)]
pub mod update;
#[cfg(windows)]
pub mod win32;

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn links_the_core() {
        assert_eq!(super::core::APP_NAME, "Overmark");
    }
}
