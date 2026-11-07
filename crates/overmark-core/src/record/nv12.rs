//! BGRA pixels to NV12, the encoder's input: a full-size plane of luma (Y), then a half-size
//! plane of interleaved chroma (U, V), one pair for each 2x2 block of pixels.
//!
//! BT.709 in limited ("TV") range, 16-235 for luma and 16-240 for chroma: what H.264 players
//! assume for HD video, and what the encoder's media type says (`overmark_win::win32::mf`).

/// The bytes an NV12 image `width` x `height` takes (both even).
pub const fn len(width: usize, height: usize) -> usize {
    width * height * 3 / 2
}

// BT.709's luma weights, and its chroma scales, in 16.16 fixed point, already scaled to the
// limited range (219/255 for luma, 224/255 for chroma).
const SHIFT: u32 = 16;
const HALF: i32 = 1 << (SHIFT - 1);
const YR: i32 = 11_966; // 0.2126 * 219/255 * 65536
const YG: i32 = 40_254; // 0.7152 * 219/255
const YB: i32 = 4_064; // 0.0722 * 219/255
const UR: i32 = -6_596; // -0.2126 / 1.8556 * 224/255
const UG: i32 = -22_189; // -0.7152 / 1.8556 * 224/255
const UB: i32 = 28_785; // 0.5 * 224/255
const VR: i32 = 28_785; // 0.5 * 224/255
const VG: i32 = -26_145; // -0.7152 / 1.5748 * 224/255
const VB: i32 = -2_640; // -0.0722 / 1.5748 * 224/255

fn luma(b: i32, g: i32, r: i32) -> u8 {
    (16 + ((YR * r + YG * g + YB * b + HALF) >> SHIFT)) as u8
}

/// Converts `bgra` (`width` x `height`, rows `width * 4` bytes, top row first) into `out`
/// ([`len`] bytes). `width` and `height` must be even; alpha is ignored.
pub fn from_bgra(bgra: &[u8], width: usize, height: usize, out: &mut [u8]) {
    assert!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "NV12 needs even sides"
    );
    assert!(bgra.len() >= width * height * 4, "too few pixels");
    assert!(out.len() >= len(width, height), "NV12 buffer too small");
    let (y_plane, uv_plane) = out.split_at_mut(width * height);
    let stride = width * 4;
    for row in 0..height / 2 {
        let top = &bgra[2 * row * stride..(2 * row + 1) * stride];
        let bottom = &bgra[(2 * row + 1) * stride..(2 * row + 2) * stride];
        let (y_top, y_rest) = y_plane[2 * row * width..].split_at_mut(width);
        let y_bottom = &mut y_rest[..width];
        let uv = &mut uv_plane[row * width..(row + 1) * width];
        for col in 0..width / 2 {
            let (mut sb, mut sg, mut sr) = (0i32, 0i32, 0i32);
            for (src, dst) in [(top, &mut *y_top), (bottom, &mut *y_bottom)] {
                for dx in 0..2 {
                    let x = 2 * col + dx;
                    let p = &src[x * 4..x * 4 + 3];
                    let (b, g, r) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
                    dst[x] = luma(b, g, r);
                    sb += b;
                    sg += g;
                    sr += r;
                }
            }
            // The block's mean colour, rounded, gives its chroma.
            let (b, g, r) = ((sb + 2) >> 2, (sg + 2) >> 2, (sr + 2) >> 2);
            uv[2 * col] = (128 + ((UR * r + UG * g + UB * b + HALF) >> SHIFT)) as u8;
            uv[2 * col + 1] = (128 + ((VR * r + VG * g + VB * b + HALF) >> SHIFT)) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(b: u8, g: u8, r: u8, w: usize, h: usize) -> Vec<u8> {
        [b, g, r, 255].repeat(w * h)
    }

    fn convert(bgra: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut out = vec![0; len(w, h)];
        from_bgra(bgra, w, h, &mut out);
        out
    }

    /// The YUV of one colour, from its first pixel and its first chroma pair.
    fn yuv(b: u8, g: u8, r: u8) -> (u8, u8, u8) {
        let out = convert(&solid(b, g, r, 2, 2), 2, 2);
        (out[0], out[4], out[5])
    }

    #[test]
    fn black_white_and_grey_have_neutral_chroma_in_limited_range() {
        assert_eq!(yuv(0, 0, 0), (16, 128, 128));
        assert_eq!(yuv(255, 255, 255), (235, 128, 128));
        let (y, u, v) = yuv(128, 128, 128);
        assert!((125..=127).contains(&y), "{y}");
        assert_eq!((u, v), (128, 128));
    }

    #[test]
    fn primaries_match_bt709() {
        // BT.709 limited range: red (63, 102, 240), green (173, 42, 26), blue (32, 240, 118),
        // each within a step of rounding.
        let near = |a: (u8, u8, u8), b: (u8, u8, u8)| {
            a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1 && a.2.abs_diff(b.2) <= 1
        };
        assert!(near(yuv(0, 0, 255), (63, 102, 240)), "{:?}", yuv(0, 0, 255));
        assert!(near(yuv(0, 255, 0), (173, 42, 26)), "{:?}", yuv(0, 255, 0));
        assert!(near(yuv(255, 0, 0), (32, 240, 118)), "{:?}", yuv(255, 0, 0));
    }

    #[test]
    fn planes_are_laid_out_as_nv12() {
        // 4x2: the left 2x2 block black, the right white.
        let mut bgra = Vec::new();
        for _ in 0..2 {
            bgra.extend(solid(0, 0, 0, 2, 1));
            bgra.extend(solid(255, 255, 255, 2, 1));
        }
        let out = convert(&bgra, 4, 2);
        assert_eq!(out.len(), 12);
        assert_eq!(&out[..8], &[16, 16, 235, 235, 16, 16, 235, 235]);
        assert_eq!(&out[8..], &[128, 128, 128, 128]);
    }

    #[test]
    fn chroma_is_the_blocks_mean() {
        // A 2x2 block half red, half blue: its chroma is that of their mean.
        let mut bgra = solid(0, 0, 255, 1, 1);
        bgra.extend(solid(255, 0, 0, 1, 1));
        bgra.extend(solid(0, 0, 255, 1, 1));
        bgra.extend(solid(255, 0, 0, 1, 1));
        let out = convert(&bgra, 2, 2);
        let mean = yuv(128, 0, 128);
        assert_eq!((out[4], out[5]), (mean.1, mean.2));
    }

    #[test]
    #[should_panic(expected = "even")]
    fn odd_sides_are_refused() {
        convert(&solid(0, 0, 0, 3, 2), 3, 2);
    }
}
