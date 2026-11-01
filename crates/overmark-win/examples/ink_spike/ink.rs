//! An ink layer on either path, and the stroke drawing both share (tiny-skia, as core will).

use overmark_win::win32::Rect;
use overmark_win::win32::dcomp::CompSurface;
use overmark_win::win32::layered::LayeredSurface;
use overmark_win::win32::window::{Handler, Options, Window};
use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke, Transform};
use windows::Win32::Foundation::HWND;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// `UpdateLayeredWindowIndirect` from a DIB the renderer draws into.
    Layered,
    /// DirectComposition and a D3D11 swap chain, uploaded from the renderer's pixels.
    DComp,
}

impl Path {
    pub fn parse(s: &str) -> Option<Path> {
        match s {
            "layered" | "ulw" => Some(Path::Layered),
            "dcomp" | "dc" => Some(Path::DComp),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Path::Layered => "UpdateLayeredWindow",
            Path::DComp => "DirectComposition",
        }
    }
}

enum Surface {
    Layered(LayeredSurface),
    DComp(CompSurface, Vec<u8>),
}

/// How long a present took, in seconds: copying pixels to the GPU (DirectComposition only)
/// and the present call itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct PresentTiming {
    pub upload: f64,
    pub present: f64,
}

/// A top-most window over `rect`, showing a premultiplied BGRA image the size of `rect`.
pub struct InkLayer {
    surface: Surface,
    window: Window,
    origin: (i32, i32),
    width: i32,
    height: i32,
}

impl InkLayer {
    pub fn new(
        path: Path,
        rect: Rect,
        mut options: Options,
        handler: Handler,
    ) -> windows::core::Result<InkLayer> {
        match path {
            Path::Layered => options.layered = true,
            Path::DComp => {
                options.no_redirection_bitmap = true;
            }
        }
        let window = Window::new(rect, options, handler)?;
        let (w, h) = (rect.width(), rect.height());
        let surface = match path {
            Path::Layered => Surface::Layered(LayeredSurface::new(w, h)?),
            Path::DComp => Surface::DComp(
                CompSurface::new(window.hwnd(), w, h)?,
                vec![0u8; (w * h * 4) as usize],
            ),
        };
        let mut layer = InkLayer {
            surface,
            window,
            origin: (rect.left, rect.top),
            width: w,
            height: h,
        };
        layer.present(None, false)?;
        Ok(layer)
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// DirectComposition only: compose by each channel's minimum (see `CompSurface`).
    pub fn set_min_blend(&mut self, on: bool) -> windows::core::Result<()> {
        match &mut self.surface {
            Surface::DComp(s, _) => s.set_min_blend(on),
            Surface::Layered(_) => Ok(()),
        }
    }

    pub fn pixels(&mut self) -> &mut [u8] {
        match &mut self.surface {
            Surface::Layered(s) => s.pixels(),
            Surface::DComp(_, p) => p,
        }
    }

    /// The pixels, read-only: borrowed beside another layer's mutable ones, never copied.
    pub fn pixels_ref(&self) -> &[u8] {
        match &self.surface {
            Surface::Layered(s) => s.pixels_ref(),
            Surface::DComp(_, p) => p,
        }
    }

    pub fn pixmap(&mut self) -> PixmapMut<'_> {
        let (w, h) = (self.width as u32, self.height as u32);
        PixmapMut::from_bytes(self.pixels(), w, h).expect("pixmap")
    }

    /// Shows what changed in `dirty` (`None`: everything).
    pub fn present(
        &mut self,
        dirty: Option<Rect>,
        vsync: bool,
    ) -> windows::core::Result<PresentTiming> {
        let mut t = PresentTiming::default();
        match &mut self.surface {
            Surface::Layered(s) => {
                let t0 = now();
                s.present(self.window.hwnd(), self.origin, dirty)?;
                t.present = now() - t0;
            }
            Surface::DComp(s, pixels) => {
                let t0 = now();
                s.upload(pixels, dirty);
                let t1 = now();
                s.present(dirty, vsync)?;
                t.upload = t1 - t0;
                t.present = now() - t1;
            }
        }
        Ok(t)
    }
}

pub fn now() -> f64 {
    overmark_win::win32::clock::now_secs()
}

/// A colour as the surfaces store it: tiny-skia writes RGBA, the surfaces read BGRA, so red
/// and blue are swapped going in.
pub fn bgra(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(b, g, r, a)
}

/// Draws a round-capped segment and returns the rectangle it touched.
pub fn segment(
    pm: &mut PixmapMut<'_>,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    color: Color,
) -> Rect {
    let mut pb = PathBuilder::new();
    pb.move_to(from.0, from.1);
    // A zero-length segment still draws a dot with round caps.
    pb.line_to(to.0 + 0.01, to.1);
    let Some(path) = pb.finish() else {
        return Rect::default();
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    let pad = width / 2.0 + 2.0;
    Rect::new(
        (from.0.min(to.0) - pad).floor() as i32,
        (from.1.min(to.1) - pad).floor() as i32,
        (from.0.max(to.0) + pad).ceil() as i32,
        (from.1.max(to.1) + pad).ceil() as i32,
    )
    .clamp_to(pm.width() as i32, pm.height() as i32)
}

/// Fills `rect` with `color` (replacing what's there) and returns it.
pub fn fill(pixels: &mut [u8], width: i32, rect: Rect, color: [u8; 4]) -> Rect {
    for y in rect.top..rect.bottom {
        for x in rect.left..rect.right {
            let i = ((y * width + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&color);
        }
    }
    rect
}

/// Percentiles of `samples` (seconds), as milliseconds: p50, p95, p99, max.
pub fn percentiles(samples: &[f64]) -> [f64; 4] {
    if samples.is_empty() {
        return [0.0; 4];
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize] * 1000.0;
    [at(0.50), at(0.95), at(0.99), s[s.len() - 1] * 1000.0]
}
