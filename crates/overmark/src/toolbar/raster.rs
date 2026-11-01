//! Draws egui's frame on the CPU: the triangles epaint tessellates, into a window's BGRA
//! pixels (premultiplied), with the textures egui gives (its font atlas, mostly).
//!
//! Why not a GPU renderer: a GL or D3D device costs 85-280 ms to make and 50 MB or more to keep,
//! more than the whole opening's budget; a toolbar's few thousand triangles
//! take about a millisecond here. epaint already anti-aliases with feathered edges, so each
//! triangle is filled plainly: a pixel is in it when its centre is, with the top-left rule, so a
//! pixel on the edge two triangles share is drawn once (no seam where a translucent shape's
//! triangles meet). Blending is premultiplied source-over in sRGB space, as egui's own
//! renderers do.

use std::collections::HashMap;

use eframe::egui::{self, ClippedPrimitive, TextureId, TexturesDelta, epaint};

/// A texture: premultiplied sRGBA texels, row by row.
struct Texture {
    width: usize,
    height: usize,
    texels: Vec<[u8; 4]>,
}

/// The textures egui has given, by id.
#[derive(Default)]
pub struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    /// Applies a frame's new and changed textures (before painting it).
    pub fn set(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                let egui::ImageData::Color(image) = &d.image;
                let [w, h] = image.size;
                let texels: Vec<[u8; 4]> = image.pixels.iter().map(|c| c.to_array()).collect();
                match d.pos {
                    None => {
                        self.map.insert(
                            *id,
                            Texture {
                                width: w,
                                height: h,
                                texels,
                            },
                        );
                    }
                    Some([x0, y0]) => {
                        let Some(t) = self.map.get_mut(id) else {
                            continue;
                        };
                        for y in 0..h {
                            for x in 0..w {
                                let (tx, ty) = (x0 + x, y0 + y);
                                if tx < t.width && ty < t.height {
                                    t.texels[ty * t.width + tx] = texels[y * w + x];
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Frees what a frame no longer needs (after painting it).
    pub fn free(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.map.remove(id);
        }
    }

    /// The texel at (`u`, `v`) (0 to 1), bilinear; white if there's no such texture.
    fn sample(&self, id: TextureId, u: f32, v: f32) -> [f32; 4] {
        let Some(t) = self.map.get(&id) else {
            return [255.0; 4];
        };
        if t.width == 0 || t.height == 0 {
            return [255.0; 4];
        }
        let x = (u * t.width as f32 - 0.5).clamp(0.0, (t.width - 1) as f32);
        let y = (v * t.height as f32 - 0.5).clamp(0.0, (t.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(t.width - 1), (y0 + 1).min(t.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| t.texels[y * t.width + x];
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0; 4];
        for (i, o) in out.iter_mut().enumerate() {
            let top = f32::from(a[i]) * (1.0 - fx) + f32::from(b[i]) * fx;
            let bottom = f32::from(c[i]) * (1.0 - fx) + f32::from(d[i]) * fx;
            *o = top * (1.0 - fy) + bottom * fy;
        }
        out
    }
}

/// A target: BGRA, premultiplied, `width` × `height` pixels.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub width: usize,
    pub height: usize,
}

/// Paints `primitives` (points, at `pixels_per_point`) over `target`, each clipped to its rect.
pub fn paint(
    target: &mut Target,
    pixels_per_point: f32,
    primitives: &[ClippedPrimitive],
    textures: &Textures,
) {
    for p in primitives {
        let epaint::Primitive::Mesh(mesh) = &p.primitive else {
            continue;
        };
        let clip = (
            (p.clip_rect.min.x * pixels_per_point).round().max(0.0) as i64,
            (p.clip_rect.min.y * pixels_per_point).round().max(0.0) as i64,
            ((p.clip_rect.max.x * pixels_per_point).round() as i64).min(target.width as i64),
            ((p.clip_rect.max.y * pixels_per_point).round() as i64).min(target.height as i64),
        );
        if clip.2 <= clip.0 || clip.3 <= clip.1 {
            continue;
        }
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            triangle(target, pixels_per_point, clip, v, mesh.texture_id, textures);
        }
    }
}

/// Whether the edge from `a` to `b` is a top or left edge of a triangle wound so that the
/// inside has positive edge functions.
fn top_left(a: (f32, f32), b: (f32, f32)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dy == 0.0 && dx < 0.0) || dy > 0.0
}

fn triangle(
    target: &mut Target,
    ppp: f32,
    clip: (i64, i64, i64, i64),
    v: [&epaint::Vertex; 3],
    texture: TextureId,
    textures: &Textures,
) {
    let mut p = v.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
    let mut v = v;
    let edge = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let mut area = edge(p[0], p[1], p[2]);
    if area == 0.0 {
        return;
    }
    if area < 0.0 {
        p.swap(1, 2);
        v.swap(1, 2);
        area = -area;
    }
    let x0 = (p[0].0.min(p[1].0).min(p[2].0).floor() as i64).max(clip.0);
    let y0 = (p[0].1.min(p[1].1).min(p[2].1).floor() as i64).max(clip.1);
    let x1 = (p[0].0.max(p[1].0).max(p[2].0).ceil() as i64).min(clip.2);
    let y1 = (p[0].1.max(p[1].1).max(p[2].1).ceil() as i64).min(clip.3);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let bias = [
        top_left(p[1], p[2]),
        top_left(p[2], p[0]),
        top_left(p[0], p[1]),
    ];
    let colours = v.map(|v| v.color.to_array().map(f32::from));
    let solid = colours[0] == colours[1] && colours[1] == colours[2];
    // A shape's fill (not text) samples one texel, the white one, at all three corners: one
    // sample for the triangle, not a bilinear one per pixel.
    let flat = (v[0].uv == v[1].uv && v[1].uv == v[2].uv)
        .then(|| textures.sample(texture, v[0].uv.x, v[0].uv.y));
    let width = target.width;
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let w = [
                edge(p[1], p[2], (px, py)),
                edge(p[2], p[0], (px, py)),
                edge(p[0], p[1], (px, py)),
            ];
            let inside = w
                .iter()
                .zip(bias)
                .all(|(&w, top_left)| w > 0.0 || (w == 0.0 && top_left));
            if !inside {
                continue;
            }
            let b = w.map(|w| w / area);
            let colour = if solid {
                colours[0]
            } else {
                let mut c = [0.0; 4];
                for (i, ci) in c.iter_mut().enumerate() {
                    *ci = colours[0][i] * b[0] + colours[1][i] * b[1] + colours[2][i] * b[2];
                }
                c
            };
            let texel = flat.unwrap_or_else(|| {
                let u = v[0].uv.x * b[0] + v[1].uv.x * b[1] + v[2].uv.x * b[2];
                let t = v[0].uv.y * b[0] + v[1].uv.y * b[1] + v[2].uv.y * b[2];
                textures.sample(texture, u, t)
            });
            // Premultiplied colour times premultiplied texel, then source-over.
            let src: [f32; 4] = std::array::from_fn(|i| colour[i] * texel[i] / 255.0);
            if src[3] <= 0.0 && src[0] <= 0.0 && src[1] <= 0.0 && src[2] <= 0.0 {
                continue;
            }
            let i = (y as usize * width + x as usize) * 4;
            let dst = &mut target.pixels[i..i + 4];
            let keep = 1.0 - src[3] / 255.0;
            // The target is BGRA; egui's colours are RGBA.
            let order = [2, 1, 0, 3];
            for (k, &ch) in order.iter().enumerate() {
                let d = f32::from(dst[k]);
                dst[k] = (src[ch] + d * keep).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Pos2, Rect, epaint::Mesh};

    fn primitive(mesh: Mesh) -> ClippedPrimitive {
        ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: epaint::Primitive::Mesh(mesh),
        }
    }

    fn quad(rect: Rect, colour: Color32) -> Mesh {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(rect, colour);
        mesh
    }

    #[test]
    fn a_translucent_quad_is_blended_once_everywhere_its_diagonal_too() {
        let (w, h) = (16usize, 12usize);
        let mut px = vec![0u8; w * h * 4];
        // Half-transparent red, premultiplied.
        let red = Color32::from_rgba_premultiplied(128, 0, 0, 128);
        let mesh = quad(
            Rect::from_min_max(Pos2::new(2.0, 2.0), Pos2::new(12.0, 10.0)),
            red,
        );
        let mut target = Target {
            pixels: &mut px,
            width: w,
            height: h,
        };
        paint(&mut target, 1.0, &[primitive(mesh)], &Textures::default());
        for y in 0..h {
            for x in 0..w {
                let p = &px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                let inside = (2..12).contains(&x) && (2..10).contains(&y);
                let want: [u8; 4] = if inside { [0, 0, 128, 128] } else { [0; 4] };
                assert_eq!(p, want, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn it_scales_clips_and_draws_over_what_is_there() {
        let (w, h) = (20usize, 20usize);
        // Opaque white under it.
        let mut px = [255u8, 255, 255, 255].repeat(w * h);
        let mesh = quad(
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(10.0, 10.0)),
            Color32::from_rgba_premultiplied(0, 0, 100, 100),
        );
        let clipped = ClippedPrimitive {
            clip_rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(5.0, 10.0)),
            primitive: epaint::Primitive::Mesh(mesh),
        };
        let mut target = Target {
            pixels: &mut px,
            width: w,
            height: h,
        };
        // At 2 pixels a point: 20 px wide, clipped to the left 10 px.
        paint(&mut target, 2.0, &[clipped], &Textures::default());
        let at = |x: usize, y: usize| px[(y * w + x) * 4..(y * w + x) * 4 + 4].to_vec();
        // Blue at 100/255 over white: blue 100 + 255 * 155/255 = 255, red and green 155.
        assert_eq!(at(3, 15), vec![255, 155, 155, 255]);
        assert_eq!(at(12, 5), vec![255, 255, 255, 255], "clipped");
    }

    #[test]
    fn textures_are_sampled_and_patched() {
        let mut textures = Textures::default();
        let id = TextureId::Managed(7);
        let image = egui::ColorImage::new([2, 1], vec![Color32::WHITE, Color32::TRANSPARENT]);
        let mut delta = TexturesDelta::default();
        delta.set.insert(
            id,
            vec![egui::epaint::ImageDelta::full(image, Default::default())].into(),
        );
        textures.set(&delta);
        delta.clear();
        assert_eq!(textures.sample(id, 0.25, 0.5), [255.0; 4]);
        assert_eq!(textures.sample(id, 0.75, 0.5), [0.0; 4]);
        // A patch over the second texel.
        let patch = egui::ColorImage::new([1, 1], vec![Color32::from_gray(100)]);
        let mut delta = TexturesDelta::default();
        delta.set.insert(
            id,
            vec![egui::epaint::ImageDelta::partial(
                [1, 0],
                patch,
                Default::default(),
            )]
            .into(),
        );
        textures.set(&delta);
        delta.clear();
        assert_eq!(textures.sample(id, 0.75, 0.5), [100.0, 100.0, 100.0, 255.0]);
        let mut delta = TexturesDelta::default();
        delta.free.insert(id);
        textures.free(&delta);
        delta.clear();
        assert_eq!(textures.sample(id, 0.75, 0.5), [255.0; 4], "gone: white");
    }
}
