//! The Win32, DirectX and WinRT calls, behind safe wrappers. The only place in Overmark that
//! uses `unsafe`: each block calls a documented Windows API with arguments it owns.
//!
//! Built by the ink-layer spike (docs/ARCHITECTURE.md "The ink layer"):
//! - [`dpi`]: per-monitor DPI awareness v2 and the monitors in physical pixels;
//! - [`window`]: top-most popup windows with a handler for their messages;
//! - [`layered`]: an ink surface presented with `UpdateLayeredWindowIndirect`;
//! - [`dcomp`]: an ink surface presented through DirectComposition and a D3D11 swap chain;
//! - [`affinity`]: keeping a window out of screen capture;
//! - [`capture`]: DXGI Desktop Duplication (saves, frozen frames) and Windows Graphics Capture
//!   (an observer, as a Teams or Zoom share sees the screen);
//! - [`magnifier`]: a clean grab without our windows, which never leave the screen;
//! - [`mf`]: Media Foundation, loaded at run time: H.264 into an MP4, and reading one back;
//! - [`pointer`]: mouse, pen and touch through `WM_POINTER*` (no hooks);
//! - [`tray`]: the notification-area icon, its menu and balloons, and one instance per session;
//! - [`registry`]: values under `HKEY_CURRENT_USER` (the taskbar's theme, Start with Windows);
//! - [`shared_text`]: a little text one process publishes for another (the hotkeys' state);
//! - [`language`]: the Windows UI language;
//! - [`clock`]: the performance counter, in the units capture timestamps use.
#![allow(unsafe_code)]

pub mod affinity;
pub mod capture;
pub mod clock;
pub mod d3d;
pub mod dcomp;
pub mod dpi;
pub mod hotkey_probe;
pub mod http;
pub mod known_folders;
pub mod language;
pub mod layered;
pub mod magnifier;
pub mod mf;
pub mod panel;
pub mod pointer;
pub mod process;
pub mod registry;
pub mod shared_text;
pub mod shell;
pub mod toast;
pub mod tray;
pub mod uia;
pub mod window;

/// The bytes a `width` × `height` image of 4-byte pixels takes, or `None` if a side isn't
/// positive or the size overflows (an image must also fit in a Rust slice: `isize::MAX` bytes).
pub fn byte_len(width: i32, height: i32) -> Option<usize> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let n = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    (n <= isize::MAX as usize).then_some(n)
}

/// `byte_len` as an error, for constructors.
pub(crate) fn checked_len(width: i32, height: i32) -> windows::core::Result<usize> {
    byte_len(width, height).ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_INVALIDARG,
            format!("an image can't be {width}x{height}"),
        )
    })
}

/// A rectangle in physical pixels: `left`/`top` inclusive, `right`/`bottom` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// The smallest rectangle holding both; an empty one adds nothing.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// Where both overlap (empty if they don't).
    pub fn intersect(&self, other: &Rect) -> Rect {
        let r = Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        if r.is_empty() { Rect::default() } else { r }
    }

    /// This rectangle cut to `0..width` and `0..height`.
    pub fn clamp_to(&self, width: i32, height: i32) -> Rect {
        let r = Rect::new(
            self.left.clamp(0, width),
            self.top.clamp(0, height),
            self.right.clamp(0, width),
            self.bottom.clamp(0, height),
        );
        if r.is_empty() { Rect::default() } else { r }
    }

    pub(crate) fn to_win32(self) -> windows::Win32::Foundation::RECT {
        windows::Win32::Foundation::RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }

    pub(crate) fn from_win32(r: windows::Win32::Foundation::RECT) -> Rect {
        Rect::new(r.left, r.top, r.right, r.bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::{Rect, byte_len};

    #[test]
    fn byte_len_rejects_bad_sizes() {
        assert_eq!(byte_len(2, 3), Some(24));
        assert_eq!(byte_len(3840, 2160), Some(3840 * 2160 * 4));
        assert_eq!(byte_len(0, 5), None);
        assert_eq!(byte_len(5, 0), None);
        assert_eq!(byte_len(1, -1), None);
        assert_eq!(byte_len(-1, 1), None);
        assert_eq!(byte_len(i32::MIN, i32::MIN), None);
        // 2^31 x 2^31 x 4 overflows isize on 64-bit and usize on 32-bit.
        assert_eq!(byte_len(i32::MAX, i32::MAX), None);
    }

    #[test]
    fn union_skips_empty_rects() {
        let a = Rect::new(10, 10, 20, 20);
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(Rect::default().union(&a), a);
        assert_eq!(a.union(&Rect::new(0, 15, 12, 30)), Rect::new(0, 10, 20, 30));
    }

    #[test]
    fn clamp_cuts_to_the_surface() {
        assert_eq!(
            Rect::new(-5, -5, 10, 10).clamp_to(8, 20),
            Rect::new(0, 0, 8, 10)
        );
        assert!(Rect::new(30, 30, 40, 40).clamp_to(8, 8).is_empty());
    }
}
