//! Drawing the mouse pointer into a recorded frame. Desktop Duplication's image has no pointer
//! in it: it gives the pointer's shape and position apart, in one of three forms, which this
//! draws as Windows does.

/// How a pointer shape's pixels are given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// 32-bit BGRA, blended over the screen by its alpha (straight, not premultiplied).
    Colour,
    /// 32-bit BGRA where alpha is a mask: 0 puts the colour on the screen, 0xFF XORs it with
    /// the screen (the text cursor's inverting parts).
    MaskedColour,
    /// 1 bit a pixel: an AND mask, then an XOR mask below it, each `height` rows.
    Monochrome,
}

/// A pointer shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub kind: ShapeKind,
    /// In pixels. For [`ShapeKind::Monochrome`], `height` is the pointer's own (half the
    /// rows given: the masks are stacked).
    pub width: u32,
    pub height: u32,
    /// The bytes from one row to the next.
    pub pitch: u32,
    pub data: Vec<u8>,
}

/// Draws `shape` with its top-left at (`x`, `y`) into `frame` (`width` x `height` BGRA,
/// tightly packed rows). Parts outside the frame are cut.
pub fn draw(shape: &Shape, x: i32, y: i32, frame: &mut [u8], width: i32, height: i32) {
    let pitch = shape.pitch as usize;
    let rows_needed = match shape.kind {
        ShapeKind::Monochrome => 2 * shape.height as usize,
        _ => shape.height as usize,
    };
    if shape.data.len() < pitch * rows_needed {
        return;
    }
    for sy in 0..shape.height as i32 {
        let fy = y + sy;
        if !(0..height).contains(&fy) {
            continue;
        }
        for sx in 0..shape.width as i32 {
            let fx = x + sx;
            if !(0..width).contains(&fx) {
                continue;
            }
            let d = ((fy * width + fx) * 4) as usize;
            let Some(dst) = frame.get_mut(d..d + 4) else {
                return;
            };
            let (sxu, syu) = (sx as usize, sy as usize);
            match shape.kind {
                ShapeKind::Colour => {
                    let s = &shape.data[syu * pitch + sxu * 4..][..4];
                    let a = u32::from(s[3]);
                    for c in 0..3 {
                        let blended = u32::from(s[c]) * a + u32::from(dst[c]) * (255 - a);
                        dst[c] = ((blended + 127) / 255) as u8;
                    }
                }
                ShapeKind::MaskedColour => {
                    let s = &shape.data[syu * pitch + sxu * 4..][..4];
                    if s[3] == 0 {
                        dst[..3].copy_from_slice(&s[..3]);
                    } else {
                        for c in 0..3 {
                            dst[c] ^= s[c];
                        }
                    }
                }
                ShapeKind::Monochrome => {
                    let bit = |row: usize| shape.data[row * pitch + sxu / 8] & (0x80 >> (sxu % 8));
                    let and = if bit(syu) != 0 { 0xFF } else { 0 };
                    let xor = if bit(syu + shape.height as usize) != 0 {
                        0xFF
                    } else {
                        0
                    };
                    for c in dst.iter_mut().take(3) {
                        *c = (*c & and) ^ xor;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: [u8; 4] = [100, 100, 100, 255];

    fn frame(w: i32, h: i32) -> Vec<u8> {
        GREY.repeat((w * h) as usize)
    }

    fn px(f: &[u8], w: i32, x: i32, y: i32) -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [f[i], f[i + 1], f[i + 2]]
    }

    #[test]
    fn colour_blends_by_alpha() {
        // 2x1: opaque red, then half-transparent blue.
        let shape = Shape {
            kind: ShapeKind::Colour,
            width: 2,
            height: 1,
            pitch: 8,
            data: vec![0, 0, 255, 255, 255, 0, 0, 128],
        };
        let mut f = frame(4, 4);
        draw(&shape, 1, 2, &mut f, 4, 4);
        assert_eq!(px(&f, 4, 1, 2), [0, 0, 255]);
        // 255 * 128/255 + 100 * 127/255 = 128 + 49.8
        assert_eq!(px(&f, 4, 2, 2), [178, 50, 50]);
        assert_eq!(px(&f, 4, 0, 2), [100, 100, 100]);
        assert_eq!(px(&f, 4, 1, 1), [100, 100, 100]);
    }

    #[test]
    fn masked_colour_replaces_or_xors() {
        let shape = Shape {
            kind: ShapeKind::MaskedColour,
            width: 2,
            height: 1,
            pitch: 8,
            data: vec![10, 20, 30, 0, 0xFF, 0xFF, 0xFF, 0xFF],
        };
        let mut f = frame(2, 1);
        draw(&shape, 0, 0, &mut f, 2, 1);
        assert_eq!(px(&f, 2, 0, 0), [10, 20, 30]);
        assert_eq!(px(&f, 2, 1, 0), [155, 155, 155]);
    }

    #[test]
    fn monochrome_and_then_xor() {
        // 4x1: AND 0b0011, XOR 0b0101: black, white, screen, inverted.
        let shape = Shape {
            kind: ShapeKind::Monochrome,
            width: 4,
            height: 1,
            pitch: 1,
            data: vec![0b0011_0000, 0b0101_0000],
        };
        let mut f = frame(4, 1);
        draw(&shape, 0, 0, &mut f, 4, 1);
        assert_eq!(px(&f, 4, 0, 0), [0, 0, 0]);
        assert_eq!(px(&f, 4, 1, 0), [255, 255, 255]);
        assert_eq!(px(&f, 4, 2, 0), [100, 100, 100]);
        assert_eq!(px(&f, 4, 3, 0), [155, 155, 155]);
    }

    #[test]
    fn parts_outside_the_frame_are_cut() {
        let shape = Shape {
            kind: ShapeKind::Colour,
            width: 3,
            height: 3,
            pitch: 12,
            data: [0, 0, 255, 255].repeat(9),
        };
        let mut f = frame(4, 4);
        // Hanging off the top-left: only (0, 0) is in the frame.
        draw(&shape, -2, -2, &mut f, 4, 4);
        assert_eq!(px(&f, 4, 0, 0), [0, 0, 255]);
        assert_eq!(px(&f, 4, 1, 0), [100, 100, 100]);
        assert_eq!(px(&f, 4, 0, 1), [100, 100, 100]);
        // Off the bottom-right: (3, 3) only.
        draw(&shape, 3, 3, &mut f, 4, 4);
        assert_eq!(px(&f, 4, 3, 3), [0, 0, 255]);
        // Wholly outside: nothing, and no panic.
        let before = f.clone();
        draw(&shape, 10, -10, &mut f, 4, 4);
        assert_eq!(f, before);
    }

    #[test]
    fn a_short_buffer_draws_nothing() {
        let shape = Shape {
            kind: ShapeKind::Monochrome,
            width: 8,
            height: 2,
            pitch: 1,
            data: vec![0; 3],
        };
        let mut f = frame(8, 2);
        let before = f.clone();
        draw(&shape, 0, 0, &mut f, 8, 2);
        assert_eq!(f, before);
    }
}
