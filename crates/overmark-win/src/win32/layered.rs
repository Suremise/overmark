//! An ink surface presented with `UpdateLayeredWindowIndirect`: a 32-bit premultiplied DIB
//! the renderer draws into, handed to DWM as the window's per-pixel-alpha image. Only the dirty
//! rectangle is copied on each present.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush,
    HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ULW_ALPHA, UPDATELAYEREDWINDOWINFO, UpdateLayeredWindowIndirect,
};

use super::Rect;

/// A `width` × `height` premultiplied BGRA image, presented to a layered window.
pub struct LayeredSurface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    /// `width * height * 4`, checked in `new`.
    len: usize,
    width: i32,
    height: i32,
    presented_once: bool,
}

impl LayeredSurface {
    /// Fails if a side isn't positive or the image is too big (see [`super::byte_len`]).
    pub fn new(width: i32, height: i32) -> windows::core::Result<LayeredSurface> {
        let len = super::checked_len(width, height)?;
        // SAFETY: a top-down 32-bit DIB section selected into a memory DC we own; `bits`
        // points at its width × height × 4 bytes until the DIB is deleted in Drop.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(b) if !bits.is_null() => b,
                Ok(b) => {
                    let _ = DeleteObject(b.into());
                    let _ = DeleteDC(dc);
                    return Err(windows::core::Error::from_thread());
                }
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let old = SelectObject(dc, bitmap.into());
            Ok(LayeredSurface {
                dc,
                bitmap,
                old,
                bits: bits as *mut u8,
                len,
                width,
                height,
                presented_once: false,
            })
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// The pixels: rows of `width` premultiplied BGRA pixels, top row first.
    pub fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: the DIB's memory, `len` bytes (a top-down DIB of positive width and height, as
        // `new` checked), alive as long as `self`, borrowed mutably through it.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) }
    }

    /// The pixels, read-only (as [`LayeredSurface::pixels`]).
    pub fn pixels_ref(&self) -> &[u8] {
        // SAFETY: as `pixels`, borrowed shared through `&self`.
        unsafe { std::slice::from_raw_parts(self.bits, self.len) }
    }

    /// Hands the image to `hwnd` at `origin` (screen coordinates). `dirty` is the part that
    /// changed since the last present; `None` sends the whole image (the first present always
    /// does).
    pub fn present(
        &mut self,
        hwnd: HWND,
        origin: (i32, i32),
        dirty: Option<Rect>,
    ) -> windows::core::Result<()> {
        let dst = POINT {
            x: origin.0,
            y: origin.1,
        };
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let dirty_rect: Option<RECT> = match dirty {
            Some(r) if self.presented_once => {
                let r = r.clamp_to(self.width, self.height);
                if r.is_empty() {
                    return Ok(());
                }
                Some(r.to_win32())
            }
            _ => None,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &dst,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &src,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty_rect
                .as_ref()
                .map_or(std::ptr::null(), |r| r as *const RECT),
        };
        // SAFETY: every pointer in `info` points at a local that outlives the call; GdiFlush
        // makes sure GDI sees the renderer's writes to the DIB.
        unsafe {
            let _ = GdiFlush();
            UpdateLayeredWindowIndirect(hwnd, &info).ok()?;
        }
        self.presented_once = true;
        Ok(())
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        // SAFETY: undoes `new`, in reverse.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LayeredSurface;

    #[test]
    fn rejects_sizes_that_arent_positive_or_fit() {
        for (w, h) in [(1, -1), (-1, 1), (0, 1), (1, 0), (i32::MAX, i32::MAX)] {
            assert!(LayeredSurface::new(w, h).is_err(), "{w}x{h} was accepted");
        }
    }

    #[test]
    fn pixels_cover_the_image() {
        let mut s = LayeredSurface::new(3, 2).expect("a 3x2 surface");
        let px = s.pixels();
        assert_eq!(px.len(), 3 * 2 * 4);
        px.fill(7);
        assert!(s.pixels().iter().all(|&b| b == 7));
    }
}
