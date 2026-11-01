//! A floating panel of Overmark's own: the toolbar's window.
//!
//! A layered window (per-pixel alpha through [`super::layered`]) that never takes the focus
//! (the keys stay with the ink layer), stays above the ink, and is **excluded from capture**
//! (`WDA_EXCLUDEFROMCAPTURE`), so a screen share or recording never shows it. Transparent
//! pixels let the pointer through to the ink layer beneath; the panel's own pixels take it, so
//! a click on the toolbar never draws. Where the caller says what's solid
//! ([`Panel::set_solid`]), only that takes it: elsewhere (a tooltip, a shadow's faint edge) the
//! panel answers `WM_NCHITTEST` with `HTTRANSPARENT`, and the pointer goes on to the ink layer
//! beneath, a window of the same thread.
//!
//! Its pixels come from the caller (the toolbar's egui frame, rendered on the CPU); its pointer
//! samples go back as [`PanelEvent`]s in client pixels. **Screen readers** see it through
//! AccessKit's own Windows adapter, which answers `WM_GETOBJECT` with the accessibility tree the
//! caller gives ([`Panel::set_accessibility`]); an action a screen reader asks for (Narrator's
//! "press", UI Automation's Invoke) comes back as [`PanelEvent::Action`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{
    HTTRANSPARENT, MA_NOACTIVATE, WM_APP, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_POINTERDOWN,
    WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE,
};

use super::layered::LayeredSurface;
use super::window::{self, Options, Window};
use super::{Rect, affinity, pointer};

/// Posted to the panel when a screen reader's action is waiting (it may come on another
/// thread): it wakes the app's loop, which then takes it with [`Panel::take_events`].
const WM_PANEL_ACTION: u32 = WM_APP + 0x50;
/// Posted to a panel by checks: a pointer sample at client pixel (`lparam`'s low and high
/// words, signed) with `wparam` 1 while it touches. The same path as `WM_POINTER*` once read:
/// how checks drive the toolbar without global input.
pub const WM_PANEL_POINTER: u32 = WM_APP + 0x51;

/// The `wparam` and `lparam` of a [`WM_PANEL_POINTER`].
pub fn panel_pointer(x: i32, y: i32, down: bool) -> (usize, isize) {
    (
        usize::from(down),
        ((y as i16 as u16 as isize) << 16) | (x as i16 as u16 as isize),
    )
}

/// What a panel answers the system's hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Its own: it takes the pointer.
    Panel,
    /// Transparent to it: the pointer goes on to the window beneath (of the same thread).
    Through,
    /// Anything else (the raw `WM_NCHITTEST` result).
    Other(isize),
}

/// What panel `hwnd` (in this process or another) answers the system's hit test at screen
/// pixel (`x`, `y`), asked of it within a second; `None` if it didn't answer. Checks ask this
/// of the toolbar's window: from another thread `WindowFromPoint` doesn't ask a window, and
/// takes it as opaque wherever its pixels are drawn.
pub fn hit_test(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        HTCLIENT, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
    };
    let (_, lp) = panel_pointer(x, y, false);
    let mut result = 0usize;
    // SAFETY: a message of plain numbers; `result` is ours for the call.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_NCHITTEST,
            WPARAM(0),
            LPARAM(lp),
            SMTO_ABORTIFHUNG,
            1000,
            Some(&mut result),
        )
    };
    (sent.0 != 0).then_some(match result as isize {
        r if r == HTCLIENT as isize => Hit::Panel,
        r if r == HTTRANSPARENT as isize => Hit::Through,
        r => Hit::Other(r),
    })
}

/// What happened on the panel.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelEvent {
    /// The pointer moved, in client pixels (and `screen` pixels, from where the window was
    /// then); `down` while the primary button (or a pen or a finger) touches.
    Pointer {
        x: i32,
        y: i32,
        screen: (i32, i32),
        down: bool,
    },
    /// The pointer left the panel.
    Left,
    /// A screen reader (or UI Automation) asked for an action.
    Action(ActionRequest),
}

struct Actions {
    hwnd: isize,
    queue: Arc<Mutex<Vec<ActionRequest>>>,
}

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Ok(mut q) = self.queue.lock() {
            q.push(request);
        }
        window::post(HWND(self.hwnd as *mut _), WM_PANEL_ACTION, 0, 0);
    }
}

struct Activation {
    tree: Rc<RefCell<Option<TreeUpdate>>>,
}

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.tree.borrow().clone()
    }
}

/// The panel. Dropping it closes it.
pub struct Panel {
    // The adapter goes before the window (it unhooks itself from the window's procedure).
    adapter: Option<accesskit_windows::SubclassingAdapter>,
    window: Window,
    surface: LayeredSurface,
    rect: Rect,
    events: Rc<RefCell<Vec<PanelEvent>>>,
    actions: Arc<Mutex<Vec<ActionRequest>>>,
    tree: Rc<RefCell<Option<TreeUpdate>>>,
    solid: Rc<RefCell<Option<Vec<Rect>>>>,
}

/// Whether the pointer at client pixel (`x`, `y`) is the panel's: anywhere until it's told
/// what's solid, then only there.
fn takes_pointer(solid: Option<&[Rect]>, x: i32, y: i32) -> bool {
    solid.is_none_or(|rects| {
        rects
            .iter()
            .any(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    })
}

impl Panel {
    /// A panel over `rect` (screen pixels), titled `title`, not shown yet: transparent until
    /// its pixels are given.
    pub fn new(rect: Rect, title: &'static str) -> windows::core::Result<Panel> {
        let events: Rc<RefCell<Vec<PanelEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let origin = Rc::new(std::cell::Cell::new((rect.left, rect.top)));
        let handler_events = events.clone();
        let handler_origin = origin.clone();
        let solid: Rc<RefCell<Option<Vec<Rect>>>> = Rc::new(RefCell::new(None));
        let handler_solid = solid.clone();
        let window = Window::new(
            rect,
            Options {
                layered: true,
                no_activate: true,
                title: Some(title),
                ..Default::default()
            },
            Box::new(move |hwnd, msg, wp, lp| match msg {
                WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                    let (ox, oy) = window_origin(hwnd).unwrap_or(handler_origin.get());
                    let mut e = handler_events.borrow_mut();
                    for s in pointer::samples(wp) {
                        e.push(PanelEvent::Pointer {
                            x: s.x - ox,
                            y: s.y - oy,
                            screen: (s.x, s.y),
                            down: s.in_contact && msg != WM_POINTERUP,
                        });
                    }
                    Some(LRESULT(0))
                }
                WM_PANEL_POINTER => {
                    let (ox, oy) = window_origin(hwnd).unwrap_or(handler_origin.get());
                    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
                    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                    handler_events.borrow_mut().push(PanelEvent::Pointer {
                        x,
                        y,
                        screen: (ox + x, oy + y),
                        down: wp.0 & 1 != 0,
                    });
                    Some(LRESULT(0))
                }
                // Only what's solid takes the pointer; elsewhere it goes on to the window
                // beneath (the ink layer).
                WM_NCHITTEST => {
                    let (ox, oy) = window_origin(hwnd).unwrap_or(handler_origin.get());
                    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32 - ox;
                    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32 - oy;
                    let solid = handler_solid.borrow();
                    (!takes_pointer(solid.as_deref(), x, y))
                        .then_some(LRESULT(HTTRANSPARENT as isize))
                }
                WM_POINTERLEAVE => {
                    handler_events.borrow_mut().push(PanelEvent::Left);
                    None
                }
                // A click never brings it forward or takes the focus from the ink layer.
                WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
                WM_PANEL_ACTION => Some(LRESULT(0)),
                _ => None,
            }),
        )?;
        affinity::set_excluded(window.hwnd(), true)?;
        let surface = LayeredSurface::new(rect.width(), rect.height())?;
        let actions = Arc::new(Mutex::new(Vec::new()));
        let tree = Rc::new(RefCell::new(None));
        // Before the window is first shown, as the adapter requires.
        let adapter = accesskit_windows::SubclassingAdapter::new(
            window.hwnd(),
            Activation { tree: tree.clone() },
            Actions {
                hwnd: window.hwnd().0 as isize,
                queue: actions.clone(),
            },
        );
        let mut panel = Panel {
            adapter: Some(adapter),
            window,
            surface,
            rect,
            events,
            actions,
            tree,
            solid,
        };
        panel.surface.pixels().fill(0);
        Ok(panel)
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    /// Where it is, in screen pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Shows it over everything of ours (the ink layer included), without taking the focus;
    /// again to bring it back on top (after the ink layer took the focus).
    pub fn show(&self) {
        self.window.show(false);
    }

    /// Its window's title, which screen readers read (the toolbar's, from Fluent).
    pub fn set_title(&self, title: &str) {
        use windows::Win32::UI::WindowsAndMessaging::SetWindowTextW;
        // SAFETY: our own window; the string lives for the call.
        let _ = unsafe { SetWindowTextW(self.window.hwnd(), &windows::core::HSTRING::from(title)) };
    }

    /// Moves its top-left corner to (`x`, `y`), screen pixels; its size stays.
    pub fn move_to(&mut self, x: i32, y: i32) {
        let (w, h) = (self.rect.width(), self.rect.height());
        self.rect = Rect::new(x, y, x + w, y + h);
        if let Err(error) = self.surface.present(self.window.hwnd(), (x, y), None) {
            tracing::warn!(%error, "couldn't move the panel");
        }
    }

    /// Its pixels: BGRA, premultiplied, rows top to bottom, its width by its height.
    pub fn pixels(&mut self) -> &mut [u8] {
        self.surface.pixels()
    }

    /// Shows its pixels (in `dirty` only, client pixels, if given).
    pub fn present(&mut self, dirty: Option<Rect>) {
        if let Err(error) =
            self.surface
                .present(self.window.hwnd(), (self.rect.left, self.rect.top), dirty)
        {
            tracing::warn!(%error, "couldn't show the panel");
        }
    }

    /// What takes the pointer, in client pixels: the toolbar's frames, not its tooltips or
    /// shadows. Elsewhere it goes through to the window beneath, whatever is drawn there.
    pub fn set_solid(&mut self, rects: Vec<Rect>) {
        *self.solid.borrow_mut() = Some(rects);
    }

    /// What screen readers see: the whole tree, as egui gives it each frame.
    pub fn set_accessibility(&mut self, tree: TreeUpdate) {
        *self.tree.borrow_mut() = Some(tree.clone());
        if let Some(adapter) = self.adapter.as_mut()
            && let Some(events) = adapter.update_if_active(|| tree)
        {
            events.raise();
        }
    }

    /// The pointer samples and screen readers' actions since the last call, in order (the
    /// actions after the samples).
    pub fn take_events(&mut self) -> Vec<PanelEvent> {
        let mut out = std::mem::take(&mut *self.events.borrow_mut());
        if let Ok(mut q) = self.actions.lock() {
            out.extend(q.drain(..).map(PanelEvent::Action));
        }
        out
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.adapter = None;
    }
}

fn window_origin(hwnd: HWND) -> Option<(i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = windows::Win32::Foundation::RECT::default();
    // SAFETY: our own window; writes one RECT we own.
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some((r.left, r.top))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the panel answers the system's hit test at screen pixel (`x`, `y`).
    fn hit(panel: &Panel, x: i32, y: i32) -> Option<Hit> {
        hit_test(panel.hwnd(), x, y)
    }

    /// Only what's solid takes the pointer; elsewhere (a tooltip, a shadow) the panel
    /// is transparent to it, and it goes on to the ink layer beneath.
    #[test]
    fn only_its_solid_rectangles_take_the_pointer() {
        let mut panel =
            Panel::new(Rect::new(300, 200, 700, 400), "Overmark test panel").expect("a panel");
        // Until it's told, all of it (its pixels' alpha decides, as before).
        assert_eq!(hit(&panel, 310, 210), Some(Hit::Panel));
        panel.set_solid(vec![Rect::new(10, 10, 110, 50), Rect::new(10, 60, 60, 100)]);
        for (x, y) in [(310, 210), (409, 249), (310, 260), (359, 299)] {
            assert_eq!(hit(&panel, x, y), Some(Hit::Panel), "({x}, {y}) is solid");
        }
        for (x, y) in [
            (309, 210),
            (410, 249),
            (310, 255),
            (360, 299),
            (310, 300),
            (650, 350),
        ] {
            assert_eq!(
                hit(&panel, x, y),
                Some(Hit::Through),
                "({x}, {y}) goes through"
            );
        }
        // Moved, it follows.
        panel.move_to(0, 0);
        assert_eq!(hit(&panel, 10, 10), Some(Hit::Panel));
        assert_eq!(hit(&panel, 310, 210), Some(Hit::Through));
    }

    /// And the system routes it so: over a window of the same thread (as the ink layer is),
    /// the panel is the window under its solid part, and elsewhere, its pixels drawn and all,
    /// the window beneath is. (`WindowFromPoint` asks only windows of the calling thread for
    /// their hit test, so this is checked here, on the thread that owns them both.)
    #[test]
    fn the_pointer_goes_through_to_the_window_beneath_outside_what_is_solid() {
        let r = Rect::new(40, 40, 120, 120);
        let under = Window::new(
            r,
            Options {
                no_activate: true,
                title: Some("Overmark test window beneath"),
                ..Default::default()
            },
            Box::new(|_, _, _, _| None),
        )
        .expect("a window");
        under.show(false);
        let mut panel = Panel::new(r, "Overmark test panel").expect("a panel");
        // Opaque all over: by alpha alone it would take the pointer everywhere.
        panel.pixels().fill(255);
        panel.present(None);
        panel.show();
        panel.set_solid(vec![Rect::new(0, 0, 30, 30)]);
        assert_eq!(window::window_at(50, 50), panel.hwnd(), "its solid part");
        assert_eq!(
            window::window_at(100, 100),
            under.hwnd(),
            "beneath, elsewhere"
        );
    }
}
