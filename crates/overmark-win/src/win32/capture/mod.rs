//! Screen capture.
//!
//! - [`dd`]: DXGI Desktop Duplication, for saves and frozen frames. It never shows a capture
//!   border, on Windows 10 or 11 (docs/ARCHITECTURE.md, Capture API).
//! - [`wgc`]: Windows Graphics Capture, the API Teams, Zoom and the Snipping Tool use: here it
//!   is an observer that sees the screen as a call's audience does.
//! - [`gdi_screen`]: a GDI grab, for Frozen opens, when none of our windows is on screen yet.

pub mod dd;
pub mod wgc;

/// A captured image: rows of `width` BGRA pixels, top row first.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub bgra: Vec<u8>,
}

impl Frame {
    /// The pixel at (`x`, `y`) as `[b, g, r, a]`.
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.bgra[i],
            self.bgra[i + 1],
            self.bgra[i + 2],
            self.bgra[i + 3],
        ]
    }
}

/// How a monitor's scan-out image is turned relative to its desktop: Desktop Duplication
/// gives the image unturned (in the panel's own orientation), with this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    Rotate90,
    Rotate180,
    Rotate270,
}

/// The desktop's image from a duplicated (unturned) one, following Microsoft's
/// DesktopDuplication sample (its dirty-rectangle mapping): for `Rotate90` the desktop pixel
/// (x, y) is the texture's (y, W - 1 - x), W the desktop's width (the texture's height); for
/// `Rotate270` it is (H - 1 - y, x). Not yet checked on a turned monitor (docs/ARCHITECTURE.md).
pub fn unrotate(texture: &Frame, rotation: Rotation) -> Frame {
    let (tw, th) = (texture.width, texture.height);
    let (w, h) = match rotation {
        Rotation::None | Rotation::Rotate180 => (tw, th),
        Rotation::Rotate90 | Rotation::Rotate270 => (th, tw),
    };
    if rotation == Rotation::None {
        return texture.clone();
    }
    let mut bgra = vec![0u8; texture.bgra.len()];
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = match rotation {
                Rotation::None => (x, y),
                Rotation::Rotate90 => (y, w - 1 - x),
                Rotation::Rotate180 => (tw - 1 - x, th - 1 - y),
                Rotation::Rotate270 => (h - 1 - y, x),
            };
            let s = ((sy * tw + sx) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            bgra[d..d + 4].copy_from_slice(&texture.bgra[s..s + 4]);
        }
    }
    Frame {
        width: w,
        height: h,
        bgra,
    }
}

/// Whether two BGRA pixels match within `tolerance` per colour channel (alpha ignored).
pub fn close(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tolerance)
}

/// The screen in `rect` (physical pixels) through GDI (BitBlt of the screen's DC): no device,
/// no border, and with no window of ours on screen it is a clean grab. `None` if GDI failed.
pub fn gdi_screen(rect: super::Rect) -> Option<Frame> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
        SelectObject,
    };
    let (w, h) = (rect.width(), rect.height());
    let len = super::byte_len(w, h)?;
    // SAFETY: a DIB section of `len` bytes, filled by BitBlt from the screen's DC and copied
    // out; every GDI object is released before returning.
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let out = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(dc, bmp.into());
                let ok = BitBlt(
                    dc,
                    0,
                    0,
                    w,
                    h,
                    Some(screen),
                    rect.left,
                    rect.top,
                    SRCCOPY | CAPTUREBLT,
                )
                .is_ok();
                let frame = ok.then(|| {
                    let mut bgra = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
                    for p in bgra.as_chunks_mut::<4>().0 {
                        p[3] = 255;
                    }
                    Frame {
                        width: w,
                        height: h,
                        bgra,
                    }
                });
                SelectObject(dc, old);
                let _ = DeleteObject(bmp.into());
                frame
            }
            _ => None,
        };
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` x `h` frame whose pixel (x, y) is `[x, y, 0, 255]`.
    fn numbered(w: i32, h: i32) -> Frame {
        let mut bgra = Vec::new();
        for y in 0..h {
            for x in 0..w {
                bgra.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        Frame {
            width: w,
            height: h,
            bgra,
        }
    }

    #[test]
    fn unrotate_turns_the_texture_into_the_desktop() {
        // A 3x2 texture: a portrait monitor's 2x3 desktop for 90 and 270.
        let t = numbered(3, 2);
        let same = unrotate(&t, Rotation::None);
        assert_eq!((same.width, same.height, &same.bgra), (3, 2, &t.bgra));

        let r90 = unrotate(&t, Rotation::Rotate90);
        assert_eq!((r90.width, r90.height), (2, 3));
        // Desktop (x, y) = texture (y, W - 1 - x), W = 2.
        assert_eq!(r90.pixel(0, 0), t.pixel(0, 1));
        assert_eq!(r90.pixel(1, 0), t.pixel(0, 0));
        assert_eq!(r90.pixel(0, 2), t.pixel(2, 1));

        let r270 = unrotate(&t, Rotation::Rotate270);
        assert_eq!((r270.width, r270.height), (2, 3));
        // Desktop (x, y) = texture (H - 1 - y, x), H = 3.
        assert_eq!(r270.pixel(0, 0), t.pixel(2, 0));
        assert_eq!(r270.pixel(1, 2), t.pixel(0, 1));

        let r180 = unrotate(&t, Rotation::Rotate180);
        assert_eq!((r180.width, r180.height), (3, 2));
        assert_eq!(r180.pixel(0, 0), t.pixel(2, 1));
        assert_eq!(r180.pixel(2, 1), t.pixel(0, 0));

        // Each turn keeps every pixel exactly once.
        for r in [Rotation::Rotate90, Rotation::Rotate180, Rotation::Rotate270] {
            let mut a = unrotate(&t, r).bgra.as_chunks::<4>().0.to_vec();
            let mut b = t.bgra.as_chunks::<4>().0.to_vec();
            a.sort();
            b.sort();
            assert_eq!(a, b, "{r:?} lost or doubled a pixel");
        }
    }

    #[test]
    fn pixel_reads_bgra_rows() {
        let mut bgra = vec![0u8; 2 * 2 * 4];
        bgra[12..16].copy_from_slice(&[1, 2, 3, 4]);
        let f = Frame {
            width: 2,
            height: 2,
            bgra,
        };
        assert_eq!(f.pixel(1, 1), [1, 2, 3, 4]);
        assert!(close(f.pixel(1, 1), [2, 3, 4, 0], 1));
        assert!(!close(f.pixel(1, 1), [9, 3, 4, 4], 1));
    }
}
