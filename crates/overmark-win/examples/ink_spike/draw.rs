//! Drawing over every monitor, for the eye check: one ink layer per monitor taking the mouse
//! and pen (WM_POINTER, no hooks), a highlighter layer under it composed by MIN_BLEND (a
//! darken blend against the live screen), a dim veil and a toolbar stand-in excluded from
//! capture. On exit it prints the frame cost and the time from input to present.
//!
//! The strokes are kept apart from what's under them, as the app will: a frozen frame is a
//! clean grab of the screen (the Magnification API leaves our windows out while they stay on
//! screen), shown under the strokes, so clearing the strokes leaves it clean. A save is a clean
//! grab (or the frozen frame) with core compositing the strokes on it.
//!
//! Keys: Esc quits; C clears the strokes; H switches pen and highlighter; F freezes or
//! unfreezes; D dims; S saves each monitor (clean screen + strokes) and W saves it without the
//! strokes, to %TEMP%\overmark-spike-save-N.bmp and %TEMP%\overmark-spike-clean-N.bmp.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use overmark_win::core::composite;
use overmark_win::win32::Rect;
use overmark_win::win32::affinity;
use overmark_win::win32::capture::dd::Duplication;
use overmark_win::win32::capture::{Frame, close};
use overmark_win::win32::clock;
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::magnifier::Magnifier;
use overmark_win::win32::pointer::{self, Sample};
use overmark_win::win32::window::{self, Handler, Options, Window};
use tiny_skia::{Color, PixmapMut};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_ERASEBKGND, WM_KEYDOWN, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE,
};

use crate::ink::{InkLayer, Path, bgra, now, percentiles, segment};

enum Event {
    Pointer(Sample),
    Up(u32),
    Key(u32),
}

type Queue = Rc<RefCell<Vec<Event>>>;

fn ink_handler(queue: Queue) -> Handler {
    Box::new(move |_, msg, wp, _| match msg {
        WM_POINTERDOWN | WM_POINTERUPDATE => {
            let mut q = queue.borrow_mut();
            q.extend(pointer::samples(wp).into_iter().map(Event::Pointer));
            Some(LRESULT(0))
        }
        WM_POINTERUP => {
            queue.borrow_mut().push(Event::Up((wp.0 & 0xFFFF) as u32));
            Some(LRESULT(0))
        }
        WM_KEYDOWN => {
            queue.borrow_mut().push(Event::Key(wp.0 as u32));
            Some(LRESULT(0))
        }
        _ => None,
    })
}

struct Screen {
    monitor: Monitor,
    /// Shows the frozen frame (when frozen) with the strokes over it.
    ink: InkLayer,
    /// The highlighter's strokes, composed with the live screen by MIN_BLEND; folded into the
    /// ink layer by core's darken while frozen (the frozen frame hides what's under it).
    highlight: InkLayer,
    dim: Window,
    /// The pen's strokes alone, premultiplied BGRA.
    strokes: Vec<u8>,
    /// A clean grab, shown under the strokes while frozen.
    frozen: Option<Vec<u8>>,
    magnifier: Option<Magnifier>,
    dup: Option<Duplication>,
    ink_dirty: Rect,
    highlight_dirty: Rect,
}

impl Screen {
    fn full(&self) -> Rect {
        Rect::new(0, 0, self.ink.width(), self.ink.height())
    }

    /// Rebuilds the ink layer's pixels in `r` from the frozen frame, the highlighter and the
    /// strokes (core's compositing, row by row).
    fn recompose(&mut self, r: Rect, path: Path) {
        let w = self.ink.width();
        let r = r.clamp_to(w, self.ink.height());
        // Disjoint borrows of the screen's buffers, only the rows in `r` touched: nothing is
        // copied whole per sample.
        let highlight = self.highlight.pixels_ref();
        let frozen = self.frozen.as_deref();
        let strokes = &self.strokes[..];
        let px = self.ink.pixels();
        for y in r.top..r.bottom {
            let a = ((y * w + r.left) * 4) as usize;
            let b = ((y * w + r.right) * 4) as usize;
            match frozen {
                Some(f) => {
                    px[a..b].copy_from_slice(&f[a..b]);
                    composite::darken(&mut px[a..b], &highlight[a..b]);
                    composite::over(&mut px[a..b], &strokes[a..b]);
                }
                None => {
                    px[a..b].copy_from_slice(&strokes[a..b]);
                    if path == Path::Layered {
                        // Windows passes clicks on a layered window's fully transparent pixels
                        // to the window below: alpha 1 keeps them ours.
                        for p in px[a..b].as_chunks_mut::<4>().0 {
                            if p[3] == 0 {
                                *p = [0, 0, 0, 1];
                            }
                        }
                    }
                }
            }
        }
        self.ink_dirty = self.ink_dirty.union(&r);
    }

    /// The monitor now, without any of our windows: the Magnification API, or else Desktop
    /// Duplication with the ink layers excluded for the grab (a share may see them blink).
    fn clean_grab(&mut self) -> Option<(Frame, &'static str)> {
        if let Some(g) = self.magnifier.as_ref().and_then(|m| m.grab(500)) {
            return Some((g, "Magnification API"));
        }
        let dup = self.dup.as_mut()?;
        let _ = affinity::set_excluded(self.ink.hwnd(), true);
        let _ = affinity::set_excluded(self.highlight.hwnd(), true);
        let t = clock::now_100ns();
        let g = dup.grab_settled(t, 25, 500).ok();
        let _ = affinity::set_excluded(self.ink.hwnd(), false);
        let _ = affinity::set_excluded(self.highlight.hwnd(), false);
        g.map(|g| (g.frame, "Desktop Duplication, ink excluded for the grab"))
    }
}

fn save_bmp(path: &std::path::Path, width: i32, height: i32, bgra: &[u8]) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(54 + bgra.len());
    let size = 54 + bgra.len() as u32;
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&(-height).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(bgra);
    std::fs::write(path, out)
}

fn pixmap(px: &mut [u8], w: i32, h: i32) -> PixmapMut<'_> {
    PixmapMut::from_bytes(px, w as u32, h as u32).expect("pixmap")
}

/// Draws a pen or highlighter segment (layer coordinates) on a screen.
fn draw(
    s: &mut Screen,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    highlighter: bool,
    path: Path,
    colour: Option<Color>,
) {
    let (w, h) = (s.ink.width(), s.ink.height());
    if highlighter {
        let r = segment(
            &mut s.highlight.pixmap(),
            from,
            to,
            width,
            colour.unwrap_or_else(marker),
        );
        s.highlight_dirty = s.highlight_dirty.union(&r);
        if s.frozen.is_some() {
            s.recompose(r, path);
        }
    } else {
        let r = segment(
            &mut pixmap(&mut s.strokes, w, h),
            from,
            to,
            width,
            colour.unwrap_or_else(pen),
        );
        s.recompose(r, path);
    }
}

fn pen() -> Color {
    bgra(230, 30, 40, 255)
}

fn marker() -> Color {
    bgra(255, 230, 0, 255)
}

pub fn run(path: &str, smoke: bool) -> windows::core::Result<()> {
    let started = now();
    let path = Path::parse(path).unwrap_or_else(|| {
        eprintln!("unknown path {path:?}: layered or dcomp");
        std::process::exit(2);
    });
    pointer::enable_mouse_in_pointer()?;
    let queue: Queue = Rc::new(RefCell::new(Vec::new()));
    let monitors = dpi::monitors();
    let mut screens = Vec::new();
    for m in &monitors {
        let quiet = Options {
            click_through: true,
            no_activate: true,
            ..Default::default()
        };
        let dim = Window::new(
            m.rect,
            quiet,
            Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
        )?;
        dim.fill_gdi(0, 0, 0);
        dim.set_opacity(110);
        affinity::set_excluded(dim.hwnd(), true)?;
        let mut highlight = InkLayer::new(Path::DComp, m.rect, quiet, Box::new(|_, _, _, _| None))?;
        highlight.set_min_blend(true)?;
        highlight.window().show(false);
        let ink = InkLayer::new(path, m.rect, Options::default(), ink_handler(queue.clone()))?;
        ink.window().show(m.primary);
        let len = (m.rect.width() * m.rect.height() * 4) as usize;
        let mut s = Screen {
            monitor: m.clone(),
            ink,
            highlight,
            dim,
            strokes: vec![0; len],
            frozen: None,
            magnifier: None,
            dup: Duplication::new(m).ok(),
            ink_dirty: Rect::default(),
            highlight_dirty: Rect::default(),
        };
        let full = s.full();
        s.recompose(full, path);
        screens.push(s);
    }
    // The toolbar's stand-in: top centre of the primary monitor, excluded from capture. Grey
    // in Live, blue when frozen, yellow with the highlighter.
    let p = &monitors[0];
    let scale = p.scale();
    let (bw, bh) = ((360.0 * scale) as i32, (44.0 * scale) as i32);
    let bx = p.rect.left + (p.rect.width() - bw) / 2;
    let by = p.rect.top + (16.0 * scale) as i32;
    let toolbar = Window::new(
        Rect::new(bx, by, bx + bw, by + bh),
        Options {
            no_activate: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )?;
    affinity::set_excluded(toolbar.hwnd(), true)?;
    toolbar.show(false);
    // Every window of ours, left out of the clean grabs.
    let mut ours: Vec<HWND> = vec![toolbar.hwnd()];
    for s in &screens {
        ours.extend([s.ink.hwnd(), s.highlight.hwnd(), s.dim.hwnd()]);
    }
    for s in &mut screens {
        s.magnifier = Magnifier::new(s.monitor.rect, &ours).ok();
    }
    let paint_toolbar = |frozen: bool, highlighter: bool| {
        if highlighter {
            toolbar.fill_gdi(150, 130, 20)
        } else if frozen {
            toolbar.fill_gdi(30, 60, 130)
        } else {
            toolbar.fill_gdi(55, 55, 60)
        }
    };
    paint_toolbar(false, false);
    window::pump();
    println!(
        "Drawing with {} on {} monitor(s), layers up in {:.0} ms; clean grabs through {}. Esc quits; C clears; H pen/highlighter; F freeze; D dim; S save; W save without drawing.",
        path.name(),
        screens.len(),
        (now() - started) * 1000.0,
        if screens.iter().all(|s| s.magnifier.is_some()) {
            "the Magnification API"
        } else {
            "Desktop Duplication (the magnifier didn't start)"
        }
    );
    // The smoke run: an opaque pen stroke and a highlighter stroke drawn directly (no input),
    // then the keys through our own window's queue. Its clean saves are checked: no ink in them.
    let smoke_stroke = ((200.0, 300.0), (600.0, 300.0));
    let smoke_marker = ((200.0, 360.0), (600.0, 360.0));
    if smoke {
        let s = &mut screens[0];
        draw(s, smoke_stroke.0, smoke_stroke.1, 12.0, false, path, None);
        draw(s, smoke_marker.0, smoke_marker.1, 22.0, true, path, None);
        let hwnd = s.ink.hwnd();
        // Live: save, save clean; freeze; save, save clean; clear (the frozen frame must stay
        // clean); save clean; unfreeze; quit.
        for k in *b"SWFSWCWF" {
            window::post_key(hwnd, u16::from(k));
        }
        window::post_key(hwnd, 0x1B);
    }

    let mut highlighter = false;
    let mut dimmed = false;
    let mut last: HashMap<u32, (i32, i32)> = HashMap::new();
    let mut frame_cost = Vec::new();
    let mut input_to_present = Vec::new();
    let mut saves = 0;
    let mut smoke_failures = 0;
    'run: loop {
        if !window::pump() {
            break;
        }
        let events: Vec<Event> = std::mem::take(&mut *queue.borrow_mut());
        if events.is_empty() {
            window::wait_for_message(50);
            continue;
        }
        let frame_start = now();
        let mut oldest_input: Option<i64> = None;
        for e in events {
            match e {
                Event::Key(0x1B) => break 'run,
                Event::Key(k) if k == u32::from(b'C') => {
                    for s in &mut screens {
                        s.strokes.fill(0);
                        s.highlight.pixels().fill(0);
                        s.highlight_dirty = s.full();
                        let full = s.full();
                        s.recompose(full, path);
                    }
                }
                Event::Key(k) if k == u32::from(b'H') => {
                    highlighter = !highlighter;
                    paint_toolbar(screens[0].frozen.is_some(), highlighter);
                }
                Event::Key(k) if k == u32::from(b'D') => {
                    dimmed = !dimmed;
                    for s in &screens {
                        if dimmed {
                            s.dim.show(false);
                            s.dim.fill_gdi(0, 0, 0);
                            // Back on top of the dim: the highlighter, then the ink.
                            s.highlight.window().show(false);
                            s.ink.window().show(s.monitor.primary);
                        } else {
                            s.dim.hide();
                        }
                    }
                }
                Event::Key(k) if k == u32::from(b'F') => {
                    let freeze = screens[0].frozen.is_none();
                    for s in &mut screens {
                        if freeze {
                            let t = now();
                            let Some((g, how)) = s.clean_grab() else {
                                eprintln!("no clean grab of {}: not frozen", s.monitor.name);
                                continue;
                            };
                            let mut f = g.bgra;
                            for p in f.as_chunks_mut::<4>().0 {
                                p[3] = 255;
                            }
                            s.frozen = Some(f);
                            s.highlight.window().hide();
                            println!(
                                "Frozen {} ({how}, {:.1} ms)",
                                s.monitor.name,
                                (now() - t) * 1000.0
                            );
                        } else {
                            s.frozen = None;
                            s.highlight.window().show(false);
                            s.ink.window().show(s.monitor.primary);
                        }
                        let full = s.full();
                        s.recompose(full, path);
                    }
                    paint_toolbar(freeze, highlighter);
                }
                Event::Key(k) if k == u32::from(b'S') || k == u32::from(b'W') => {
                    let with_strokes = k == u32::from(b'S');
                    for (i, s) in screens.iter_mut().enumerate() {
                        let t = now();
                        let (mut image, how) = match &s.frozen {
                            Some(f) => (f.clone(), "the frozen frame"),
                            None => match s.clean_grab() {
                                Some((g, how)) => (g.bgra, how),
                                None => {
                                    eprintln!("no clean grab of {}: not saved", s.monitor.name);
                                    continue;
                                }
                            },
                        };
                        if with_strokes {
                            composite::darken(&mut image, s.highlight.pixels());
                            composite::over(&mut image, &s.strokes);
                        }
                        let (w, h) = (s.ink.width(), s.ink.height());
                        if smoke && i == 0 && with_strokes && s.strokes.iter().any(|&b| b != 0) {
                            // A save has the pen stroke composited, and the highlighter darkening.
                            let at = |x: i32, y: i32| -> [u8; 4] {
                                image[((y * w + x) * 4) as usize..][..4].try_into().unwrap()
                            };
                            let ink_ok = close(at(400, 300), [40, 30, 230, 255], 2);
                            let marker_ok = at(400, 360)[0] == 0;
                            if !(ink_ok && marker_ok) {
                                smoke_failures += 1;
                            }
                            println!(
                                "  smoke: save ({how}): the pen stroke {:?} ({}), under the highlighter {:?} (blue darkened to 0: {})",
                                at(400, 300),
                                if ink_ok { "ok" } else { "MISSING" },
                                at(400, 360),
                                if marker_ok { "ok" } else { "NO" }
                            );
                        }
                        if smoke && i == 0 && !with_strokes {
                            // A clean save has none of the opaque pen stroke in it.
                            let (x, y) = (400, 300);
                            let px = image[((y * w + x) * 4) as usize..][..4].try_into().unwrap();
                            let ink_in_it = close(px, [40, 30, 230, 255], 30);
                            if ink_in_it {
                                smoke_failures += 1;
                            }
                            println!(
                                "  smoke: clean save ({how}) under the pen stroke: {px:?}: {}",
                                if ink_in_it { "INK IN IT" } else { "clean" }
                            );
                        }
                        let name = if with_strokes { "save" } else { "clean" };
                        let file = std::env::temp_dir()
                            .join(format!("overmark-spike-{name}-{}.bmp", i + 1));
                        match save_bmp(&file, w, h, &image) {
                            Ok(()) => println!(
                                "Saved {} ({w}x{h}, from {how}, {:.1} ms)",
                                file.display(),
                                (now() - t) * 1000.0
                            ),
                            Err(e) => eprintln!("couldn't save {}: {e}", file.display()),
                        }
                        saves += 1;
                    }
                }
                Event::Key(_) => {}
                Event::Up(id) => {
                    last.remove(&id);
                }
                Event::Pointer(sample) => {
                    if !sample.in_contact {
                        last.remove(&sample.id);
                        continue;
                    }
                    oldest_input =
                        Some(oldest_input.map_or(sample.time_100ns, |o| o.min(sample.time_100ns)));
                    let Some(s) = screens.iter_mut().find(|s| {
                        let r = s.monitor.rect;
                        sample.x >= r.left
                            && sample.x < r.right
                            && sample.y >= r.top
                            && sample.y < r.bottom
                    }) else {
                        continue;
                    };
                    let (ox, oy) = s.ink.origin();
                    let to = ((sample.x - ox) as f32, (sample.y - oy) as f32);
                    let from = last
                        .get(&sample.id)
                        .map_or(to, |&(x, y)| ((x - ox) as f32, (y - oy) as f32));
                    last.insert(sample.id, (sample.x, sample.y));
                    let scale = s.monitor.scale();
                    let width = if highlighter {
                        22.0
                    } else {
                        1.5 + 4.5 * sample.pressure.unwrap_or(0.6)
                    };
                    draw(s, from, to, width * scale, highlighter, path, None);
                }
            }
        }
        let mut presented = false;
        for s in &mut screens {
            if !s.highlight_dirty.is_empty() {
                s.highlight.present(Some(s.highlight_dirty), true)?;
                s.highlight_dirty = Rect::default();
                presented = true;
            }
            if !s.ink_dirty.is_empty() {
                s.ink.present(Some(s.ink_dirty), true)?;
                s.ink_dirty = Rect::default();
                presented = true;
            }
        }
        if presented {
            frame_cost.push(now() - frame_start);
            if let Some(t) = oldest_input {
                input_to_present.push((clock::now_100ns() - t) as f64 / 10_000_000.0);
            }
        }
    }
    drop(toolbar);
    let [c50, c95, c99, cmax] = percentiles(&frame_cost);
    let [l50, l95, l99, lmax] = percentiles(&input_to_present);
    println!(
        "{} frames drawn with {}; frame cost p50 {c50:.3} p95 {c95:.3} p99 {c99:.3} max {cmax:.3} ms",
        frame_cost.len(),
        path.name()
    );
    println!(
        "from the oldest input sample to its present done: p50 {l50:.2} p95 {l95:.2} p99 {l99:.2} max {lmax:.2} ms; {saves} save(s)"
    );
    if smoke {
        println!(
            "smoke: {}",
            if smoke_failures == 0 { "ok" } else { "FAILED" }
        );
        if smoke_failures > 0 {
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `draw --bench`: times this mode's own drawing path, the one a pointer sample takes (`draw`,
/// then `recompose` for pen strokes and frozen highlighter strokes, then the presents), for the
/// pen and the highlighter, Live and Frozen, at the monitor's size and at 3840x2160. Each frame
/// waits for DWM's next composition and takes six samples of a fast scribble (as `bench`). The
/// windows are click-through and never activate, and the strokes faint (alpha 10); the frozen
/// frame is a flat grey (its cost is the same as a grab's).
pub fn bench(seconds: f64) -> windows::core::Result<()> {
    let m = dpi::monitors().first().expect("a monitor").clone();
    println!(
        "draw --bench: {}x{} at {} Hz; each frame six samples through draw, recompose and present.",
        m.rect.width(),
        m.rect.height(),
        m.refresh_hz()
    );
    let quiet = Options {
        click_through: true,
        no_activate: true,
        ..Default::default()
    };
    let uhd = Rect::new(
        m.rect.left,
        m.rect.top,
        m.rect.left + 3840,
        m.rect.top + 2160,
    );
    let mut worst_p99: f64 = 0.0;
    for path in [Path::DComp, Path::Layered] {
        for rect in [m.rect, uhd] {
            let mut highlight =
                InkLayer::new(Path::DComp, rect, quiet, Box::new(|_, _, _, _| None))?;
            highlight.set_min_blend(true)?;
            highlight.window().show(false);
            let ink = InkLayer::new(path, rect, quiet, Box::new(|_, _, _, _| None))?;
            ink.window().show(false);
            let dim = Window::new(rect, quiet, Box::new(|_, _, _, _| None))?;
            let len = window_len(rect);
            let mut s = Screen {
                monitor: m.clone(),
                ink,
                highlight,
                dim,
                strokes: vec![0; len],
                frozen: None,
                magnifier: None,
                dup: None,
                ink_dirty: Rect::default(),
                highlight_dirty: Rect::default(),
            };
            let full = s.full();
            s.recompose(full, path);
            s.ink.present(None, true)?;
            let (w, h) = (rect.width() as f32, rect.height() as f32);
            let at = |t: f64| -> (f32, f32) {
                let t = t as f32;
                (
                    w * 0.5 + w * 0.42 * (t * 1.9).sin(),
                    h * 0.5 + h * 0.42 * (t * 2.7 + 0.6).sin(),
                )
            };
            for (frozen, highlighter) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                s.frozen = frozen.then(|| {
                    let mut f = vec![0u8; len];
                    for p in f.as_chunks_mut::<4>().0 {
                        *p = [96, 96, 96, 255];
                    }
                    f
                });
                s.strokes.fill(0);
                s.highlight.pixels().fill(0);
                let full = s.full();
                s.recompose(full, path);
                s.ink.present(None, true)?;
                s.highlight.present(None, true)?;
                s.ink_dirty = Rect::default();
                s.highlight_dirty = Rect::default();
                let colour = if highlighter {
                    bgra(255, 230, 0, 10)
                } else {
                    bgra(230, 30, 40, 10)
                };
                let mut costs = Vec::new();
                let start = now();
                let mut last_t = 0.0;
                while now() - start < seconds {
                    window::pump();
                    window::wait_for_composition();
                    let frame_start = now();
                    let t = frame_start - start;
                    for i in 1..=6 {
                        let a = last_t + (t - last_t) * f64::from(i - 1) / 6.0;
                        let b = last_t + (t - last_t) * f64::from(i) / 6.0;
                        let width = if highlighter { 22.0 } else { 6.0 };
                        draw(&mut s, at(a), at(b), width, highlighter, path, Some(colour));
                    }
                    if !s.highlight_dirty.is_empty() {
                        s.highlight.present(Some(s.highlight_dirty), true)?;
                        s.highlight_dirty = Rect::default();
                    }
                    if !s.ink_dirty.is_empty() {
                        s.ink.present(Some(s.ink_dirty), true)?;
                        s.ink_dirty = Rect::default();
                    }
                    costs.push(now() - frame_start);
                    last_t = t;
                }
                let [p50, p95, p99, max] = percentiles(&costs);
                worst_p99 = worst_p99.max(p99);
                println!(
                    "  {:<20} {}x{} {:<6} {:<11} {:>4} frames: p50 {p50:.3}  p95 {p95:.3}  p99 {p99:.3}  max {max:.3} ms",
                    path.name(),
                    rect.width(),
                    rect.height(),
                    if frozen { "Frozen" } else { "Live" },
                    if highlighter { "highlighter" } else { "pen" },
                    costs.len()
                );
            }
            window::pump();
        }
    }
    println!(
        "draw --bench: worst p99 {worst_p99:.3} ms against the 3.47 ms bar: {}",
        if worst_p99 <= 1000.0 / 144.0 / 2.0 {
            "PASS"
        } else {
            "FAIL"
        }
    );
    Ok(())
}

fn window_len(r: Rect) -> usize {
    overmark_win::win32::byte_len(r.width(), r.height()).expect("a layer's size")
}
