//! Keeping a window out of screen capture (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004 and
//! later). An excluded window is still on screen, but every capture (Windows Graphics Capture,
//! Desktop Duplication, BitBlt of the screen, and so Teams and Zoom) sees what's behind it.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

/// Excludes `hwnd` from capture, or includes it again.
pub fn set_excluded(hwnd: HWND, excluded: bool) -> windows::core::Result<()> {
    let affinity = if excluded {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    // SAFETY: `hwnd` is a top-level window of this process (the API refuses others).
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }
}

/// Whether `hwnd` is excluded from capture now.
pub fn is_excluded(hwnd: HWND) -> bool {
    let mut a = 0u32;
    // SAFETY: writes one u32 we own.
    unsafe { GetWindowDisplayAffinity(hwnd, &mut a) }.is_ok() && a == WDA_EXCLUDEFROMCAPTURE.0
}
