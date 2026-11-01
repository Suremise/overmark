//! A clean grab of the screen without our own windows, while they stay on screen: the
//! Magnification API's magnifier control, at 1:1, renders a monitor with a filter list that
//! leaves out the windows we name, and hands the image to a callback. The ink layer never
//! leaves the screen, so a call's audience sees no flicker (docs/ARCHITECTURE.md, Saving).
//!
//! Found by the ink-layer spike, with its limits:
//! - the control renders only while its host window is on a monitor and the host's size is the
//!   image's: the host is monitor-sized, fully transparent (opacity 0), click-through, never
//!   activated and excluded from capture, so neither the user nor a share sees it;
//! - `MagSetImageScalingCallback` is documented as deprecated. It works on Windows 11 25H2 (and
//!   is reported to on Windows 10); if it ever stops, [`Magnifier::grab`] returns `None` and the
//!   caller falls back (docs/ARCHITECTURE.md, Saving).

use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{HRGN, InvalidateRect, UpdateWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Magnification::{
    MAGIMAGEHEADER, MAGTRANSFORM, MW_FILTERMODE_EXCLUDE, MagInitialize, MagSetImageScalingCallback,
    MagSetWindowFilterList, MagSetWindowSource, MagSetWindowTransform,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_CHILD, WS_VISIBLE,
};
use windows::core::{BOOL, w};

use super::Rect;
use super::capture::Frame;
use super::clock;
use super::window::{self, Options, Window};

/// The last image a magnifier control rendered. The callback's window isn't the control (it is
/// an internal one), so images aren't told apart by window: grabs run one at a time, on the
/// thread that owns the controls.
static LAST: Mutex<Option<Frame>> = Mutex::new(None);
/// Whether a grab is waiting for an image: the control renders again on its own (a repaint),
/// and an image nobody asked for isn't kept (15 MB at 2560x1440).
static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn on_image(
    _hwnd: HWND,
    src: *mut core::ffi::c_void,
    header: MAGIMAGEHEADER,
    _dst: *mut core::ffi::c_void,
    _dst_header: MAGIMAGEHEADER,
    _unclipped: RECT,
    _clipped: RECT,
    _dirty: HRGN,
) -> BOOL {
    if !WANTED.load(std::sync::atomic::Ordering::Acquire) {
        return true.into();
    }
    let (w, h) = (header.width as i32, header.height as i32);
    let Some(len) = super::byte_len(w, h) else {
        return true.into();
    };
    if src.is_null() || (header.stride as usize) < w as usize * 4 {
        return true.into();
    }
    let mut bgra = Vec::with_capacity(len);
    for y in 0..h as usize {
        // SAFETY: the magnifier passes `height` rows of `stride` bytes at `src`, each holding
        // `width` 4-byte pixels (checked above).
        let row = unsafe {
            std::slice::from_raw_parts(
                (src as *const u8).add(y * header.stride as usize),
                w as usize * 4,
            )
        };
        bgra.extend_from_slice(row);
    }
    *LAST.lock().unwrap() = Some(Frame {
        width: w,
        height: h,
        bgra,
    });
    true.into()
}

/// How many magnifiers are alive: the runtime is initialised for the first and torn down
/// after the last, so a closed overlay leaves none of it in memory.
static LIVE: Mutex<usize> = Mutex::new(0);

fn initialise() -> windows::core::Result<()> {
    let mut live = LIVE.lock().unwrap();
    // SAFETY: no arguments; balanced by MagUninitialize when the count drops to 0.
    if *live == 0 && !unsafe { MagInitialize() }.as_bool() {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "MagInitialize failed",
        ));
    }
    *live += 1;
    Ok(())
}

/// One use of the runtime, given back when dropped.
struct Runtime;

impl Drop for Runtime {
    fn drop(&mut self) {
        release();
    }
}

fn release() {
    let mut live = LIVE.lock().unwrap();
    *live = live.saturating_sub(1);
    if *live == 0 {
        // SAFETY: the last magnifier is gone; MagInitialize is called again for the next.
        let _ = unsafe { windows::Win32::UI::Magnification::MagUninitialize() };
    }
}

/// A magnifier of one monitor that leaves out the windows it's told to.
pub struct Magnifier {
    host: Window,
    control: HWND,
    rect: Rect,
    /// Dropped last: the runtime outlives the control and its host.
    _runtime: Runtime,
}

impl Magnifier {
    /// A magnifier of `monitor` (physical pixels), leaving out `exclude`. Its host is hidden
    /// until a grab.
    pub fn new(monitor: Rect, exclude: &[HWND]) -> windows::core::Result<Magnifier> {
        initialise()?;
        let runtime = Runtime;
        let host = Window::new(
            monitor,
            Options {
                click_through: true,
                no_activate: true,
                ..Default::default()
            },
            Box::new(|_, _, _, _| None),
        )?;
        host.set_opacity(0);
        super::affinity::set_excluded(host.hwnd(), true)?;
        // SAFETY: a child magnifier control in our own host window; the transform is a local
        // that outlives the call.
        let control = unsafe {
            let instance = GetModuleHandleW(None)?;
            let control = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("Magnifier"),
                w!("OvermarkMagnifier"),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                monitor.width(),
                monitor.height(),
                Some(host.hwnd()),
                None,
                Some(instance.into()),
                None,
            )?;
            let mut t = MAGTRANSFORM::default();
            t.v[0] = 1.0;
            t.v[4] = 1.0;
            t.v[8] = 1.0;
            if !MagSetWindowTransform(control, &mut t).as_bool()
                || !MagSetImageScalingCallback(control, Some(on_image)).as_bool()
            {
                let e = windows::core::Error::from_thread();
                let _ = DestroyWindow(control);
                return Err(e);
            }
            control
        };
        let m = Magnifier {
            host,
            control,
            rect: monitor,
            _runtime: runtime,
        };
        m.set_excluded(exclude)?;
        Ok(m)
    }

    /// Replaces the windows left out of the image.
    pub fn set_excluded(&self, exclude: &[HWND]) -> windows::core::Result<()> {
        let mut list = exclude.to_vec();
        // SAFETY: `list` outlives the call, which copies it.
        let ok = unsafe {
            MagSetWindowFilterList(
                self.control,
                MW_FILTERMODE_EXCLUDE,
                list.len() as i32,
                list.as_mut_ptr(),
            )
        };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }

    /// The monitor as it is now, without the excluded windows; `None` if the magnifier gave no
    /// monitor-sized image within `timeout_ms` (the caller falls back).
    pub fn grab(&self, timeout_ms: u32) -> Option<Frame> {
        *LAST.lock().unwrap() = None;
        WANTED.store(true, std::sync::atomic::Ordering::Release);
        self.host.show(false);
        let deadline = clock::now_secs() + f64::from(timeout_ms) / 1000.0;
        let mut image = None;
        while image.is_none() && clock::now_secs() < deadline {
            // SAFETY: our own control.
            unsafe {
                let _ = MagSetWindowSource(self.control, self.rect.to_win32());
                let _ = InvalidateRect(Some(self.control), None, false);
                let _ = UpdateWindow(self.control);
            }
            window::pump();
            image = LAST.lock().unwrap().take();
            if image.is_none() {
                window::wait_for_message(2);
            }
        }
        WANTED.store(false, std::sync::atomic::Ordering::Release);
        *LAST.lock().unwrap() = None;
        self.host.hide();
        image.filter(|f| f.width == self.rect.width() && f.height == self.rect.height())
    }
}

impl Drop for Magnifier {
    fn drop(&mut self) {
        // SAFETY: our own control (the host is destroyed by its own Drop).
        unsafe {
            let _ = DestroyWindow(self.control);
        }
    }
}
