//! Turning a clean grab and the drawing into a capture: composing at full resolution with
//! core's renderer, cropping a region, encoding PNG or JPG, writing the file under the
//! settings' name, and copying to the clipboard.
//!
//! The grab is always a clean one (the frozen frame, or a grab without Overmark's windows), and
//! the drawing is rendered over it here: the composited overlay is never screenshotted, so a
//! save has no dim, no toolbar and the ink exactly once.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tiny_skia::Pixmap;

use crate::core::drawing::Drawing;
use crate::core::render::{self, Background, Highlight, RenderOptions};
use crate::core::settings::ImageFormat;
use crate::core::settings::file_name::{self, CaptureTime, DEFAULT_PATTERN};
use crate::win32::Rect;
use crate::win32::capture::Frame;

/// A capture: rows of straight RGBA pixels, opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The pixel at (`x`, `y`) as RGBA.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// As BGRA, for Windows' bitmaps.
    pub fn to_bgra(&self) -> Vec<u8> {
        let mut out = self.rgba.clone();
        for p in out.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
        out
    }
}

/// The guides a save draws over its base: which, how, and at the monitor's scale.
pub type SaveGuides = (
    crate::core::backdrop::Guides,
    crate::core::backdrop::GuideStyle,
    crate::core::render::View,
);

/// What's under the ink: `frame` (BGRA) as an opaque premultiplied RGBA pixmap, with `guides`
/// drawn over it. A save's base, and what a blur on the frozen frame pixelates on screen: one
/// composition for both, so a blur looks the same in either where the guides are the same.
pub fn base(frame: &Frame, guides: Option<&SaveGuides>) -> Option<Pixmap> {
    let (w, h) = (
        u32::try_from(frame.width).ok()?,
        u32::try_from(frame.height).ok()?,
    );
    let mut base = Pixmap::new(w, h)?;
    if base.data().len() != frame.bgra.len() {
        return None;
    }
    for (d, s) in base
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(frame.bgra.as_chunks::<4>().0)
    {
        *d = [s[2], s[1], s[0], 255];
    }
    if let Some((kind, style, view)) = guides {
        crate::core::backdrop::render_guides(&mut base.as_mut(), *kind, style, *view, None);
    }
    Some(base)
}

/// [`base`], kept until the guides it was made with change (or [`Under::reset`], for a new
/// frame): the frozen frame under the screen's blurs.
#[derive(Default)]
pub struct Under {
    made_with: Option<Option<SaveGuides>>,
    pixmap: Option<Pixmap>,
}

impl Under {
    /// The base of `frame` with `guides`, made again only if the guides differ from last time.
    pub fn get(&mut self, frame: &Frame, guides: Option<&SaveGuides>) -> Option<&Pixmap> {
        let key = guides.copied();
        if self.made_with != Some(key) || self.pixmap.is_none() {
            self.pixmap = base(frame, guides);
            self.made_with = Some(key);
        }
        self.pixmap.as_ref()
    }

    /// Forgets it (the frame changed).
    pub fn reset(&mut self) {
        *self = Under::default();
    }
}

/// The save's base `frame` (BGRA: the clean grab, the frozen frame, or a board's colour) with
/// `guides` over it (when saves include them), then `drawing` rendered over both at full
/// resolution (none: "save without the drawing"), as core's `composite::compose_save` orders
/// them. Highlighter strokes darken what's under them, as the live highlighter shows them.
/// `now_ms` is the drawing's clock, for disappearing ink.
pub fn compose(
    frame: &Frame,
    guides: Option<&SaveGuides>,
    drawing: Option<&Drawing>,
    now_ms: u64,
    disappearing_ink_ms: Option<u64>,
) -> Option<Image> {
    let base = base(frame, guides)?;
    let (w, h) = (base.width(), base.height());
    let out = match drawing {
        Some(d) if !d.is_empty() => {
            let mut out = Pixmap::new(w, h)?;
            let options = RenderOptions {
                highlight: Highlight::Darken,
                now_ms,
                disappearing_ink_ms,
                ..Default::default()
            };
            render::render_with(
                d,
                &mut out.as_mut(),
                &options,
                Background::Image(base.as_ref()),
            );
            out
        }
        _ => base,
    };
    // Opaque, so premultiplied and straight are the same.
    Some(Image {
        width: w,
        height: h,
        rgba: out.take(),
    })
}

/// The part of `image` in `rect` (clipped to it); `None` if nothing is left.
pub fn crop(image: &Image, rect: Rect) -> Option<Image> {
    let r = rect.clamp_to(image.width as i32, image.height as i32);
    if r.is_empty() {
        return None;
    }
    let (w, h) = (r.width() as u32, r.height() as u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in r.top as u32..r.bottom as u32 {
        let a = ((y * image.width + r.left as u32) * 4) as usize;
        rgba.extend_from_slice(&image.rgba[a..a + (w * 4) as usize]);
    }
    Some(Image {
        width: w,
        height: h,
        rgba,
    })
}

/// The file's bytes in `format` (JPG at `quality`, 1-100).
pub fn encode(image: &Image, format: ImageFormat, quality: u8) -> io::Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    match format {
        ImageFormat::Png => image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                &image.rgba,
                image.width,
                image.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(io::Error::other)?,
        ImageFormat::Jpg => {
            let rgb: Vec<u8> = image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .write_image(
                    &rgb,
                    image.width,
                    image.height,
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(io::Error::other)?
        }
    }
    Ok(out)
}

/// Why a capture couldn't be saved: the folder, and what went wrong.
#[derive(Debug)]
pub struct SaveError {
    pub folder: PathBuf,
    pub reason: io::Error,
}

/// Writes `image` into `folder` (made if it's missing) under the settings' name pattern for
/// `time`, in `format`. The file is created new (`create_new`): a name taken meanwhile gets the
/// next number, never an overwrite. A pattern that can't be used falls back to the default.
pub fn write_capture(
    folder: &Path,
    pattern: &str,
    time: &CaptureTime,
    format: ImageFormat,
    quality: u8,
    image: &Image,
) -> Result<PathBuf, SaveError> {
    let fail = |reason| SaveError {
        folder: folder.to_path_buf(),
        reason,
    };
    fs::create_dir_all(folder).map_err(fail)?;
    let bytes = encode(image, format, quality).map_err(fail)?;
    let pattern = if file_name::check_pattern(pattern).is_ok() {
        pattern
    } else {
        DEFAULT_PATTERN
    };
    for _ in 0..100 {
        let path = file_name::capture_path(folder, pattern, time, format.extension())
            .map_err(|e| fail(io::Error::other(e.to_string())))?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(&bytes).map_err(fail)?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(fail(e)),
        }
    }
    Err(fail(io::Error::other("no free file name")))
}

/// Puts `image` on the clipboard.
pub fn copy(image: &Image) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: std::borrow::Cow::Borrowed(&image.rgba),
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::colour::Colour;
    use crate::core::drawing::InkPoint;
    use crate::core::session::Session;
    use crate::core::tool::ToolKind;

    const T: CaptureTime = CaptureTime {
        year: 2026,
        month: 10,
        day: 21,
        hour: 9,
        minute: 42,
        second: 17,
        millisecond: 5,
    };

    /// A 200x100 grab of one colour (BGRA).
    fn grab(bgra: [u8; 4]) -> Frame {
        Frame {
            width: 200,
            height: 100,
            bgra: bgra.repeat(200 * 100),
        }
    }

    /// A horizontal pen stroke across the middle, in `colour`.
    fn stroke(colour: Colour, tool: ToolKind) -> Session {
        let mut s = Session::new();
        s.tool = tool;
        s.colour = colour;
        s.width = 16.0;
        s.stroke_begin(InkPoint::mouse(20.0, 50.0, 0));
        for i in 1..=30u32 {
            s.stroke_extend(InkPoint::mouse(
                20.0 + 5.0 * f64::from(i),
                50.0,
                u64::from(i) * 16,
            ));
        }
        s.stroke_end();
        s
    }

    /// A blur from (16, 16) to (112, 112), 16 px blocks, drawn on the frozen frame.
    fn blurred() -> Session {
        use crate::core::backdrop::Backdrop;
        let mut s = Session::new();
        s.backdrop = Backdrop::Frozen;
        s.tool = ToolKind::Blur;
        s.width = 16.0;
        s.pointer_down(InkPoint::mouse(16.0, 16.0, 0), false)
            .unwrap();
        s.pointer_up(InkPoint::mouse(112.0, 112.0, 16), false);
        s
    }

    #[test]
    fn a_blur_over_the_guides_is_the_same_on_screen_and_in_a_save() {
        use crate::core::backdrop::{Backdrop, GuideStyle, Guides};
        use crate::core::render::View;
        use crate::core::settings::{BackdropSettings, GuideSettings};
        // A white frozen frame, a grid every 8 px, a blur over it.
        let frame = Frame {
            width: 128,
            height: 128,
            bgra: [255, 255, 255, 255].repeat(128 * 128),
        };
        let settings = GuideSettings {
            spacing: 8,
            ..GuideSettings::default()
        };
        let style = GuideStyle::new(&settings, &BackdropSettings::default(), Backdrop::Frozen);
        let guides = (Guides::Grid, style, View::default());
        let s = blurred();
        let saved = compose(&frame, Some(&guides), Some(s.drawing()), 16, None).unwrap();
        // The screen's blur pass renders over the same base (when shares see the guides).
        let mut under = Under::default();
        let base = under.get(&frame, Some(&guides)).unwrap().clone();
        let mut screen = Pixmap::new(128, 128).unwrap();
        let options = RenderOptions {
            now_ms: 16,
            ..Default::default()
        };
        render::render_with(
            s.drawing(),
            &mut screen.as_mut(),
            &options,
            Background::Image(base.as_ref()),
        );
        let on_screen = screen.pixel(64, 64).unwrap();
        let p = saved.pixel(64, 64);
        assert_eq!(
            [
                on_screen.red(),
                on_screen.green(),
                on_screen.blue(),
                on_screen.alpha()
            ],
            p
        );
        assert!(p[0] < 255, "the guides are in the blur: {p:?}");
        // G turns the guides off: the base is made again, without them.
        let plain = under.get(&frame, None).unwrap();
        assert_eq!(plain.pixel(64, 64).unwrap().red(), 255);
        assert_eq!(plain.pixel(8, 8).unwrap().red(), 255, "no grid line");
    }

    #[test]
    fn the_guides_go_over_the_base_and_under_the_ink() {
        use crate::core::backdrop::{Backdrop, GuideStyle, Guides};
        use crate::core::render::View;
        use crate::core::settings::{BackdropSettings, GuideSettings};
        // A whiteboard, its grid every 32 px, and an opaque red stroke along y = 50.
        let frame = grab([255, 255, 255, 255]);
        let style = GuideStyle::new(
            &GuideSettings::default(),
            &BackdropSettings::default(),
            Backdrop::Whiteboard,
        );
        let guides = (Guides::Grid, style, View::default());
        let s = stroke(Colour::rgb(255, 0, 0), ToolKind::Pen);
        let img = compose(&frame, Some(&guides), Some(s.drawing()), 10_000, None).unwrap();
        // On a grid line, away from the stroke: the board darkened by the guide.
        let line = img.pixel(64, 10);
        assert!(line[0] < 250 && line[3] == 255, "{line:?}");
        // Between the lines: the board.
        assert_eq!(img.pixel(80, 10), [255, 255, 255, 255]);
        // Where the stroke crosses a line: the ink, over the guide.
        assert_eq!(img.pixel(64, 50), [255, 0, 0, 255]);
        // Without guides (Settings' include_in_saves off), the line's pixel is the board.
        let bare = compose(&frame, None, Some(s.drawing()), 10_000, None).unwrap();
        assert_eq!(bare.pixel(64, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn composes_the_drawing_once_over_the_grab() {
        // A backdrop of RGB (200, 100, 50); a pen at half alpha in pure blue.
        let frame = grab([50, 100, 200, 255]);
        let s = stroke(
            Colour {
                r: 0,
                g: 0,
                b: 255,
                a: 128,
            },
            ToolKind::Pen,
        );
        let img = compose(&frame, None, Some(s.drawing()), 10_000, None).unwrap();
        assert_eq!((img.width, img.height), (200, 100));
        // Away from the stroke: the grab, untouched.
        assert_eq!(img.pixel(5, 5), [200, 100, 50, 255]);
        // In the middle of the stroke: blue over the backdrop once, about halfway. Twice (ink
        // in the grab and again on top) would be about (50, 25, 204).
        let p = img.pixel(100, 50);
        assert!(
            p[0].abs_diff(100) <= 3 && p[1].abs_diff(50) <= 3 && p[2].abs_diff(152) <= 3,
            "{p:?}"
        );
        // Without the drawing: the grab alone.
        let bare = compose(&frame, None, None, 0, None).unwrap();
        assert!(
            bare.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [200, 100, 50, 255])
        );
    }

    #[test]
    fn a_highlighter_darkens_as_it_does_live() {
        let frame = grab([50, 100, 200, 255]);
        let s = stroke(Colour::rgb(255, 255, 0), ToolKind::Highlighter);
        let img = compose(&frame, None, Some(s.drawing()), 10_000, None).unwrap();
        // Yellow darkening (200, 100, 50): each channel's minimum, (200, 100, 0).
        let p = img.pixel(100, 50);
        assert!(
            p[0].abs_diff(200) <= 3 && p[1].abs_diff(100) <= 3 && p[2] <= 3,
            "{p:?}"
        );
    }

    #[test]
    fn crops_to_the_region_clipped_to_the_image() {
        let mut frame = grab([0, 0, 0, 255]);
        let i = ((10 * 200 + 30) * 4) as usize;
        frame.bgra[i..i + 4].copy_from_slice(&[3, 2, 1, 255]);
        let img = compose(&frame, None, None, 0, None).unwrap();
        let c = crop(&img, Rect::new(30, 10, 60, 40)).unwrap();
        assert_eq!((c.width, c.height), (30, 30));
        assert_eq!(c.pixel(0, 0), [1, 2, 3, 255]);
        let edge = crop(&img, Rect::new(190, 90, 400, 400)).unwrap();
        assert_eq!((edge.width, edge.height), (10, 10));
        assert!(crop(&img, Rect::new(300, 0, 400, 10)).is_none());
    }

    #[test]
    fn writes_named_files_new_each_time_in_a_folder_it_makes() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Pictures").join("Overmark");
        let img = compose(&grab([10, 20, 30, 255]), None, None, 0, None).unwrap();
        let first =
            write_capture(&folder, DEFAULT_PATTERN, &T, ImageFormat::Png, 90, &img).unwrap();
        assert_eq!(first, folder.join("2026-10-21 09.42.17.png"));
        let second =
            write_capture(&folder, DEFAULT_PATTERN, &T, ImageFormat::Png, 90, &img).unwrap();
        assert_eq!(second, folder.join("2026-10-21 09.42.17 (2).png"));
        // The PNG reads back as the image.
        let back = image::load_from_memory(&fs::read(&first).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!((back.width(), back.height()), (200, 100));
        assert_eq!(back.get_pixel(3, 3).0, [30, 20, 10, 255]);
        // JPG, and a pattern that can't be used falls back to the default.
        let jpg = write_capture(&folder, "{oops", &T, ImageFormat::Jpg, 80, &img).unwrap();
        assert_eq!(jpg.extension().unwrap(), "jpg");
        assert!(
            jpg.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("2026-10-21 09.42.17")
        );
        let back = image::load_from_memory(&fs::read(&jpg).unwrap())
            .unwrap()
            .to_rgb8();
        let p = back.get_pixel(100, 50).0;
        assert!(
            p[0].abs_diff(30) <= 4 && p[1].abs_diff(20) <= 4 && p[2].abs_diff(10) <= 4,
            "{p:?}"
        );
    }

    #[test]
    fn a_folder_that_cant_be_made_is_an_error_naming_it() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("not-a-folder");
        fs::write(&file, "").unwrap();
        let img = compose(&grab([0, 0, 0, 255]), None, None, 0, None).unwrap();
        let e = write_capture(
            &file.join("sub"),
            DEFAULT_PATTERN,
            &T,
            ImageFormat::Png,
            90,
            &img,
        )
        .unwrap_err();
        assert_eq!(e.folder, file.join("sub"));
    }
}
