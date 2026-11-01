//! Per-monitor DPI awareness v2, and the monitors in physical pixels.
//!
//! With PMv2 every window and monitor rectangle is in physical pixels, whatever each monitor's
//! scale, so one ink layer per monitor covers it pixel for pixel; the scale (`dpi / 96`) is only
//! for sizing UI such as the toolbar.

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::core::BOOL;

use super::Rect;

/// Makes the process per-monitor DPI aware (v2). Call it before any window is made. The app's
/// manifest will say the same; this is for the spike and for tests.
pub fn enable_per_monitor_v2() -> windows::core::Result<()> {
    // SAFETY: takes a constant context; fails only if awareness was already set.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
}

/// One monitor, in physical pixels (virtual-screen coordinates).
#[derive(Debug, Clone)]
pub struct Monitor {
    pub handle: HMONITOR,
    /// The device name (`\\.\DISPLAY1`).
    pub name: String,
    pub rect: Rect,
    /// The working area (without the taskbar).
    pub work: Rect,
    /// The effective DPI: 96 at 100 %, 144 at 150 %.
    pub dpi: u32,
    pub primary: bool,
}

impl Monitor {
    /// The scale Windows uses for this monitor's UI: 1.0 at 100 %, 1.5 at 150 %.
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// The refresh rate of its current display mode, in Hz.
    pub fn refresh_hz(&self) -> u32 {
        use windows::Win32::Graphics::Gdi::{
            DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
        };
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let name = windows::core::HSTRING::from(self.name.as_str());
        // SAFETY: writes a DEVMODEW with its size set, for a device name we own.
        let ok = unsafe { EnumDisplaySettingsW(&name, ENUM_CURRENT_SETTINGS, &mut mode) };
        if ok.as_bool() {
            mode.dmDisplayFrequency
        } else {
            60
        }
    }
}

/// Every monitor, the primary first.
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed below, alive for the call.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(m);
        true.into()
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which outlives the call.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(each),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    let mut out: Vec<Monitor> = handles.into_iter().filter_map(describe).collect();
    out.sort_by_key(|m| !m.primary);
    out
}

/// One monitor's description, or `None` if it went away meanwhile.
pub fn describe(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` is a MONITORINFOEXW with its size set, as the API asks.
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return None;
    }
    let (mut dx, mut dy) = (96u32, 96u32);
    // SAFETY: writes two u32s we own.
    let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(32);
    Some(Monitor {
        handle,
        name: String::from_utf16_lossy(&info.szDevice[..len]),
        rect: Rect::from_win32(info.monitorInfo.rcMonitor),
        work: Rect::from_win32(info.monitorInfo.rcWork),
        dpi: dx,
        primary: info.monitorInfo.dwFlags & 1 != 0,
    })
}
