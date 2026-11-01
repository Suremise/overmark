//! The tray: an icon in the notification area with a tooltip, balloons and a right-click menu,
//! one instance per user session, and the wait that drives the app's message loop.
//!
//! Adapted from Replica's tray scaffold (`replica-win/src/tray.rs` and `win32/ui.rs`, "ready to
//! copy"): its own `Shell_NotifyIconW` icon rather than the `tray-icon` crate, so balloons come
//! from the same icon in every mode, portable included. [`IconFile`], [`ThemedIcon`],
//! [`taskbar_is_light`], the icon's sizing and the checked menu items are Replica's as they are.
//!
//! The icon follows the taskbar's theme and the monitor's DPI: Windows tells the tray's window
//! ([`TrayEvent::ThemeChanged`]) and the app calls [`Tray::refresh`].

use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::OnceLock;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0,
    WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO,
    NIIF_RESPECT_QUIET_TIME, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW,
    HICON, LR_DEFAULTCOLOR, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, MWMO_INPUTAVAILABLE,
    MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostMessageW, QS_ALLINPUT,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WINDOW_EX_STYLE, WM_APP,
    WM_CONTEXTMENU, WM_DPICHANGED, WM_LBUTTONDBLCLK, WM_NULL, WM_QUIT, WM_SETTINGCHANGE,
    WM_THEMECHANGED, WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::registry;

/// The notify icon's callback message.
const CALLBACK: u32 = WM_APP + 1;
/// A [`Waker`]'s message: its code is the `WPARAM`.
const WAKE: u32 = WM_APP + 2;
/// With `NOTIFYICON_VERSION_4`: a click or Enter on the icon, and a click on a balloon.
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

/// The window class of the tray's hidden window: what another process looks for.
pub const WINDOW_CLASS: &str = "Overmark.NotifyWindow";

/// What the user did with the icon, or what another process or thread asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// A click on the icon. A double-click sends this first, then [`TrayEvent::DoubleClick`].
    Activate,
    /// Enter (or Space) on the icon, from the keyboard: never part of a double-click.
    KeyActivate,
    /// A double-click on the icon.
    DoubleClick,
    /// A right-click (or the menu key) on the icon, at these screen coordinates.
    Menu { x: i32, y: i32 },
    /// A click on a balloon.
    BalloonClicked,
    /// The taskbar's theme, the DPI or Explorer changed: call [`Tray::refresh`].
    ThemeChanged,
    /// A [`Waker`] woke the loop, with its code.
    Woken(u32),
    /// Another launch asked this instance to open the layer: `frozen` for Frozen.
    OpenRequested { frozen: bool },
    /// The Settings window saved the settings: read them again ([`signal_reload`]).
    ReloadRequested,
    /// Another process asked this instance to quit (an uninstall or an upgrade).
    QuitRequested,
    /// WM_QUIT came: the loop should end.
    Quit,
}

thread_local! {
    static EVENTS: RefCell<VecDeque<TrayEvent>> = const { RefCell::new(VecDeque::new()) };
}

fn push(event: TrayEvent) {
    EVENTS.with(|e| e.borrow_mut().push_back(event));
}

fn taskbar_created() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    // SAFETY: a static, NUL-terminated name.
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

thread_local! {
    /// The icon to add again when Explorer restarts: (window, id).
    static READD: RefCell<Option<NOTIFYICONDATAW>> = const { RefCell::new(None) };
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == CALLBACK {
        match (lparam.0 as u32) & 0xFFFF {
            NIN_SELECT => push(TrayEvent::Activate),
            NIN_KEYSELECT => push(TrayEvent::KeyActivate),
            WM_LBUTTONDBLCLK => push(TrayEvent::DoubleClick),
            WM_CONTEXTMENU => push(TrayEvent::Menu {
                x: (wparam.0 & 0xFFFF) as u16 as i16 as i32,
                y: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
            }),
            NIN_BALLOONUSERCLICK => push(TrayEvent::BalloonClicked),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == WAKE {
        push(TrayEvent::Woken(wparam.0 as u32));
        return LRESULT(0);
    }
    if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DPICHANGED) {
        push(TrayEvent::ThemeChanged);
    }
    if msg == taskbar_created() {
        READD.with(|r| {
            if let Some(data) = r.borrow().as_ref() {
                // SAFETY: a fully initialised NOTIFYICONDATAW for our own window.
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_ADD, data);
                    let mut v = *data;
                    v.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                    let _ = Shell_NotifyIconW(NIM_SETVERSION, &v);
                }
            }
        });
        // The new taskbar may have another theme or DPI.
        push(TrayEvent::ThemeChanged);
    }
    // SAFETY: passes the message on unchanged to the default handler.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn window_class() -> windows::core::Result<PCWSTR> {
    static REGISTERED: OnceLock<Result<(), i32>> = OnceLock::new();
    let class = w!("Overmark.NotifyWindow");
    let result = REGISTERED.get_or_init(|| {
        // SAFETY: the class name is static and the window procedure lives for the program.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.code().0)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                Err(GetLastError().to_hresult().0)
            } else {
                Ok(())
            }
        }
    });
    match result {
        Ok(()) => Ok(class),
        Err(code) => Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            *code,
        ))),
    }
}

/// Copies `text` into a fixed-size wide buffer, cut short to fit, NUL-terminated.
fn fill(buffer: &mut [u16], text: &str) {
    let max = buffer.len() - 1;
    let mut n = 0;
    for unit in text.encode_utf16().take(max) {
        buffer[n] = unit;
        n += 1;
    }
    buffer[n] = 0;
}

/// Where Windows records whether the taskbar is light.
const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

/// Whether the taskbar is light (so a dark glyph shows on it). Dark when Windows doesn't say,
/// as it is by default.
pub fn taskbar_is_light() -> bool {
    registry::get_dword(PERSONALIZE, "SystemUsesLightTheme")
        .ok()
        .flatten()
        .is_some_and(|v| v != 0)
}

/// How long Windows waits for a second click to make a double-click.
pub fn double_click_ms() -> u32 {
    // SAFETY: a plain query.
    unsafe { GetDoubleClickTime() }
}

/// The images of a `.ico` file, to pick a size from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconFile(pub &'static [u8]);

impl IconFile {
    /// Each image's size and data (a PNG or a DIB, as stored).
    pub fn images(&self) -> Vec<(u32, &'static [u8])> {
        let bytes = self.0;
        let read16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read32 = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        if bytes.len() < 6 || read16(0) != 0 || read16(2) != 1 {
            return Vec::new();
        }
        (0..read16(4) as usize)
            .filter_map(|i| {
                let entry = 6 + i * 16;
                if entry + 16 > bytes.len() {
                    return None;
                }
                let size = match bytes[entry] {
                    0 => 256,
                    n => u32::from(n),
                };
                let (length, offset) = (read32(entry + 8) as usize, read32(entry + 12) as usize);
                bytes
                    .get(offset..offset.checked_add(length)?)
                    .map(|image| (size, image))
            })
            .collect()
    }

    /// The image for `size` pixels: that size, else the next bigger, else the biggest.
    pub fn image_for(&self, size: u32) -> Option<(u32, &'static [u8])> {
        let mut images = self.images();
        images.sort_by_key(|(s, _)| *s);
        images
            .iter()
            .find(|(s, _)| *s >= size)
            .or(images.last())
            .copied()
    }
}

/// One icon in two variants: for a light taskbar (a dark glyph) and for a dark one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemedIcon {
    pub light_taskbar: IconFile,
    pub dark_taskbar: IconFile,
}

impl ThemedIcon {
    /// The variant for a light taskbar, or a dark one.
    pub fn pick(&self, light: bool) -> IconFile {
        if light {
            self.light_taskbar
        } else {
            self.dark_taskbar
        }
    }
}

/// An icon made from one image of a `.ico` file, destroyed when dropped.
struct OwnedIcon(HICON);

impl OwnedIcon {
    fn from_file(file: IconFile, size: u32) -> windows::core::Result<OwnedIcon> {
        let (actual, image) = file.image_for(size).ok_or_else(|| {
            windows::core::Error::new(windows::Win32::Foundation::E_INVALIDARG, "an empty .ico")
        })?;
        let size = actual.min(size) as i32;
        // SAFETY: Windows reads `image` (a live slice) and makes its own icon from it.
        unsafe { CreateIconFromResourceEx(image, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR) }
            .map(OwnedIcon)
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: made by CreateIconFromResourceEx and destroyed once.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

/// The kind of a balloon, which picks its little icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalloonKind {
    Info,
    Warning,
    Error,
}

/// A popup-menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    Command {
        id: u32,
        text: String,
    },
    /// A command with a tick beside it when `checked`.
    Check {
        id: u32,
        text: String,
        checked: bool,
    },
    Separator,
}

/// Wakes the tray's loop from another thread, with a code ([`TrayEvent::Woken`]).
#[derive(Debug, Clone, Copy)]
pub struct Waker(isize);

impl Waker {
    /// Wakes the loop. Returns false if the tray is gone.
    pub fn wake(&self, code: u32) -> bool {
        // SAFETY: posting to a window by handle is safe from any thread; a gone window fails.
        unsafe {
            PostMessageW(
                Some(HWND(self.0 as *mut _)),
                WAKE,
                WPARAM(code as usize),
                LPARAM(0),
            )
        }
        .is_ok()
    }
}

/// The icon in the notification area. It lives on the thread that made it.
pub struct Tray {
    hwnd: HWND,
    icon: OwnedIcon,
    current: ThemedIcon,
    light: bool,
    size: u32,
    tip: String,
    _not_send: PhantomData<*const ()>,
}

impl Tray {
    /// Adds `icon` (its variant for the taskbar's theme) with the tooltip `tip`.
    pub fn new(icon: ThemedIcon, tip: &str) -> windows::core::Result<Tray> {
        let hwnd = hidden_window()?;
        let light = taskbar_is_light();
        let size = small_icon_size(hwnd);
        let made = OwnedIcon::from_file(icon.pick(light), size);
        let first = match made {
            Ok(i) => i,
            Err(e) => {
                // SAFETY: our window, destroyed once.
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(e);
            }
        };
        let tray = Tray {
            hwnd,
            icon: first,
            current: icon,
            light,
            size,
            tip: tip.to_owned(),
            _not_send: PhantomData,
        };
        tray.add()?;
        Ok(tray)
    }

    /// For another thread to wake the loop.
    pub fn waker(&self) -> Waker {
        Waker(self.hwnd.0 as isize)
    }

    /// Which variant shows now: true for a light taskbar's.
    pub fn is_light(&self) -> bool {
        self.light
    }

    /// The icon showing now.
    pub fn icon(&self) -> ThemedIcon {
        self.current
    }

    /// Shows another icon (a new state).
    pub fn set_icon(&mut self, icon: ThemedIcon) -> windows::core::Result<()> {
        if icon == self.current {
            return Ok(());
        }
        self.current = icon;
        self.size = 0;
        self.refresh()
    }

    /// Follows a change of the taskbar's theme or of DPI ([`TrayEvent::ThemeChanged`]).
    pub fn refresh(&mut self) -> windows::core::Result<()> {
        let light = taskbar_is_light();
        let size = small_icon_size(self.hwnd);
        if light == self.light && size == self.size {
            return Ok(());
        }
        let icon = OwnedIcon::from_file(self.current.pick(light), size)?;
        let old = std::mem::replace(&mut self.icon, icon);
        self.light = light;
        self.size = size;
        let data = self.data(NIF_ICON);
        READD.with(|r| {
            if let Some(d) = r.borrow_mut().as_mut() {
                d.hIcon = data.hIcon;
            }
        });
        let shown = self.notify(NIM_MODIFY, &data);
        // The old icon goes only once the new one shows.
        drop(old);
        shown
    }

    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: flags,
            uCallbackMessage: CALLBACK,
            hIcon: self.icon.0,
            ..Default::default()
        };
        fill(&mut data.szTip, &self.tip);
        data
    }

    fn notify(
        &self,
        message: NOTIFY_ICON_MESSAGE,
        data: &NOTIFYICONDATAW,
    ) -> windows::core::Result<()> {
        // SAFETY: `data` is a fully initialised NOTIFYICONDATAW for our window.
        if unsafe { Shell_NotifyIconW(message, data) }.as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Shell_NotifyIcon failed",
            ))
        }
    }

    fn add(&self) -> windows::core::Result<()> {
        let mut data = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        READD.with(|r| *r.borrow_mut() = Some(data));
        self.notify(NIM_ADD, &data)?;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        self.notify(NIM_SETVERSION, &data)
    }

    pub fn set_tooltip(&mut self, tip: &str) -> windows::core::Result<()> {
        if self.tip == tip {
            return Ok(());
        }
        self.tip = tip.to_owned();
        let data = self.data(NIF_TIP | NIF_SHOWTIP);
        READD.with(|r| {
            if let Some(d) = r.borrow_mut().as_mut() {
                d.szTip = data.szTip;
            }
        });
        self.notify(NIM_MODIFY, &data)
    }

    /// The tooltip showing now.
    pub fn tooltip(&self) -> &str {
        &self.tip
    }

    /// Shows a balloon from the icon (a notification; quiet during "do not disturb").
    pub fn balloon(&self, title: &str, body: &str, kind: BalloonKind) -> windows::core::Result<()> {
        let mut data = self.data(NIF_INFO);
        fill(&mut data.szInfoTitle, title);
        fill(&mut data.szInfo, body);
        data.dwInfoFlags = NIIF_RESPECT_QUIET_TIME
            | match kind {
                BalloonKind::Info => NIIF_INFO,
                BalloonKind::Warning => NIIF_WARNING,
                BalloonKind::Error => NIIF_ERROR,
            };
        self.notify(NIM_MODIFY, &data)
    }

    /// Lets the process that `pid` names take the foreground (a window this one starts).
    pub fn allow_foreground(pid: u32) {
        // SAFETY: a process-wide permission with no pointers.
        let _ = unsafe { AllowSetForegroundWindow(pid) };
    }

    /// Shows `items` at the screen point `(x, y)` and returns the chosen command's ID.
    pub fn show_menu(
        &self,
        items: &[MenuItem],
        x: i32,
        y: i32,
    ) -> windows::core::Result<Option<u32>> {
        // SAFETY: the menu is ours and destroyed below; the window must be foreground for the
        // menu to close when the user clicks elsewhere, and the WM_NULL afterwards is the
        // documented companion to that.
        unsafe {
            let menu = CreatePopupMenu()?;
            for item in items {
                let r = match item {
                    MenuItem::Command { id, text } => {
                        AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(text.as_str()))
                    }
                    MenuItem::Check { id, text, checked } => {
                        let flags = if *checked {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        };
                        AppendMenuW(menu, flags, *id as usize, &HSTRING::from(text.as_str()))
                    }
                    MenuItem::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
                };
                if let Err(e) = r {
                    let _ = DestroyMenu(menu);
                    return Err(e);
                }
            }
            let _ = SetForegroundWindow(self.hwnd);
            let chosen = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0,
                x,
                y,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            Ok((chosen.0 > 0).then_some(chosen.0 as u32))
        }
    }
}

/// The tray's hidden window, of its registered class.
fn hidden_window() -> windows::core::Result<HWND> {
    let class = window_class()?;
    // SAFETY: a hidden tool window of our registered class; its owner destroys it.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            class,
            w!("Overmark"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        )
    }
}

/// The small-icon size for `hwnd`'s monitor, in pixels (16 at 100%).
fn small_icon_size(hwnd: HWND) -> u32 {
    // SAFETY: a query about our own live window.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd).max(96)) };
    size.max(16) as u32
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data(NOTIFY_ICON_DATA_FLAGS(0));
        let _ = self.notify(NIM_DELETE, &data);
        READD.with(|r| *r.borrow_mut() = None);
        // SAFETY: our window, destroyed once on its own thread.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// A named, auto-reset event: this process waits on it, others set it.
pub struct NamedEvent(HANDLE);

// SAFETY: an event handle may be used from any thread.
unsafe impl Send for NamedEvent {}
unsafe impl Sync for NamedEvent {}

impl NamedEvent {
    /// Creates (or opens) the event `name`.
    pub fn create(name: &str) -> windows::core::Result<NamedEvent> {
        // SAFETY: the name outlives the call; default security.
        unsafe { CreateEventW(None, false, false, &HSTRING::from(name)) }.map(NamedEvent)
    }

    /// Sets the existing event `name`.
    pub fn signal(name: &str) -> windows::core::Result<()> {
        // SAFETY: the handle is closed straight after use.
        unsafe {
            let handle = OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(name))?;
            let result = SetEvent(handle);
            let _ = CloseHandle(handle);
            result
        }
    }

    /// Waits up to `timeout_ms` for the event to be set (`None`: as long as it takes).
    /// Returns whether it was.
    pub fn wait(&self, timeout_ms: Option<u32>) -> bool {
        use windows::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: our live handle.
        unsafe { WaitForSingleObject(self.0, timeout_ms.unwrap_or(INFINITE)) == WAIT_OBJECT_0 }
    }
}

impl Drop for NamedEvent {
    fn drop(&mut self) {
        // SAFETY: closes our handle once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Proof that this is the only instance: a named mutex held while it lives.
pub struct InstanceLock(HANDLE);

// SAFETY: a mutex handle may be closed from any thread (it was made with initial ownership
// only to learn whether it existed; closing it is what releases the name).
unsafe impl Send for InstanceLock {}

impl InstanceLock {
    /// Takes the name `name`, or `None` if another holds it.
    pub fn acquire(name: &str) -> windows::core::Result<Option<InstanceLock>> {
        // SAFETY: the name outlives the call; GetLastError is read straight after it.
        unsafe {
            let handle = CreateMutexW(None, true, &HSTRING::from(name))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(InstanceLock(handle)))
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // SAFETY: closing the handle releases the mutex (owned since creation).
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// One instance of the app per user session.
pub struct SingleInstance {
    _lock: InstanceLock,
    open: NamedEvent,
    open_frozen: NamedEvent,
    quit: NamedEvent,
    reload: NamedEvent,
}

/// What [`SingleInstance::acquire`] found.
pub enum Instance {
    /// This is the only one.
    First(SingleInstance),
    /// Another was running; it has been asked to open the layer.
    Second,
}

impl SingleInstance {
    /// Becomes the one instance of `app` (a name such as `Overmark`) in this session, or, if
    /// there already is one, asks it to open the layer (`frozen`: Frozen) and lets it take the
    /// foreground (this launch has the right to give it; the running one doesn't).
    pub fn acquire(app: &str, frozen: bool) -> windows::core::Result<Instance> {
        let open = format!(r"Local\{app}.Open");
        let open_frozen = format!(r"Local\{app}.OpenFrozen");
        match InstanceLock::acquire(&format!(r"Local\{app}.Instance"))? {
            Some(lock) => Ok(Instance::First(SingleInstance {
                _lock: lock,
                open: NamedEvent::create(&open)?,
                open_frozen: NamedEvent::create(&open_frozen)?,
                quit: NamedEvent::create(&format!(r"Local\{app}.Quit"))?,
                reload: NamedEvent::create(&reload_name(app))?,
            })),
            None => {
                // SAFETY: a process-wide permission with no pointers.
                let _ = unsafe {
                    AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY)
                };
                let event = if frozen { &open_frozen } else { &open };
                // The first instance may still be starting: its events come a moment after its
                // lock.
                for _ in 0..50 {
                    if NamedEvent::signal(event).is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Ok(Instance::Second)
            }
        }
    }
}

fn reload_name(app: &str) -> String {
    format!(r"Local\{app}.Reload")
}

/// Asks the running instance of `app`, if any, to read its settings again (the Settings window
/// saved them). Returns whether one was asked.
pub fn signal_reload(app: &str) -> bool {
    NamedEvent::signal(&reload_name(app)).is_ok()
}

/// Asks the running instance of `app`, if any, to quit. Returns whether one was asked.
pub fn request_quit(app: &str) -> bool {
    NamedEvent::signal(&format!(r"Local\{app}.Quit")).is_ok()
}

/// Whether an instance of `app` is running in this session (without asking it anything).
pub fn is_running(app: &str) -> bool {
    matches!(
        InstanceLock::acquire(&format!(r"Local\{app}.Instance")),
        Ok(None)
    )
}

/// Asks the running instance of `app`, if any, to quit, and waits up to `wait_ms` for it to
/// end, so this launch can take its place (a launch with options the running one wouldn't
/// take, such as `--dim`). Returns whether none is running now.
pub fn replace_running(app: &str, wait_ms: u32) -> bool {
    let lock = format!(r"Local\{app}.Instance");
    let free = || matches!(InstanceLock::acquire(&lock), Ok(Some(_)));
    if free() {
        return true;
    }
    request_quit(app);
    let until = std::time::Instant::now() + std::time::Duration::from_millis(u64::from(wait_ms));
    while std::time::Instant::now() < until {
        if free() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// Waits for a message for this thread or one of `instance`'s requests, for up to
/// `timeout_ms` (`None`: as long as it takes, using no CPU meanwhile), then handles every
/// waiting message and returns the tray's events and the requests.
pub fn wait(timeout_ms: Option<u32>, instance: Option<&SingleInstance>) -> Vec<TrayEvent> {
    let handles: Vec<HANDLE> = instance
        .map(|i| vec![i.open.0, i.open_frozen.0, i.quit.0, i.reload.0])
        .unwrap_or_default();
    // SAFETY: the handles are live for the call; a message or an event ends the wait.
    let waited = unsafe {
        MsgWaitForMultipleObjectsEx(
            (!handles.is_empty()).then_some(handles.as_slice()),
            timeout_ms.unwrap_or(INFINITE),
            QS_ALLINPUT,
            MWMO_INPUTAVAILABLE,
        )
    };
    match waited.0.wrapping_sub(WAIT_OBJECT_0.0) {
        0 if !handles.is_empty() => push(TrayEvent::OpenRequested { frozen: false }),
        1 if handles.len() > 1 => push(TrayEvent::OpenRequested { frozen: true }),
        2 if handles.len() > 2 => push(TrayEvent::QuitRequested),
        3 if handles.len() > 3 => push(TrayEvent::ReloadRequested),
        _ => {}
    }
    let mut msg = MSG::default();
    // SAFETY: `msg` is a live MSG; each message is translated and dispatched on this thread.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                push(TrayEvent::Quit);
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    EVENTS.with(|e| e.borrow_mut().drain(..).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: &[u8] = include_bytes!("../../../../assets/tray/idle-on-light.ico");

    #[test]
    fn a_second_instance_asks_the_first_to_open() {
        let app = format!("OvermarkTest-{}-tray", std::process::id());
        let Instance::First(first) = SingleInstance::acquire(&app, false).unwrap() else {
            panic!("the first one")
        };
        for frozen in [false, true] {
            let second = std::thread::spawn({
                let app = app.clone();
                move || {
                    matches!(
                        SingleInstance::acquire(&app, frozen).unwrap(),
                        Instance::Second
                    )
                }
            });
            assert!(second.join().unwrap());
            let events = wait(Some(5000), Some(&first));
            assert_eq!(events, vec![TrayEvent::OpenRequested { frozen }]);
        }
        assert!(signal_reload(&app));
        assert_eq!(
            wait(Some(5000), Some(&first)),
            vec![TrayEvent::ReloadRequested]
        );
        assert!(request_quit(&app));
        assert_eq!(
            wait(Some(5000), Some(&first)),
            vec![TrayEvent::QuitRequested]
        );
        drop(first);
        assert!(!signal_reload(&app), "no instance, no reload");
        assert!(matches!(
            SingleInstance::acquire(&app, false).unwrap(),
            Instance::First(_)
        ));
    }

    #[test]
    fn fill_cuts_and_terminates() {
        let mut b = [7u16; 4];
        fill(&mut b, "abcdef");
        assert_eq!(b, [b'a' as u16, b'b' as u16, b'c' as u16, 0]);
    }

    #[test]
    fn reads_the_sizes_in_an_ico_file_and_picks_one() {
        let file = IconFile(IDLE);
        let sizes: Vec<u32> = file.images().iter().map(|i| i.0).collect();
        assert_eq!(sizes, [16, 20, 24, 32]);
        assert_eq!(file.image_for(16).unwrap().0, 16);
        assert_eq!(file.image_for(18).unwrap().0, 20);
        assert_eq!(file.image_for(48).unwrap().0, 32);
        assert!(IconFile(b"not an icon").image_for(16).is_none());
        for (size, _) in file.images() {
            OwnedIcon::from_file(file, size).unwrap();
        }
    }

    /// A waker from another thread reaches the loop with its code. (The tray's window alone:
    /// no icon is shown.)
    #[test]
    fn a_waker_wakes_the_loop_with_its_code() {
        let hwnd = hidden_window().unwrap();
        let waker = Waker(hwnd.0 as isize);
        std::thread::spawn(move || assert!(waker.wake(7)))
            .join()
            .unwrap();
        let mut events = Vec::new();
        for _ in 0..50 {
            events.extend(wait(Some(100), None));
            if events.contains(&TrayEvent::Woken(7)) {
                break;
            }
        }
        assert!(events.contains(&TrayEvent::Woken(7)), "{events:?}");
        // SAFETY: the test's own window, destroyed once.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}
