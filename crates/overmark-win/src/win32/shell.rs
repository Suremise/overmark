//! Small things from the shell and the system: the local time for a capture's name, the
//! monitor under the cursor, and opening a file or showing it in its folder.

use std::path::Path;

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SW_SHOWNORMAL};
use windows::core::{HSTRING, w};

use crate::core::settings::file_name::CaptureTime;
use crate::win32::dpi::{self, Monitor};

/// The local time now, for a capture's file name.
pub fn capture_time() -> CaptureTime {
    // SAFETY: fills a SYSTEMTIME we own.
    let t = unsafe { GetLocalTime() };
    CaptureTime {
        year: i32::from(t.wYear),
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
        millisecond: t.wMilliseconds,
    }
}

/// The cursor's position on screen, in physical pixels.
pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: writes a POINT we own.
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

/// The monitor the cursor is on (the primary if it's on none).
pub fn monitor_under_cursor() -> Option<Monitor> {
    let (x, y) = cursor();
    // SAFETY: a plain query.
    let h = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTOPRIMARY) };
    dpi::describe(h)
}

/// Opens `path` with its app, as a double-click does.
pub fn open(path: &Path) -> bool {
    open_or_why(path).is_ok()
}

/// [`open`], with Windows' reason (in the user's language) when it can't.
pub fn open_or_why(path: &Path) -> Result<(), String> {
    // SAFETY: strings that outlive the call.
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path.as_os_str()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    shell_result(r.0 as isize)
}

/// Opens Explorer on `path`'s folder with `path` selected (or the folder itself, given one).
pub fn show_in_folder(path: &Path) -> bool {
    show_in_folder_or_why(path).is_ok()
}

/// [`show_in_folder`], with Windows' reason when it can't.
pub fn show_in_folder_or_why(path: &Path) -> Result<(), String> {
    let args = if path.is_dir() {
        format!("\"{}\"", path.display())
    } else {
        format!("/select,\"{}\"", path.display())
    };
    // SAFETY: strings that outlive the call.
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("explorer.exe"),
            &HSTRING::from(args),
            None,
            SW_SHOWNORMAL,
        )
    };
    shell_result(r.0 as isize)
}

/// `ShellExecuteW`'s result: above 32 it worked; otherwise the thread's last error says why.
fn shell_result(r: isize) -> Result<(), String> {
    if r > 32 {
        return Ok(());
    }
    let message = windows::core::Error::from_thread().message();
    Err(if message.trim().is_empty() {
        format!("{r}")
    } else {
        message.trim().to_owned()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_time_is_a_real_date() {
        let t = super::capture_time();
        assert!(t.year >= 2026 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
        assert!(t.hour < 24 && t.minute < 60 && t.second < 61);
    }

    #[test]
    fn a_failed_open_says_why() {
        // Above 32 it worked; at or below, there's a reason (Windows' message, or the code).
        assert_eq!(super::shell_result(42), Ok(()));
        let why = super::shell_result(2).expect_err("2 is a failure");
        assert!(!why.is_empty());
    }

    #[test]
    fn the_cursor_is_on_a_monitor() {
        assert!(super::monitor_under_cursor().is_some());
    }
}
