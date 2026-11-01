//! The small notice after a capture: where it went; a click opens the file, a
//! right-click its folder. A window of Overmark's own rather than a Windows toast, which needs
//! an AppUserModelID a portable copy hasn't got: it works the same in both modes. It's
//! excluded from capture (a share doesn't see it), never takes the focus, and goes after a
//! few seconds or a click.
//!
//! Its size comes from its text: each paragraph is measured with the fonts, width and wrapping
//! it's painted with (`DT_CALCRECT`), so a long path, an error's detail or a longer translation
//! is shown whole, wrapped (a path breaks inside a word too: `DT_EDITCONTROL`). Only text taller
//! than the monitor's work area is cut, at the work area.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, HWND, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DRAW_TEXT_FORMAT, DT_CALCRECT, DT_EDITCONTROL, DT_LEFT, DT_NOPREFIX,
    DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_SWISS, FW_NORMAL, FW_SEMIBOLD,
    FillRect, HDC, HFONT, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SW_HIDE, SetTimer, ShowWindow, WM_ERASEBKGND, WM_LBUTTONUP, WM_PAINT, WM_RBUTTONUP,
    WM_TIMER,
};
use windows::core::w;

use super::Rect;
use super::affinity;
use super::shell;
use super::window::{Options, Window};

/// How long it stays.
const SHOW_MS: u32 = 5000;
const TIMER_ID: usize = 1;
/// Its width at 100 %, and its padding and the gap between paragraphs.
const WIDTH: f32 = 440.0;
const PAD: f32 = 14.0;
const GAP: f32 = 4.0;
/// How it wraps, measuring and painting alike.
const FLAGS: DRAW_TEXT_FORMAT =
    DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_WORDBREAK.0 | DT_EDITCONTROL.0 | DT_NOPREFIX.0);

/// The title's font and the lines' (pixels at 100 %, semibold).
fn style(paragraph: usize) -> (f32, bool, COLORREF) {
    if paragraph == 0 {
        (15.0, true, COLORREF(0x00FF_FFFF))
    } else {
        (13.0, false, COLORREF(0x00D0_D0D0))
    }
}

/// Where each paragraph goes, in client pixels, and the window's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    /// One per paragraph: the title, then the lines.
    pub paragraphs: Vec<Rect>,
    /// Whether the text was taller than allowed, so the window was cut at `max_height`.
    pub cut: bool,
}

fn font(height: i32, bold: bool) -> HFONT {
    // SAFETY: a system font by name; the caller deletes it.
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            if bold {
                FW_SEMIBOLD.0 as i32
            } else {
                FW_NORMAL.0 as i32
            },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            Default::default(),
            CLEARTYPE_QUALITY,
            FF_SWISS.0 as u32,
            w!("Segoe UI"),
        )
    }
}

/// Measures or draws `text` in paragraph `i`'s font, in `rect` (`DT_CALCRECT` in `extra` to
/// measure: `rect` then gets the text's height). Returns the height.
fn text(
    dc: HDC,
    i: usize,
    scale: f32,
    text: &str,
    rect: &mut RECT,
    extra: DRAW_TEXT_FORMAT,
) -> i32 {
    let (size, bold, colour) = style(i);
    let f = font((size * scale).round() as i32, bold);
    // SAFETY: the font is ours, selected into `dc` and deleted after; `wide` outlives the call.
    unsafe {
        let old = SelectObject(dc, f.into());
        SetTextColor(dc, colour);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let h = DrawTextW(dc, &mut wide, rect, FLAGS | extra);
        SelectObject(dc, old);
        let _ = DeleteObject(f.into());
        h
    }
}

/// Lays out `paragraphs` (the title, then the lines) at `scale`: each measured as it's painted,
/// one under another, and the window as tall as they need, but no taller than `max_height`.
pub fn layout(paragraphs: &[String], scale: f32, max_height: i32) -> Layout {
    let width = (WIDTH * scale).round() as i32;
    let pad = (PAD * scale).round() as i32;
    let gap = (GAP * scale).round() as i32;
    let inner = width - 2 * pad;
    let mut rects = Vec::with_capacity(paragraphs.len());
    // SAFETY: a memory DC for measuring, deleted below.
    let dc = unsafe { CreateCompatibleDC(None) };
    let mut y = pad;
    for (i, p) in paragraphs.iter().enumerate() {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: inner,
            bottom: 0,
        };
        let h = text(dc, i, scale, p, &mut r, DT_CALCRECT).max(r.bottom);
        rects.push(Rect::new(pad, y, pad + inner, y + h));
        y += h + gap;
    }
    // SAFETY: our own memory DC.
    let _ = unsafe { DeleteDC(dc) };
    let needed = y - if paragraphs.is_empty() { 0 } else { gap } + pad;
    let max_height = max_height.max(2 * pad + 1);
    Layout {
        width,
        height: needed.min(max_height),
        paragraphs: rects,
        cut: needed > max_height,
    }
}

struct State {
    paragraphs: Vec<String>,
    layout: Layout,
    file: Option<PathBuf>,
    folder: Option<PathBuf>,
    scale: f32,
    /// The monitor's work area, for its place when its text changes.
    work: Rect,
}

/// Says `text` in the notice instead of what it said (a click's failure): laid out again,
/// kept at the bottom right of its work area, shown for its full time again, and no longer
/// opening anything.
fn retell(hwnd: HWND, s: &mut State, text: String) {
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos,
    };
    let margin = (16.0 * s.scale).round() as i32;
    s.paragraphs = vec![text];
    s.layout = layout(&s.paragraphs, s.scale, s.work.height() - 2 * margin);
    s.file = None;
    s.folder = None;
    let (w, h) = (s.layout.width, s.layout.height);
    // SAFETY: our own window; the timer goes with it.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            s.work.right - margin - w,
            s.work.bottom - margin - h,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
        SetTimer(Some(hwnd), TIMER_ID, SHOW_MS, None);
    }
}

fn paint(hwnd: HWND, s: &State) {
    // SAFETY: the standard paint cycle on our own window; every GDI object made is deleted.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
        let bg = CreateSolidBrush(COLORREF(0x0030_2B28));
        FillRect(dc, &rc, bg);
        let _ = DeleteObject(bg.into());
        SetBkMode(dc, TRANSPARENT);
        for (i, (p, r)) in s.paragraphs.iter().zip(&s.layout.paragraphs).enumerate() {
            let mut r = r.to_win32();
            text(dc, i, s.scale, p, &mut r, DRAW_TEXT_FORMAT(0));
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// A notice shown on a monitor until it times out, is clicked, or is dropped.
pub struct Toast {
    window: Window,
    layout: Layout,
}

impl Toast {
    /// Shows `title` and `lines` at the bottom right of `work` (a monitor's work area, physical
    /// pixels) at `scale`. A click opens `file`, a right-click shows `folder` (or the file in
    /// its folder).
    pub fn show(
        work: Rect,
        scale: f32,
        title: &str,
        lines: Vec<String>,
        file: Option<PathBuf>,
        folder: Option<PathBuf>,
    ) -> windows::core::Result<Toast> {
        let margin = (16.0 * scale).round() as i32;
        let paragraphs: Vec<String> = std::iter::once(title.to_owned()).chain(lines).collect();
        let layout = layout(&paragraphs, scale, work.height() - 2 * margin);
        let rect = Rect::new(
            work.right - margin - layout.width,
            work.bottom - margin - layout.height,
            work.right - margin,
            work.bottom - margin,
        );
        let state = Rc::new(RefCell::new(State {
            paragraphs,
            layout: layout.clone(),
            file,
            folder,
            scale,
            work,
        }));
        let s = state.clone();
        let hide = |hwnd: HWND| {
            // SAFETY: our own window.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        };
        let window = Window::new(
            rect,
            Options {
                no_activate: true,
                title: Some(super::window::title("window-notice-name")),
                ..Default::default()
            },
            Box::new(move |hwnd, msg, wp: WPARAM, _| match msg {
                WM_ERASEBKGND => Some(LRESULT(1)),
                WM_PAINT => {
                    paint(hwnd, &s.borrow());
                    Some(LRESULT(0))
                }
                WM_LBUTTONUP => {
                    let file = s.borrow().file.clone();
                    match file.map(|f| (shell::open_or_why(&f), f)) {
                        Some((Err(reason), f)) => {
                            tracing::warn!(file = %f.display(), %reason, "the capture couldn't be opened");
                            let text = crate::core::tr!(
                                "toast-open-failed",
                                file = f.display().to_string(),
                                reason = reason
                            );
                            retell(hwnd, &mut s.borrow_mut(), text);
                        }
                        _ => hide(hwnd),
                    }
                    Some(LRESULT(0))
                }
                WM_RBUTTONUP => {
                    let target = {
                        let st = s.borrow();
                        st.file.clone().or(st.folder.clone())
                    };
                    match target.map(|p| (shell::show_in_folder_or_why(&p), p)) {
                        Some((Err(reason), p)) => {
                            tracing::warn!(path = %p.display(), %reason, "the folder couldn't be opened");
                            let text = crate::core::tr!(
                                "error-open-folder",
                                path = p.display().to_string(),
                                detail = reason
                            );
                            retell(hwnd, &mut s.borrow_mut(), text);
                        }
                        _ => hide(hwnd),
                    }
                    Some(LRESULT(0))
                }
                WM_TIMER if wp.0 == TIMER_ID => {
                    hide(hwnd);
                    Some(LRESULT(0))
                }
                _ => None,
            }),
        )?;
        affinity::set_excluded(window.hwnd(), true)?;
        window.show(false);
        // SAFETY: our own window; the timer goes with it.
        unsafe {
            SetTimer(Some(window.hwnd()), TIMER_ID, SHOW_MS, None);
        }
        Ok(Toast { window, layout })
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    /// How its text was laid out.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Whether it's still showing.
    pub fn is_visible(&self) -> bool {
        // SAFETY: a query about our own window.
        unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.window.hwnd()) }
            .as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid 148-character path, which needs two lines and more.
    const LONG: &str = r"C:\Users\Sample\OneDrive - Example Corporation International\Documents\Customer presentations and technical reviews\Captures\2026-11-01 00.37.26.png";

    fn saved(path: &str) -> Vec<String> {
        vec![
            format!("Copied, and saved to {path}"),
            "Click to open it, right-click to open its folder".to_owned(),
        ]
    }

    /// Every paragraph inside the window, below the one before, within the padding.
    fn assert_fits(l: &Layout, scale: f32) {
        let pad = (PAD * scale).round() as i32;
        assert!(!l.cut);
        for (i, r) in l.paragraphs.iter().enumerate() {
            assert!(r.height() > 0, "{l:?}");
            assert!(
                r.bottom <= l.height - pad,
                "paragraph {i} runs past the bottom: {l:?}"
            );
            if i > 0 {
                assert!(r.top >= l.paragraphs[i - 1].bottom, "{l:?}");
            }
        }
    }

    #[test]
    fn a_long_path_wraps_and_the_notice_grows_to_fit_at_100_and_150() {
        for scale in [1.0, 1.5] {
            let short = layout(&saved(r"C:\P\a.png"), scale, 2000);
            let long = layout(&saved(LONG), scale, 2000);
            assert_fits(&short, scale);
            assert_fits(&long, scale);
            // The long title takes several lines; the window grows by as much.
            let line = short.paragraphs[0].height();
            assert!(long.paragraphs[0].height() >= 2 * line, "{long:?}");
            assert_eq!(
                long.height - short.height,
                long.paragraphs[0].height() - short.paragraphs[0].height()
            );
            // The width scales; the hint is the same size in both.
            assert_eq!(long.width, (WIDTH * scale).round() as i32);
            assert_eq!(long.paragraphs[1].height(), short.paragraphs[1].height());
        }
        // At 150 % everything is bigger than at 100 %.
        let (a, b) = (
            layout(&saved(LONG), 1.0, 2000),
            layout(&saved(LONG), 1.5, 2000),
        );
        assert!(b.height > a.height && b.width > a.width);
    }

    #[test]
    fn an_error_s_detail_and_every_line_fit() {
        let lines = vec![
            format!(
                "The capture couldn't be saved to {}: Access is denied. (os error 5)",
                LONG.rsplit_once('\\').unwrap().0
            ),
            "Without the drawing".to_owned(),
            "Click to open it, right-click to open its folder".to_owned(),
        ];
        for scale in [1.0, 1.25, 1.5, 2.0] {
            assert_fits(&layout(&lines, scale, 4000), scale);
        }
    }

    #[test]
    fn painting_uses_the_measured_rectangles() {
        // Drawing a paragraph into its measured rectangle takes the height measured for it.
        let paragraphs = saved(LONG);
        let l = layout(&paragraphs, 1.5, 2000);
        // SAFETY: a memory DC for the test, deleted below.
        let dc = unsafe { CreateCompatibleDC(None) };
        for (i, (p, r)) in paragraphs.iter().zip(&l.paragraphs).enumerate() {
            let mut rc = r.to_win32();
            let h = text(dc, i, 1.5, p, &mut rc, DRAW_TEXT_FORMAT(0));
            assert_eq!(h, r.height(), "paragraph {i}");
        }
        // SAFETY: our own memory DC.
        let _ = unsafe { DeleteDC(dc) };
    }

    #[test]
    fn text_taller_than_the_work_area_is_cut_there() {
        let many: Vec<String> = (0..80).map(|i| format!("line {i} {LONG}")).collect();
        let l = layout(&many, 1.0, 600);
        assert!(l.cut);
        assert_eq!(l.height, 600);
    }
}
