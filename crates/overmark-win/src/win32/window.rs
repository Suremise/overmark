//! Borderless top-most popup windows, each with a handler for its messages.

use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetWindowLongPtrW, HWND_TOPMOST, IDC_CROSS, LoadCursorW, MSG,
    PM_REMOVE, PeekMessageW, RegisterClassExW, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_NCCREATE,
    WM_NCDESTROY, WM_QUIT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::Rect;

/// A window's message handler: `Some` answers the message, `None` leaves it to Windows.
/// It may be called again while it runs (a call it makes can send the window a message), so it
/// takes `&self` and keeps its state in cells.
pub type Handler = Box<dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>>;

/// How a window is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Per-pixel alpha through `UpdateLayeredWindow` ([`super::layered`]).
    pub layered: bool,
    /// No GDI surface: its pixels come from DirectComposition ([`super::dcomp`]).
    pub no_redirection_bitmap: bool,
    /// Mouse and pen go through it to the window below (the bench and the dim veil; never the
    /// ink layer, which takes the input).
    pub click_through: bool,
    /// Never takes the focus or comes forward when clicked.
    pub no_activate: bool,
    /// The window's title (checks find windows by it); "Overmark" if `None`. A title the user
    /// may hear (a screen reader names the window by it) comes from Fluent: [`title`].
    pub title: Option<&'static str>,
}

/// The Fluent string `key`, as a window's title: translated once and kept for the process (the
/// language doesn't change while it runs), so it can be an [`Options::title`].
pub fn title(key: &'static str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static TITLES: Mutex<Option<HashMap<&'static str, &'static str>>> = Mutex::new(None);
    let mut titles = TITLES.lock().unwrap_or_else(|e| e.into_inner());
    let titles = titles.get_or_insert_with(HashMap::new);
    if let Some(title) = titles.get(key) {
        return title;
    }
    let title: &'static str = Box::leak(crate::core::i18n::tr(key).into_boxed_str());
    titles.insert(key, title);
    title
}

const CLASS: PCWSTR = w!("OvermarkSurface");

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the `Box<Handler>` leaked in `Window::new`, set at
    // WM_NCCREATE and freed at WM_NCDESTROY; no other code writes it.
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Handler;
        if msg == WM_NCDESTROY {
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr as *mut Handler));
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if !ptr.is_null()
            && let Some(r) = (*ptr)(hwnd, msg, wp, lp)
        {
            return r;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn register_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: registers a class whose strings are static; the cursor is a system one.
        unsafe {
            let instance = GetModuleHandleW(None).expect("GetModuleHandleW");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                hbrBackground: HBRUSH::default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
    });
}

/// A top-most, borderless popup window, destroyed when dropped.
pub struct Window {
    hwnd: HWND,
}

impl Window {
    /// Makes the window over `rect` (physical pixels), hidden until [`Window::show`].
    pub fn new(rect: Rect, options: Options, handler: Handler) -> windows::core::Result<Window> {
        register_class();
        let mut ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        if options.layered {
            ex |= WS_EX_LAYERED;
        }
        if options.no_redirection_bitmap {
            ex |= WS_EX_NOREDIRECTIONBITMAP;
        }
        if options.click_through {
            // Click-through needs WS_EX_LAYERED too.
            ex |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
        }
        if options.no_activate {
            ex |= WS_EX_NOACTIVATE;
        }
        let boxed: *mut Handler = Box::into_raw(Box::new(handler));
        // SAFETY: the class is registered; `boxed` becomes the window's (see `wndproc`).
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None)?;
            CreateWindowExW(
                WINDOW_EX_STYLE(ex.0),
                CLASS,
                &windows::core::HSTRING::from(options.title.unwrap_or("Overmark")),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                None,
                None,
                Some(instance.into()),
                Some(boxed as *const _),
            )
        };
        match hwnd {
            Ok(hwnd) => {
                if options.click_through && !options.layered {
                    // Click-through needs WS_EX_LAYERED, and a layered window shows nothing
                    // until its layered attributes are set: fully opaque, so the content
                    // (DirectComposition's) shows as it is.
                    // SAFETY: our own window.
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(
                            hwnd,
                            windows::Win32::Foundation::COLORREF(0),
                            255,
                            windows::Win32::UI::WindowsAndMessaging::LWA_ALPHA,
                        );
                    }
                }
                Ok(Window { hwnd })
            }
            // If creation failed after WM_NCCREATE, WM_NCDESTROY freed the handler; before it,
            // the handler is leaked, which is rare enough not to matter.
            Err(e) => Err(e),
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// The client area's size, in physical pixels.
    pub fn client_size(&self) -> (i32, i32) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        // SAFETY: our own window; writes a RECT we own.
        let _ =
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(self.hwnd, &mut rc) };
        (rc.right - rc.left, rc.bottom - rc.top)
    }

    /// Fills the client area with an opaque colour through GDI, as an ordinary app paints
    /// (the bench's baseline for the time to the screen).
    pub fn fill_gdi(&self, r: u8, g: u8, b: u8) {
        use windows::Win32::Graphics::Gdi::{
            CreateSolidBrush, DeleteObject, FillRect, GdiFlush, GetDC, ReleaseDC,
        };
        let (w, h) = self.client_size();
        // SAFETY: our own window's DC, released below; the brush is deleted after use.
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16),
            ));
            let rc = Rect::new(0, 0, w, h).to_win32();
            FillRect(dc, &rc, brush);
            let _ = GdiFlush();
            let _ = DeleteObject(brush.into());
            ReleaseDC(Some(self.hwnd), dc);
        }
    }

    /// Makes a layered window (made with `click_through`) see-through: 255 opaque, 0 invisible.
    pub fn set_opacity(&self, alpha: u8) {
        // SAFETY: our own window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(
                self.hwnd,
                windows::Win32::Foundation::COLORREF(0),
                alpha,
                windows::Win32::UI::WindowsAndMessaging::LWA_ALPHA,
            );
        }
    }

    /// The DPI Windows gives this window (its monitor's, under per-monitor awareness v2).
    pub fn dpi(&self) -> u32 {
        // SAFETY: our own window.
        unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd) }
    }

    /// Shows the window, taking the focus only if `activate`.
    pub fn show(&self, activate: bool) {
        // SAFETY: our own window.
        unsafe {
            let _ = ShowWindow(
                self.hwnd,
                if activate { SW_SHOW } else { SW_SHOWNOACTIVATE },
            );
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | if activate {
                        Default::default()
                    } else {
                        SWP_NOACTIVATE
                    },
            );
        }
    }
}

impl Window {
    /// Puts the window just below `above` in the z-order (both top-most), without activating.
    pub fn place_below(&self, above: HWND) {
        use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
        // SAFETY: our own windows.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(above),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Brings the window forward and gives it the keyboard focus. Windows allows it after one
    /// of our hotkeys, or when the launch that asked for it gave us the right
    /// (`AllowSetForegroundWindow`); otherwise the window flashes in the taskbar instead.
    pub fn focus(&self) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        // SAFETY: our own window.
        unsafe {
            let ok = SetForegroundWindow(self.hwnd).as_bool();
            let _ = SetFocus(Some(self.hwnd));
            ok
        }
    }

    /// Hides the window (it keeps its contents).
    pub fn hide(&self) {
        // SAFETY: our own window.
        unsafe {
            let _ = ShowWindow(self.hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: our own window; its handler is freed at WM_NCDESTROY.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// The top-level window a click at (`x`, `y`) (screen pixels) would go to, as Windows hit-tests
/// it: a layered window's fully transparent pixels let it through to the window below.
pub fn window_at(x: i32, y: i32) -> HWND {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
    // SAFETY: plain queries.
    unsafe {
        let h = WindowFromPoint(windows::Win32::Foundation::POINT { x, y });
        GetAncestor(h, GA_ROOT)
    }
}

/// The calling thread's id, as [`WindowInfo::thread`] gives a window's.
pub fn this_thread() -> u32 {
    // SAFETY: no arguments.
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

/// A top-level window of some process, as checks see it.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    /// The thread that made it.
    pub thread: u32,
    pub class: String,
    pub title: String,
    pub visible: bool,
    pub ex_style: u32,
    /// A layered window's opacity from `SetLayeredWindowAttributes` (0-255), if it has one.
    pub alpha: Option<u8>,
    pub rect: Rect,
}

/// The top-level windows of process `pid` (ours, or one we started), with their classes.
pub fn process_windows(pid: u32) -> Vec<WindowInfo> {
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetClassNameW, GetLayeredWindowAttributes, GetWindowLongW,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
    };
    struct Found {
        pid: u32,
        out: Vec<WindowInfo>,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        // SAFETY: `data` is the `Found` below, alive for the enumeration; the queries are about
        // a window Windows just gave, into buffers we own.
        unsafe {
            let found = &mut *(data.0 as *mut Found);
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == found.pid {
                let mut class = [0u16; 128];
                let n = GetClassNameW(hwnd, &mut class).max(0) as usize;
                let mut title = [0u16; 128];
                let t = GetWindowTextW(hwnd, &mut title).max(0) as usize;
                let (mut key, mut alpha, mut flags) =
                    (Default::default(), 0u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
                let has_alpha = GetLayeredWindowAttributes(
                    hwnd,
                    Some(&mut key),
                    Some(&mut alpha),
                    Some(&mut flags),
                )
                .is_ok()
                    && flags.contains(LWA_ALPHA);
                let mut rc = windows::Win32::Foundation::RECT::default();
                let _ = GetWindowRect(hwnd, &mut rc);
                found.out.push(WindowInfo {
                    hwnd,
                    thread,
                    class: String::from_utf16_lossy(&class[..n]),
                    title: String::from_utf16_lossy(&title[..t]),
                    visible: IsWindowVisible(hwnd).as_bool(),
                    ex_style: GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
                    alpha: has_alpha.then_some(alpha),
                    rect: Rect::from_win32(rc),
                });
            }
        }
        true.into()
    }
    let mut found = Found {
        pid,
        out: Vec::new(),
    };
    // SAFETY: the callback only touches `found`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize)) };
    found.out
}

/// Posts a message to a window of ours (or of a process we started): how checks drive the
/// app without global input.
pub fn post(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    // SAFETY: posts to a window's queue; a stale handle fails.
    unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam)) }.is_ok()
}

/// Posts a key press (`WM_KEYDOWN` with virtual key `vk`) to one of our own windows: how
/// tests and smoke runs drive a window without global input.
pub fn post_key(hwnd: HWND, vk: u16) {
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    // SAFETY: posts to our own window's queue.
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(usize::from(vk)), LPARAM(0));
    }
}

/// Puts `hwnd` (any process's) at the top of the top-most windows without activating it: what
/// a window's activation does to the z-order, for checks to do to a layer without the
/// foreground.
pub fn to_top(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
    // SAFETY: a z-order change of plain numbers; a stale handle fails harmlessly.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

/// Whether window `a` is above window `b` in the z-order (`false` if neither is a window).
pub fn is_above(a: HWND, b: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GW_HWNDNEXT, GetTopWindow, GetWindow};
    // SAFETY: plain queries walking the z-order from the top.
    let mut h = unsafe { GetTopWindow(None) }.ok();
    while let Some(w) = h {
        if w == a || w == b {
            return w == a;
        }
        h = unsafe { GetWindow(w, GW_HWNDNEXT) }.ok();
    }
    false
}

/// Sends the mouse to `hwnd` until [`release_mouse`], so a drag that leaves it still reaches it.
pub fn capture_mouse(hwnd: HWND) {
    // SAFETY: our own window.
    let _ = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd) };
}

/// Ends [`capture_mouse`].
pub fn release_mouse() {
    // SAFETY: gives back the capture this thread took.
    let _ = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture() };
}

/// The window the user is working in (the foreground window), if any.
pub fn foreground() -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: a plain query.
    let h = unsafe { GetForegroundWindow() };
    (!h.is_invalid()).then_some(h)
}

/// Gives the focus back to `hwnd` (another app's window, from [`foreground`]), if it's still
/// there. Windows allows it while this process is in the foreground, as it is when the layer
/// closes.
pub fn give_focus_back(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    // SAFETY: IsWindow is safe for any handle; SetForegroundWindow takes a live one.
    unsafe { IsWindow(Some(hwnd)).as_bool() && SetForegroundWindow(hwnd).as_bool() }
}

/// Whether Ctrl, Shift and Alt were down when the message being handled was sent (this
/// thread's key state, as the message queue keeps it: no global input is read).
pub fn modifiers() -> (bool, bool, bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
    // SAFETY: plain queries of this thread's key state.
    let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
    (down(VK_CONTROL.0), down(VK_SHIFT.0), down(VK_MENU.0))
}

/// Handles every message waiting for this thread's windows. `false` once WM_QUIT came.
pub fn pump() -> bool {
    let mut msg = MSG::default();
    // SAFETY: the standard message loop on this thread's queue.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}

/// Waits for DWM's next composition (the next refresh), as an app drawing once a frame does.
pub fn wait_for_composition() {
    // SAFETY: no arguments.
    let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
}

/// Waits until a message comes for this thread, or `timeout_ms` passes.
pub fn wait_for_message(timeout_ms: u32) {
    use windows::Win32::UI::WindowsAndMessaging::{MsgWaitForMultipleObjects, QS_ALLINPUT};
    // SAFETY: waits on no handles, only this thread's queue.
    unsafe {
        let _ = MsgWaitForMultipleObjects(None, false, timeout_ms, QS_ALLINPUT);
    }
}

#[cfg(test)]
mod tests {
    use super::title;
    use crate::core::i18n::tr;

    /// A window's title is its Fluent string, made once (none is written in the code).
    #[test]
    fn a_title_is_its_fluent_string_kept() {
        for key in ["window-selection-name", "window-notice-name"] {
            assert_eq!(title(key), tr(key));
            assert!(std::ptr::eq(title(key), title(key)), "made once");
        }
        assert_eq!(title("window-notice-name"), "Overmark notice");
    }
}
