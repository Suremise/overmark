//! Checks the toolbar on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example toolbar_check
//! ```
//!
//! It starts `overmark.exe` with `%APPDATA%` and the captures folder in temp folders of its own,
//! over a window of a known colour, opens the layer through the app's own hotkey window, and
//! works the toolbar **as a screen reader does: through UI Automation on the toolbar's own
//! window** (what Narrator reads, and its "press": the Invoke pattern), and its grip through
//! `WM_PANEL_POINTER` posted to that window. Nothing global is sent. It checks:
//!
//! - every control is there by its name, Undo and Redo greyed out until they can act;
//! - each control does what its key does: Undo, Redo, Clear, Freeze and Unfreeze, Save, Save a
//!   region (cancelled), Save without the drawing (in More), Collapse and Expand, Close;
//!   judged from saved captures and the log (Copy isn't pressed: it would take the clipboard);
//! - each group (Backdrop, Guides, Tool, Colour, Width) opens its strip of choices,
//!   a choice there acts, and the strip stays open while drawing; its up arrow closes it; the
//!   arrows sent to the ink layer move the width's slider;
//! - the toolbar takes the pointer where it's drawn (its window is the one under it), lets it
//!   through elsewhere, a tooltip below it and a few pixels round its frame too, and
//!   is still on top after a region;
//! - **a share** (Windows Graphics Capture, after a barrier) **sees the ink and not the
//!   toolbar**, whose window is excluded from capture;
//! - dragged by its grip, it stays where it was left when the layer opens again, and Settings
//!   have it;
//! - no CPU with the layer and toolbar open and nothing moving;
//! - hotkey to the layer and toolbar on screen within 100 ms (the log's `visible_ms`), over 8 opens.
//!
//! Run it with no Overmark running (it stops if one is), when you're not typing: the layer
//! takes the screen and the focus for a few seconds.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark_core::hotkey::Hotkey;
use overmark_core::settings::{AfterCapture, DEFAULT_PALETTE, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::overlay::{
    Command as Capture, Phase, WM_OVERMARK_COMMAND, WM_OVERMARK_KEY, WM_OVERMARK_POINTER,
    key_message, pointer_message,
};
use overmark_win::win32::Rect;
use overmark_win::win32::affinity;
use overmark_win::win32::capture::wgc::{Observer, frame_after_barrier};
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::panel::{Hit, WM_PANEL_POINTER, hit_test, panel_pointer};
use overmark_win::win32::pointer::{PointerKind, Sample};
use overmark_win::win32::process::Process;
use overmark_win::win32::uia::{Element, Uia};
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// The window behind, under the toolbar and the stroke: RGB (200, 100, 50).
const BACK: [u8; 3] = [200, 100, 50];
const STROKE_Y: i32 = 600;
const SEEN_WITHIN_MS: u32 = 3000;
const OPENS: usize = 8;

struct Owned(Option<Child>);

impl Owned {
    fn pid(&self) -> u32 {
        self.0.as_ref().map_or(0, Child::id)
    }
    fn stop(&mut self, exe: &Path) -> bool {
        let Some(mut c) = self.0.take() else {
            return true;
        };
        if matches!(c.try_wait(), Ok(None)) {
            let _ = Command::new(exe).arg("--quit").status();
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if matches!(c.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = c.kill();
        let _ = c.wait();
        false
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            if matches!(c.try_wait(), Ok(None)) {
                eprintln!(
                    "toolbar_check: stopping the Overmark it started (pid {})",
                    c.id()
                );
                let _ = c.kill();
            }
            let _ = c.wait();
        }
    }
}

struct App {
    owned: Owned,
    lines: mpsc::Receiver<String>,
}

impl App {
    fn start(exe: &Path, appdata: &Path) -> App {
        let mut child = Command::new(exe)
            .env("APPDATA", appdata)
            .stderr(Stdio::piped())
            .spawn()
            .expect("start overmark.exe");
        let stderr = child.stderr.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                println!("    | {line}");
                let _ = tx.send(line);
            }
        });
        let app = App {
            owned: Owned(Some(child)),
            lines,
        };
        assert!(
            app.wait_for("resident in the tray", Duration::from_secs(10))
                .is_some(),
            "the app didn't start"
        );
        app
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> Option<String> {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            match self.lines.recv_timeout(Duration::from_millis(20)) {
                Ok(line) if line.contains(needle) => return Some(line),
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    window::pump();
                }
                Err(_) => return None,
            }
        }
        None
    }

    fn window(&self, timeout: Duration, want: impl Fn(&WindowInfo) -> bool) -> Option<WindowInfo> {
        let until = Instant::now() + timeout;
        loop {
            if let Some(w) = process_windows(self.owned.pid()).into_iter().find(&want) {
                return Some(w);
            }
            if Instant::now() >= until {
                return None;
            }
            window::pump();
            window::wait_for_message(5);
        }
    }

    /// Opens the layer (Win+Shift+D through the app's hotkey window): the ink layer, the
    /// toolbar's window and the `opened` line (its `visible_ms` covers the toolbar).
    fn open(&self, m: &Monitor) -> (HWND, WindowInfo, String) {
        let hotkeys = self
            .window(Duration::from_secs(5), |w| w.class == "global_hotkey_app")
            .expect("the hotkey window");
        let h = Hotkey::parse("Win+Shift+D").unwrap().unwrap();
        window::post(hotkeys.hwnd, WM_HOTKEY, to_hotkey(&h).id() as usize, 0);
        let shown = self
            .wait_for(" opened", Duration::from_secs(3))
            .expect("the layer and the toolbar showed");
        let rect = m.rect;
        let ink = self
            .window(Duration::from_secs(3), |w| {
                w.class == "OvermarkSurface"
                    && w.visible
                    && w.ex_style & WS_EX_TRANSPARENT == 0
                    && w.title == "Overmark"
                    && w.rect == rect
            })
            .expect("the ink layer");
        let bar = self
            .window(Duration::from_secs(3), |w| {
                w.title == "Overmark toolbar" && w.visible
            })
            .expect("the toolbar's window");
        (ink.hwnd, bar, shown)
    }

    fn save(&self, ink: HWND, command: Option<Capture>) -> Option<image::RgbaImage> {
        if let Some(c) = command {
            window::post(ink, WM_OVERMARK_COMMAND, c.wparam(), 0);
        }
        let line = self.wait_for(" delivered", Duration::from_secs(10))?;
        let at = line.find("file=Some(\"")? + "file=Some(\"".len();
        let end = line[at..].find("\")")?;
        let file = PathBuf::from(line[at..at + end].replace("\\\\", "\\"));
        Some(
            image::load_from_memory(&std::fs::read(file).ok()?)
                .ok()?
                .to_rgba8(),
        )
    }

    fn close(&self, ink: HWND) {
        window::post(ink, WM_OVERMARK_KEY, 0x1B, 0);
        self.wait_for("closed", Duration::from_secs(3));
    }
}

/// The screen's pixel at (`x`, `y`) of monitor `m`, through GDI (our ink layer is visible to it;
/// the toolbar isn't).
fn screen_px(m: &Monitor, x: i32, y: i32) -> Option<[u8; 3]> {
    let (x, y) = (m.rect.left + x, m.rect.top + y);
    overmark_win::win32::capture::gdi_screen(Rect::new(x, y, x + 1, y + 1)).map(|f| {
        let p = f.pixel(0, 0);
        [p[2], p[1], p[0]]
    })
}

/// Reads the screen at (`x`, `y`) until `want` holds of it, or a few seconds pass; the last
/// pixel read.
fn screen_until(m: &Monitor, x: i32, y: i32, want: impl Fn([u8; 3]) -> bool) -> Option<[u8; 3]> {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let p = screen_px(m, x, y);
        if p.is_some_and(&want) || Instant::now() >= deadline {
            return p;
        }
        window::pump();
        window::wait_for_message(5);
    }
}

fn stroke(ink: HWND, m: &Monitor) {
    drag(ink, m, (200, STROKE_Y), (800, STROKE_Y));
}

/// A mouse drag from `a` to `b` (monitor pixels), in 30 steps.
fn drag(ink: HWND, m: &Monitor, a: (i32, i32), b: (i32, i32)) {
    let n = 30;
    for k in 0..=n {
        let s = Sample {
            id: 1,
            kind: PointerKind::Mouse,
            x: m.rect.left + a.0 + (b.0 - a.0) * k / n,
            y: m.rect.top + a.1 + (b.1 - a.1) * k / n,
            pressure: None,
            tilt: None,
            eraser: false,
            in_contact: k < n,
            time_100ns: 0,
        };
        let phase = match k {
            0 => Phase::Down,
            _ if k == n => Phase::Up,
            _ => Phase::Move,
        };
        let (wp, lp) = pointer_message(&s, phase, false);
        window::post(ink, WM_OVERMARK_POINTER, wp, lp);
    }
}

fn px(img: &image::RgbaImage, x: i32, y: i32) -> [u8; 3] {
    let p = img.get_pixel(x as u32, y as u32).0;
    [p[0], p[1], p[2]]
}

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tol)
}

fn window_of(m: &Monitor, at: (i32, i32, i32, i32), colour: [u8; 3]) -> Window {
    let w = Window::new(
        Rect::new(
            m.rect.left + at.0,
            m.rect.top + at.1,
            m.rect.left + at.2,
            m.rect.top + at.3,
        ),
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .unwrap();
    w.fill_gdi(colour[0], colour[1], colour[2]);
    w.show(false);
    w.fill_gdi(colour[0], colour[1], colour[2]);
    w
}

fn field(line: &str, key: &str) -> Option<f64> {
    let at = line.find(&format!("{key}="))? + key.len() + 1;
    line[at..]
        .split(|c: char| c.is_whitespace())
        .next()?
        .parse()
        .ok()
}

struct Results(Vec<(String, bool, String)>);

impl Results {
    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        self.0.push((name.into(), ok, detail.into()));
    }
}

/// The toolbar's elements, read until `until` holds of them (the toolbar draws a frame, and
/// its tree, on its app's next turn) or a few seconds pass.
fn elements_until(uia: &Uia, bar: HWND, until: impl Fn(&[Element]) -> bool) -> Vec<Element> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let e = uia.elements(bar).unwrap_or_default();
        if until(&e) || Instant::now() >= deadline {
            return e;
        }
        window::pump();
        window::wait_for_message(20);
    }
}

fn named<'a>(e: &'a [Element], name: &str) -> Option<&'a Element> {
    e.iter().find(|e| e.name == name)
}

fn main() {
    // The app's windows are named in the UI language: look for them by the same names.
    overmark_core::i18n::init(&overmark_win::win32::language::ui_language());
    let _ = dpi::enable_per_monitor_v2();
    let exe = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|d| d.parent())
        .unwrap()
        .join("overmark.exe");
    assert!(exe.exists(), "build it first ({} missing)", exe.display());
    if overmark_win::win32::tray::is_running(overmark::APP_ID) {
        eprintln!("toolbar_check: Overmark is already running; quit it first (tray icon › Quit)");
        std::process::exit(2);
    }
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let captures = temp.path().join("captures");
    let mut settings = Settings::default();
    settings.capture.save_folder = captures.display().to_string();
    settings.capture.after_capture = AfterCapture::Save;
    settings.capture.toast = false;
    // Disappearing ink's shortest delay, for the toggle's step.
    settings.drawing.disappearing_ink_seconds = 0.5;
    let settings_file = appdata.join("Overmark").join("settings.toml");
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings.save(&settings_file).unwrap();
    let m = dpi::monitors()
        .into_iter()
        .find(|m| m.primary)
        .expect("a primary monitor");
    let behind = window_of(&m, (100, 0, m.rect.width() - 100, 800), BACK);
    let red = [
        DEFAULT_PALETTE[0].r,
        DEFAULT_PALETTE[0].g,
        DEFAULT_PALETTE[0].b,
    ];
    let ink_at = (500, STROKE_Y);
    let mut r = Results(Vec::new());
    let uia = Uia::new().expect("UI Automation");

    let mut app = App::start(&exe, &appdata);
    let (ink, bar, shown) = app.open(&m);
    let mut since = vec![field(&shown, "visible_ms").unwrap_or(f64::MAX)];
    r.add(
        "the toolbar's window is excluded from capture",
        affinity::is_excluded(bar.hwnd),
        "",
    );
    // What a screen reader finds.
    let names = [
        "Move the toolbar",
        "Backdrop",
        "Guides",
        "Tool",
        "Colour",
        "Disappearing ink",
        "Undo",
        "Redo",
        "Clear",
        "Save",
        "Save a region",
        "Copy",
        "More",
        "Freeze",
        "Collapse",
        "Close",
    ];
    let e = elements_until(&uia, bar.hwnd, |e| {
        names.iter().all(|n| named(e, n).is_some())
    });
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| named(&e, n).is_none())
        .collect();
    r.add(
        "a screen reader finds every control by its name",
        missing.is_empty(),
        if missing.is_empty() {
            format!("{} elements", e.len())
        } else {
            format!(
                "missing {missing:?}; found {:?}",
                e.iter().map(|e| &e.name).collect::<Vec<_>>()
            )
        },
    );
    let greyed = |e: &[Element], n: &str| named(e, n).is_some_and(|x| !x.enabled);
    // Read until it holds (or the deadline), as the tree a screen reader gets is the latest
    // frame's.
    let e = elements_until(&uia, bar.hwnd, |e| greyed(e, "Undo") && greyed(e, "Redo"));
    r.add(
        "Undo and Redo are greyed out with nothing to undo or redo",
        greyed(&e, "Undo") && greyed(&e, "Redo"),
        format!(
            "Undo {:?}, Redo {:?}",
            named(&e, "Undo").map(|x| x.enabled),
            named(&e, "Redo").map(|x| x.enabled)
        ),
    );
    // Where it's drawn it takes the pointer; elsewhere in its window the pointer goes through.
    let save_el = named(&e, "Save").cloned();
    let on_bar = save_el.as_ref().map(|s| {
        (
            (s.rect.left + s.rect.right) / 2,
            (s.rect.top + s.rect.bottom) / 2,
        )
    });
    let under = on_bar.map(|(x, y)| window::window_at(x, y));
    r.add(
        "the toolbar is the window under its buttons (a click there never draws)",
        under == Some(bar.hwnd),
        format!("{under:?} (the toolbar is {:?})", bar.hwnd),
    );
    let through = window::window_at((bar.rect.left + bar.rect.right) / 2, bar.rect.bottom - 10);
    r.add(
        "below it, in its window's empty part, the pointer goes to the ink layer",
        through == ink,
        format!("{through:?} (the ink layer is {ink:?})"),
    );
    // Resting on Save, its tooltip shows just below the toolbar; the toolbar's window
    // answers the system's hit test there (and a few pixels below its frame) "transparent", so
    // the pointer goes on to the ink layer and a press there draws; on Save it's the
    // toolbar's. Asked of the toolbar's window itself, while the tooltip shows: from another
    // process `WindowFromPoint` doesn't ask (a window of another thread is taken as opaque).
    let tip = |e: &[Element]| {
        e.iter()
            .find(|x| x.name.starts_with("Save the screen ("))
            .map(|t| t.rect)
    };
    let tooltip = on_bar.and_then(|(x, y)| {
        let (wp, lp) = panel_pointer(x - bar.rect.left, y - bar.rect.top, false);
        window::post(bar.hwnd, WM_PANEL_POINTER, wp, lp);
        tip(&elements_until(&uia, bar.hwnd, |e| tip(e).is_some()))
    });
    let under_tip =
        tooltip.map(|t| hit_test(bar.hwnd, (t.left + t.right) / 2, (t.top + t.bottom) / 2));
    let below_frame = save_el.as_ref().map(|s| {
        hit_test(
            bar.hwnd,
            (s.rect.left + s.rect.right) / 2,
            s.rect.bottom + 6 + 6,
        )
    });
    let on_save = on_bar.map(|(x, y)| hit_test(bar.hwnd, x, y));
    // The pointer away again: the tooltip goes.
    let (wp, lp) = panel_pointer(4, bar.rect.height() - 4, false);
    window::post(bar.hwnd, WM_PANEL_POINTER, wp, lp);
    r.add(
        "a tooltip below the toolbar, and a few pixels below its frame, let the pointer \
         through to the ink layer; Save takes it",
        tooltip.is_some()
            && under_tip == Some(Some(Hit::Through))
            && below_frame == Some(Some(Hit::Through))
            && on_save == Some(Some(Hit::Panel)),
        format!(
            "the tooltip {tooltip:?}: its hit test {under_tip:?}; 6 px below the frame \
             {below_frame:?}; on Save {on_save:?}"
        ),
    );

    // A stroke; then the toolbar's Undo, Redo, Clear, Freeze, saves.
    stroke(ink, &m);
    let e = elements_until(&uia, bar.hwnd, |e| {
        named(e, "Undo").is_some_and(|u| u.enabled)
    });
    r.add(
        "Undo is enabled once there's a stroke",
        named(&e, "Undo").is_some_and(|u| u.enabled),
        "",
    );
    let has_stroke = |img: &Option<image::RgbaImage>| {
        img.as_ref()
            .map(|i| near(px(i, ink_at.0, ink_at.1), red, 30))
    };
    let press = |name: &str| uia.invoke(bar.hwnd, name).unwrap_or(false);
    let step = |r: &mut Results, app: &App, name: &str, want_stroke: bool| {
        let pressed = press(name);
        let logged = app
            .wait_for("INFO toolbar action=", Duration::from_secs(3))
            .unwrap_or_default();
        let img = app.save(ink, Some(Capture::Save));
        r.add(
            &format!("{name}, pressed as a screen reader does, works as its key"),
            pressed && has_stroke(&img) == Some(want_stroke),
            format!(
                "pressed {pressed}; {logged}; the stroke in a save: {:?}",
                has_stroke(&img)
            ),
        );
    };
    step(&mut r, &app, "Undo", false);
    let _ = elements_until(&uia, bar.hwnd, |e| {
        named(e, "Redo").is_some_and(|u| u.enabled)
    });
    step(&mut r, &app, "Redo", true);
    step(&mut r, &app, "Clear", false);
    let _ = elements_until(&uia, bar.hwnd, |e| {
        named(e, "Undo").is_some_and(|u| u.enabled)
    });
    step(&mut r, &app, "Undo", true);
    // Save, from the toolbar.
    let pressed = press("Save");
    let img = app.save(ink, None);
    r.add(
        "Save saves, with the drawing",
        pressed && has_stroke(&img) == Some(true),
        format!("{:?}", has_stroke(&img)),
    );
    // More › Save without the drawing.
    let opened = press("More");
    let e = elements_until(&uia, bar.hwnd, |e| {
        named(e, "Save without the drawing").is_some()
    });
    let item = named(&e, "Save without the drawing").is_some();
    let pressed = press("Save without the drawing");
    let img = app.save(ink, None);
    r.add(
        "More › Save without the drawing saves without it",
        opened && item && pressed && has_stroke(&img) == Some(false),
        format!(
            "menu {opened}, item {item}, pressed {pressed}, stroke {:?}",
            has_stroke(&img)
        ),
    );
    // Freeze and Unfreeze.
    let pressed = press("Freeze");
    let frozen = app
        .wait_for(" F backdrop=\"Frozen\"", Duration::from_secs(3))
        .is_some();
    let e = elements_until(&uia, bar.hwnd, |e| named(e, "Unfreeze").is_some());
    r.add(
        "Freeze freezes, and the toolbar shows Frozen and Unfreeze",
        pressed && frozen && named(&e, "Unfreeze").is_some() && named(&e, "Frozen").is_some(),
        "",
    );
    let pressed = press("Unfreeze");
    let live = app
        .wait_for(" F backdrop=\"Live\"", Duration::from_secs(3))
        .is_some();
    r.add("Unfreeze lets the screen move again", pressed && live, "");
    // Each group's strip, pressed open as a screen reader does; a choice in the Tool
    // strip acts, and the strip stays open while drawing; the up arrow closes it. The width's
    // strip holds the width's slider: the arrows that come to the ink layer move it.
    let strip = |e: &[Element], group: &str| {
        let name = format!("{group} choices");
        e.iter().any(|x| x.name == name)
    };
    let mut opened = Vec::new();
    for group in ["Backdrop", "Guides", "Colour", "Tool"] {
        let pressed = press(group);
        let open = strip(&elements_until(&uia, bar.hwnd, |e| strip(e, group)), group);
        opened.push(format!("{group} {pressed}/{open}"));
    }
    let all_opened = opened.iter().all(|o| o.ends_with("true/true"));
    let picked = press("Arrow")
        && app
            .wait_for("toolbar action=Tool(Arrow)", Duration::from_secs(3))
            .is_some();
    // A stroke on the ink, then a save: the save's command comes after the stroke's samples on
    // the ink layer's queue, so a saved picture with the stroke in it proves the app handled
    // the stroke (and drew the toolbar's frame after it). Only then is the strip looked for.
    let drawn_at = (250, STROKE_Y + 100);
    drag(ink, &m, (200, drawn_at.1), (300, drawn_at.1));
    let img = app.save(ink, Some(Capture::Save));
    let drawn = img
        .as_ref()
        .is_some_and(|i| near(px(i, drawn_at.0, drawn_at.1), red, 30));
    let stayed = drawn
        && strip(
            &elements_until(&uia, bar.hwnd, |e| strip(e, "Tool")),
            "Tool",
        );
    // That arrow drew: undone, so the saves below have only the first stroke; back to the pen.
    let (wp, lp) = key_message(0x5A, true, false, false, false);
    window::post(ink, WM_OVERMARK_KEY, wp, lp);
    let _ = press("Pen");
    let closed = press("Close the choices")
        && !strip(
            &elements_until(&uia, bar.hwnd, |e| !strip(e, "Tool")),
            "Tool",
        );
    let width_of = |e: &[Element]| {
        e.iter()
            .find(|x| x.name.starts_with("Width: "))
            .map(|x| x.name.clone())
    };
    let e = elements_until(&uia, bar.hwnd, |e| width_of(e).is_some());
    let readout = width_of(&e).unwrap_or_default();
    let width_open = press(&readout)
        && strip(
            &elements_until(&uia, bar.hwnd, |e| strip(e, "Width")),
            "Width",
        );
    // Three notches of the wheel: 4 px to 5.
    for _ in 0..3 {
        window::post(ink, WM_OVERMARK_KEY, 0x27, 0);
    }
    let e = elements_until(&uia, bar.hwnd, |e| {
        width_of(e).is_some_and(|n| n != readout)
    });
    let wider = width_of(&e).unwrap_or_default();
    let _ = press("Close the choices");
    r.add(
        "each group opens its strip; a choice acts and it stays open while drawing; its arrow \
         closes it; the arrows move the width",
        all_opened && picked && stayed && closed && width_open && wider != readout,
        format!(
            "opened (pressed/open) {opened:?}, picked Arrow {picked}, the stroke in a save \
             {drawn}, open after it {stayed}, closed {closed}, width {width_open} {readout:?} \
             to {wider:?}"
        ),
    );
    // Save a region: the selection shows; Esc cancels it; the toolbar is on top again.
    let pressed = press("Save a region");
    let sel = app.window(Duration::from_secs(3), |w| {
        w.title == overmark_core::i18n::tr("window-selection-name") && w.visible
    });
    if let Some(s) = &sel {
        window::post(s.hwnd, WM_KEYDOWN, 0x1B, 0);
    }
    let cancelled = app
        .wait_for("region cancelled", Duration::from_secs(3))
        .is_some();
    let on_top = on_bar.is_some_and(|(x, y)| {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if window::window_at(x, y) == bar.hwnd {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            window::pump();
            window::wait_for_message(20);
        }
    });
    r.add(
        "Save a region starts a selection; after it the toolbar is on top again",
        pressed && sel.is_some() && cancelled && on_top,
        format!(
            "selection {}, cancelled {cancelled}, on top {on_top}",
            sel.is_some()
        ),
    );

    // A share sees the ink, not the toolbar.
    let (tx, ty) = on_bar.unwrap_or((0, 0));
    let probes = vec![
        (tx - m.rect.left, ty - m.rect.top),
        (ink_at.0, ink_at.1),
        (11, 11),
    ];
    let mark = window_of(&m, (8, 8, 14, 14), [0, 255, 0]);
    let observer = Observer::start(&m, probes, false, false).expect("a capture of the monitor");
    mark.show(false);
    mark.fill_gdi(255, 0, 255);
    let pump = || {
        window::pump();
        window::wait_for_message(5);
    };
    let seen = frame_after_barrier(
        || observer.take_frames(),
        2,
        [255, 0, 255, 255],
        SEEN_WITHIN_MS,
        pump,
    )
    .map(|f| (f.probes[0], f.probes[1]))
    .map(|(a, b)| ([a[2], a[1], a[0]], [b[2], b[1], b[0]]));
    r.add(
        "a share sees the ink, and the screen where the toolbar is (not the toolbar)",
        seen.is_some_and(|(bar_px, ink_px)| near(bar_px, BACK, 3) && near(ink_px, red, 30)),
        format!(
            "at the toolbar {:?} (the screen is {BACK:?}); at the stroke {:?}",
            seen.map(|s| s.0),
            seen.map(|s| s.1)
        ),
    );
    drop(observer);
    drop(mark);

    // Collapse and expand.
    let pressed = press("Collapse");
    let e = elements_until(&uia, bar.hwnd, |e| named(e, "Expand").is_some());
    let pill = named(&e, "Expand").is_some() && named(&e, "Undo").is_none();
    let expanded = press("Expand")
        && named(
            &elements_until(&uia, bar.hwnd, |e| named(e, "Undo").is_some()),
            "Undo",
        )
        .is_some();
    r.add(
        "Collapse makes a pill; Expand brings the toolbar back",
        pressed && pill && expanded,
        format!("pill {pill}, expanded {expanded}"),
    );

    // Disappearing ink, Settings' own setting: on from the toolbar, saved; then off again.
    let on = press("Disappearing ink")
        && app
            .wait_for("DisappearingInk(true)", Duration::from_secs(3))
            .is_some();
    // The log says the press first, then the file is written: read until it's there.
    let saved_on = {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let has = std::fs::read_to_string(&settings_file)
                .unwrap_or_default()
                .contains("disappearing_ink = true");
            if has || Instant::now() >= deadline {
                break has;
            }
            window::pump();
            window::wait_for_message(20);
        }
    };
    // A fresh stroke fades (0.5 s, then 600 ms); caught partly faded on screen, the toggle
    // turns it off, and the stroke must come back whole on screen (not stay half drawn).
    let fade_y = 700;
    drag(ink, &m, (200, fade_y), (800, fade_y));
    let partly = screen_until(&m, 500, fade_y, |p| !near(p, red, 30) && !near(p, BACK, 10));
    let caught = partly.is_some_and(|p| !near(p, red, 30) && !near(p, BACK, 10));
    let off = press("Disappearing ink")
        && app
            .wait_for("DisappearingInk(false)", Duration::from_secs(3))
            .is_some();
    let whole = screen_until(&m, 500, fade_y, |p| near(p, red, 30));
    r.add(
        "Disappearing ink turns on and off from the toolbar, and Settings have it",
        on && saved_on && off,
        format!("on {on}, saved {saved_on}, off {off}"),
    );
    r.add(
        "turned off mid-fade, the stroke is drawn whole again on screen",
        caught && whole.is_some_and(|p| near(p, red, 30)),
        format!("partly faded {partly:?}, then {whole:?} (red is {red:?})"),
    );

    // Nothing moving: no CPU (a span of time, the one thing measured over one).
    while app.lines.try_recv().is_ok() {}
    let p = Process::open(app.owned.pid()).expect("the app's process");
    let cpu0 = p.cpu_seconds();
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    let cpu_ms = (p.cpu_seconds() - cpu0) * 1000.0;
    r.add(
        "no CPU with the toolbar open and nothing moving",
        cpu_ms < 20.0,
        format!("{cpu_ms:.1} ms in 1 s"),
    );

    // Dragged by its grip, 200 px right and 150 down; it opens there again.
    let e = uia.elements(bar.hwnd).unwrap_or_default();
    let grip = named(&e, "Move the toolbar").map(|g| {
        (
            (g.rect.left + g.rect.right) / 2 - bar.rect.left,
            (g.rect.top + g.rect.bottom) / 2 - bar.rect.top,
        )
    });
    let moved = grip.and_then(|(gx, gy)| {
        for (x, y, down) in [(gx, gy, true), (gx + 200, gy + 150, true), (gx, gy, false)] {
            let (wp, lp) = panel_pointer(x, y, down);
            window::post(bar.hwnd, WM_PANEL_POINTER, wp, lp);
        }
        app.wait_for("toolbar moved", Duration::from_secs(3))
    });
    app.close(ink);
    let (ink2, bar2, shown) = app.open(&m);
    since.push(field(&shown, "visible_ms").unwrap_or(f64::MAX));
    let saved = std::fs::read_to_string(&settings_file).unwrap_or_default();
    r.add(
        "dragged by its grip, it opens where it was left; Settings have it",
        moved.is_some()
            && (bar2.rect.left - bar.rect.left, bar2.rect.top - bar.rect.top) == (200, 150)
            && saved.contains("[toolbar.positions]"),
        format!(
            "{moved:?}; first at ({}, {}), now at ({}, {})",
            bar.rect.left, bar.rect.top, bar2.rect.left, bar2.rect.top
        ),
    );
    // Close, from the toolbar.
    let pressed = uia.invoke(bar2.hwnd, "Close").unwrap_or(false);
    let closed = app.wait_for("closed", Duration::from_secs(3)).is_some();
    r.add("Close closes the layer", pressed && closed, "");
    let _ = ink2;
    // The budget, over more openings.
    while since.len() < OPENS {
        let (ink, _, shown) = app.open(&m);
        since.push(field(&shown, "visible_ms").unwrap_or(f64::MAX));
        app.close(ink);
    }
    let mut sorted = since.clone();
    sorted.sort_by(f64::total_cmp);
    r.add(
        &format!("hotkey to the layer and toolbar on screen within 100 ms ({OPENS} opens)"),
        sorted.last().is_some_and(|&ms| ms <= 100.0),
        format!(
            "min {:.1}, median {:.1}, max {:.1} ms",
            sorted[0],
            sorted[sorted.len() / 2],
            sorted[sorted.len() - 1]
        ),
    );
    let quit = app.owned.stop(&exe);
    drop(behind);

    println!();
    let mut failed = 0;
    for (name, ok, detail) in &r.0 {
        println!(
            "{} {name}{}{detail}",
            if *ok { "ok    " } else { "FAILED" },
            if detail.is_empty() { "" } else { ": " }
        );
        failed += usize::from(!ok);
    }
    if !quit {
        println!("FAILED it didn't quit when asked");
        failed += 1;
    }
    println!(
        "\ntoolbar_check: {}",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
