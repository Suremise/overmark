//! Checks drawing on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example draw_check
//! ```
//!
//! It starts `overmark.exe` with `%APPDATA%` and the captures folder in temp folders of its own,
//! over a backdrop window of a known colour (with a green stripe in it, for the blur), opens the
//! layer through the app's own hotkey window, and draws with every tool through the ink layer's
//! own test messages: `WM_OVERMARK_KEY` for keys (with their modifiers), `WM_OVERMARK_POINTER`
//! for pointer samples (a mouse's, or a pen's with pressure and its eraser end), `WM_CHAR` for
//! typing and `WM_MOUSEWHEEL` for the wheel. They take the same path as real input once it's
//! read; nothing global is sent. Each step is judged from a saved capture (`Ctrl+S`), read back:
//!
//! - every tool draws what it should where it should (pen with the mouse and with a pen, the
//!   highlighter, arrow, rectangle and its Shift square, ellipse, line and its Shift snap, text
//!   on two lines, a stamp, blur over the frozen frame, the eraser, the pen's eraser end), the
//!   laser leaves nothing, a palette key picks its colour and the wheel with a tool key held
//!   widens the tool;
//! - B on Live is refused with a notice; F does nothing while typing;
//! - Delete clears, and undoes; 110 strokes undo 100 deep and redo again;
//! - with disappearing ink, a stroke fades away by itself, and once nothing moves the app goes
//!   back to waiting (no CPU);
//! - the focus goes back to the window that had it when the layer closes;
//! - the timings are logged on close, and redraws take less than a frame.
//!
//! Run it with no Overmark running (it stops if one is), when you're not typing: the layer
//! takes the screen and the focus for a few seconds. There's no pen on the build PC: the pen's
//! samples here are the pen's path from the moment it's read; a real pen is on the release
//! checklist (docs/RELEASING.md).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark_core::colour::Colour;
use overmark_core::hotkey::Hotkey;
use overmark_core::settings::{AfterCapture, DEFAULT_PALETTE, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::overlay::{
    Command as Capture, Phase, WM_OVERMARK_COMMAND, WM_OVERMARK_KEY, WM_OVERMARK_POINTER,
    key_message, pointer_message,
};
use overmark_win::win32::Rect;
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::pointer::{PointerKind, Sample};
use overmark_win::win32::process::Process;
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_ERASEBKGND, WM_HOTKEY, WM_MOUSEWHEEL};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// The backdrop: RGB (200, 100, 50).
const BACK: [u8; 3] = [200, 100, 50];
/// The stripe the blur is drawn over.
const STRIPE: [u8; 3] = [20, 200, 20];
/// Where the backdrop is, on the primary monitor (its pixels from the top-left).
const AREA: (i32, i32, i32, i32) = (100, 100, 1500, 1060);
/// The stripe: x 1000 to 1006, y 300 to 500.
const STRIPE_AT: (i32, i32, i32, i32) = (1000, 300, 1006, 500);

/// The app this check started, stopped however the check ends (as `capture_check`'s).
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
                    "draw_check: stopping the Overmark it started (pid {})",
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
    /// Every line so far (for the timings and what came before a marker).
    log: Vec<String>,
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
        let mut app = App {
            owned: Owned(Some(child)),
            lines,
            log: Vec::new(),
        };
        assert!(
            app.wait_for("resident in the tray", Duration::from_secs(10))
                .is_some(),
            "the app didn't start"
        );
        app
    }

    fn wait_for(&mut self, needle: &str, timeout: Duration) -> Option<String> {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            match self.lines.recv_timeout(Duration::from_millis(20)) {
                Ok(line) => {
                    self.log.push(line.clone());
                    if line.contains(needle) {
                        return Some(line);
                    }
                }
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

    /// Opens the layer with the Draw hotkey, through the app's own hotkey window.
    fn open(&mut self, m: &Monitor) -> Ink {
        let hotkeys = self
            .window(Duration::from_secs(5), |w| w.class == "global_hotkey_app")
            .expect("the hotkey window");
        let id = to_hotkey(&Hotkey::parse("Win+Shift+D").unwrap().unwrap()).id() as usize;
        window::post(
            hotkeys.hwnd,
            WM_HOTKEY,
            id,
            ((0x44 << 16) | 0x0008 | 0x0004) as isize,
        );
        self.wait_for(" opened", Duration::from_secs(3))
            .expect("the layer opened");
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
        Ink {
            hwnd: ink.hwnd,
            origin: (m.rect.left, m.rect.top),
        }
    }

    /// Saves a capture (Ctrl+S) and reads it back.
    fn save(&mut self, ink: &Ink) -> Option<image::RgbaImage> {
        window::post(ink.hwnd, WM_OVERMARK_COMMAND, Capture::Save.wparam(), 0);
        let line = self.wait_for(" delivered", Duration::from_secs(10))?;
        let file = saved_file(&line)?;
        Some(
            image::load_from_memory(&std::fs::read(file).ok()?)
                .ok()?
                .to_rgba8(),
        )
    }
}

/// The ink layer, driven through its test messages.
struct Ink {
    hwnd: windows::Win32::Foundation::HWND,
    origin: (i32, i32),
}

impl Ink {
    fn key_with(&self, vk: u32, ctrl: bool, shift: bool) {
        let (wp, lp) = key_message(vk, ctrl, shift, false, false);
        window::post(self.hwnd, WM_OVERMARK_KEY, wp, lp);
        let (wp, lp) = key_message(vk, ctrl, shift, false, true);
        window::post(self.hwnd, WM_OVERMARK_KEY, wp, lp);
    }
    fn key(&self, c: char) {
        self.key_with(c as u32, false, false);
    }
    fn hold(&self, c: char, down: bool) {
        let (wp, lp) = key_message(c as u32, false, false, false, !down);
        window::post(self.hwnd, WM_OVERMARK_KEY, wp, lp);
    }
    fn type_text(&self, s: &str) {
        // As Windows sends them: UTF-16 code units, a character beyond U+FFFF as two.
        for u in s.encode_utf16() {
            window::post(self.hwnd, WM_CHAR, usize::from(u), 0);
        }
    }
    fn wheel(&self, notches: i32) {
        let delta = (notches * 120) as i16 as u16 as usize;
        window::post(self.hwnd, WM_MOUSEWHEEL, delta << 16, 0);
    }
    fn sample(&self, kind: PointerKind, x: i32, y: i32, contact: bool) -> Sample {
        Sample {
            id: 1,
            kind,
            x: self.origin.0 + x,
            y: self.origin.1 + y,
            pressure: (kind == PointerKind::Pen).then_some(0.8),
            tilt: None,
            eraser: false,
            in_contact: contact,
            time_100ns: 0,
        }
    }
    fn post(&self, s: &Sample, phase: Phase, shift: bool) {
        let (wp, lp) = pointer_message(s, phase, shift);
        window::post(self.hwnd, WM_OVERMARK_POINTER, wp, lp);
    }
    /// A drag through `points` (layer pixels), touching from the first to the last.
    fn drag_with(&self, kind: PointerKind, eraser: bool, shift: bool, points: &[(i32, i32)]) {
        let n = points.len();
        for (k, &(x, y)) in points.iter().enumerate() {
            let mut s = self.sample(kind, x, y, k + 1 < n);
            s.eraser = eraser;
            let phase = match k {
                0 => Phase::Down,
                _ if k + 1 == n => Phase::Up,
                _ => Phase::Move,
            };
            self.post(&s, phase, shift);
        }
    }
    fn drag(&self, points: &[(i32, i32)]) {
        self.drag_with(PointerKind::Mouse, false, false, points);
    }
    fn click(&self, x: i32, y: i32) {
        self.drag(&[(x, y), (x, y)]);
    }
    /// Hovering (no contact): the laser follows.
    fn hover(&self, points: &[(i32, i32)]) {
        for &(x, y) in points {
            self.post(
                &self.sample(PointerKind::Mouse, x, y, false),
                Phase::Move,
                false,
            );
        }
    }
}

/// Points from (`x0`, `y0`) to (`x1`, `y1`), every 10 px.
fn line(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let n = ((x1 - x0).abs().max((y1 - y0).abs()) / 10).max(1);
    (0..=n)
        .map(|i| (x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n))
        .collect()
}

/// The path in a "delivered" line's `file=Some("…")`.
fn saved_file(line: &str) -> Option<PathBuf> {
    let at = line.find("file=Some(\"")? + "file=Some(\"".len();
    let end = line[at..].find("\")")?;
    Some(PathBuf::from(line[at..at + end].replace("\\\\", "\\")))
}

fn px(img: &image::RgbaImage, x: i32, y: i32) -> [u8; 3] {
    let p = img.get_pixel(x as u32, y as u32).0;
    [p[0], p[1], p[2]]
}

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tol)
}

/// Whether anything but the backdrop is in the rectangle (x0..x1, y0..y1).
fn inked(img: &image::RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| !near(px(img, x, y), BACK, 6)))
}

/// How many pixels of column `x` from `y0` to `y1` aren't the backdrop.
fn thickness(img: &image::RgbaImage, x: i32, y0: i32, y1: i32) -> usize {
    (y0..y1).filter(|&y| !near(px(img, x, y), BACK, 6)).count()
}

fn rgb(c: Colour) -> [u8; 3] {
    [c.r, c.g, c.b]
}

struct Results(Vec<(String, bool, String)>);

impl Results {
    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        self.0.push((name.into(), ok, detail.into()));
    }
}

fn settings_file(appdata: &Path, captures: &Path, fade: bool) {
    let mut settings = Settings::default();
    settings.capture.save_folder = captures.display().to_string();
    settings.capture.after_capture = AfterCapture::Save;
    settings.capture.toast = false;
    settings.drawing.disappearing_ink = fade;
    settings.drawing.disappearing_ink_seconds = 0.5;
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings
        .save(&appdata.join("Overmark").join("settings.toml"))
        .unwrap();
}

fn backdrop(m: &Monitor, at: (i32, i32, i32, i32), colour: [u8; 3]) -> Window {
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

/// The `key=value` of a log line, as a number.
fn field(line: &str, key: &str) -> Option<f64> {
    let at = line.find(&format!("{key}="))? + key.len() + 1;
    line[at..]
        .split(|c: char| c.is_whitespace())
        .next()?
        .parse()
        .ok()
}

/// The tools, one by one, over Live and then Frozen.
fn tools(app: &mut App, ink: &Ink, m: &Monitor, r: &mut Results) {
    let red = rgb(DEFAULT_PALETTE[0]);
    // Pen, with the mouse (the default tool and colour), and with a pen's pressure.
    ink.key('P');
    ink.drag(&line(200, 200, 500, 200));
    ink.drag_with(PointerKind::Pen, false, false, &line(200, 240, 500, 240));
    // Highlighter.
    ink.key('H');
    ink.drag(&line(200, 280, 500, 280));
    // Arrow.
    ink.key('A');
    ink.drag(&line(200, 320, 500, 320));
    // Rectangle, and a square with Shift.
    ink.key('R');
    ink.drag(&line(200, 360, 500, 440));
    ink.drag_with(PointerKind::Mouse, false, true, &line(600, 360, 760, 420));
    // Ellipse.
    ink.key('E');
    ink.drag(&line(200, 470, 500, 550));
    // Line, and one snapped flat with Shift.
    ink.key('L');
    ink.drag(&line(200, 590, 500, 590));
    ink.drag_with(PointerKind::Mouse, false, true, &line(600, 590, 800, 600));
    // Text on two lines; F typed is text, not Frozen.
    ink.key('T');
    ink.click(200, 630);
    ink.key('H');
    ink.type_text("Hi😀");
    ink.key_with(0x0D, false, true); // Shift+Enter
    ink.key('F');
    ink.type_text("F");
    ink.key_with(0x0D, false, false); // Enter
    // A stamp.
    ink.key('N');
    ink.click(650, 680);
    // The laser: hovering, then touching; it leaves nothing.
    ink.key('Z');
    ink.hover(&line(900, 700, 1200, 700));
    ink.drag(&line(900, 740, 1200, 740));
    // The pen's eraser end, through a pen stroke; the pen stays the tool.
    ink.key('P');
    ink.drag(&line(200, 780, 500, 780));
    ink.drag_with(PointerKind::Pen, true, false, &line(350, 760, 350, 800));
    ink.drag(&line(200, 820, 500, 820));
    // A palette key, and the wheel with the tool key held: a wider blue pen.
    ink.key('5');
    ink.drag(&line(600, 820, 900, 820));
    ink.hold('P', true);
    ink.wheel(10);
    ink.hold('P', false);
    ink.drag(&line(600, 880, 900, 880));
    // The eraser, through the plain line.
    ink.key('X');
    ink.drag(&line(350, 570, 350, 610));
    // Blur on Live: refused, with a notice.
    ink.key('B');
    let notice = app.window(Duration::from_secs(3), |w| {
        w.title == overmark_core::i18n::tr("window-notice-name") && w.visible
    });
    r.add(
        "B on Live is refused, with a notice",
        notice.is_some(),
        if notice.is_some() {
            "shown"
        } else {
            "no notice"
        },
    );
    ink.key('P');
    let Some(img) = app.save(ink) else {
        r.add("Live: a capture", false, "no file");
        return;
    };
    let before: Vec<&String> = app
        .log
        .iter()
        .rev()
        .take_while(|l| !l.contains(" opened"))
        .collect();
    r.add(
        "F does nothing while typing",
        !before.iter().any(|l| l.contains("backdrop=Frozen")),
        "",
    );
    let check = |r: &mut Results, name: &str, ok: bool, detail: String| r.add(name, ok, detail);
    check(
        r,
        "pen (mouse), in the first palette colour",
        near(px(&img, 350, 200), red, 30),
        format!("{:?} (want about {:?})", px(&img, 350, 200), red),
    );
    check(
        r,
        "pen (a pen's samples, pressure 0.8)",
        inked(&img, 340, 230, 360, 250),
        String::new(),
    );
    check(
        r,
        "highlighter",
        inked(&img, 340, 270, 360, 290),
        format!("{:?}", px(&img, 350, 280)),
    );
    check(r, "arrow", inked(&img, 340, 315, 360, 325), String::new());
    check(
        r,
        "rectangle: its outline, not its inside",
        inked(&img, 340, 355, 360, 365) && !inked(&img, 300, 380, 400, 420),
        String::new(),
    );
    // Shift: dragged 160 across and 60 down, a 160 px square (its bottom at 520, not 420).
    check(
        r,
        "Shift makes a square",
        inked(&img, 670, 515, 690, 525) && !inked(&img, 670, 410, 690, 430),
        String::new(),
    );
    check(
        r,
        "ellipse: its outline, not its inside",
        inked(&img, 340, 465, 360, 475) && !inked(&img, 320, 495, 380, 525),
        String::new(),
    );
    check(
        r,
        "Shift snaps a line flat",
        inked(&img, 780, 586, 790, 594) && !inked(&img, 780, 597, 790, 606),
        String::new(),
    );
    check(
        r,
        "text, on two lines",
        inked(&img, 200, 630, 300, 655) && inked(&img, 200, 665, 300, 700),
        String::new(),
    );
    check(r, "stamp", inked(&img, 640, 670, 660, 690), String::new());
    check(
        r,
        "the laser leaves nothing",
        !inked(&img, 900, 690, 1210, 750),
        String::new(),
    );
    check(
        r,
        "the pen's eraser end erases the stroke, and the pen still draws",
        !inked(&img, 200, 770, 500, 790) && inked(&img, 340, 815, 360, 825),
        String::new(),
    );
    let blue = rgb(DEFAULT_PALETTE[4]);
    let (thin, wide) = (
        thickness(&img, 750, 800, 850),
        thickness(&img, 750, 850, 920),
    );
    check(
        r,
        "key 5 picks the fifth colour",
        near(px(&img, 750, 880), blue, 30),
        format!("{:?} (want about {:?})", px(&img, 750, 880), blue),
    );
    check(
        r,
        "the wheel with P held widens the pen",
        thin > 0 && wide as f64 >= thin as f64 * 1.5,
        format!("{thin} px, then {wide} px"),
    );
    check(
        r,
        "the eraser erases the line whole",
        !inked(&img, 200, 580, 500, 600),
        String::new(),
    );
    // On screen: the stroke is shown on the layer, not only in the drawing.
    let at = (m.rect.left + 350, m.rect.top + 200);
    let shown = overmark_win::win32::capture::gdi_screen(Rect::new(at.0, at.1, at.0 + 1, at.1 + 1))
        .map(|f| {
            let p = f.pixel(0, 0);
            [p[2], p[1], p[0]]
        });
    check(
        r,
        "the ink is on screen",
        shown.is_some_and(|p| near(p, red, 30)),
        format!("{shown:?}"),
    );

    // Frozen: blur over the stripe pixelates it, in the capture and on screen.
    ink.key('F');
    app.wait_for(" F", Duration::from_secs(3)).expect("F");
    ink.key('B');
    ink.drag(&line(970, 320, 1040, 480));
    ink.key('P');
    let Some(img) = app.save(ink) else {
        r.add("Frozen: a capture", false, "no file");
        return;
    };
    let y = 400;
    let mixed = (970..1040)
        .map(|x| px(&img, x, y))
        .find(|p| !near(*p, BACK, 8) && !near(*p, STRIPE, 8));
    r.add(
        "Frozen: blur pixelates what's under it",
        mixed.is_some(),
        format!("a mixed block {mixed:?}"),
    );
    let blur_x = (970..1040).find(|&x| {
        let p = px(&img, x, y);
        !near(p, BACK, 8) && !near(p, STRIPE, 8)
    });
    let screen = blur_x.and_then(|x| {
        let at = (m.rect.left + x, m.rect.top + y);
        overmark_win::win32::capture::gdi_screen(Rect::new(at.0, at.1, at.0 + 1, at.1 + 1)).map(
            |f| {
                let p = f.pixel(0, 0);
                [p[2], p[1], p[0]]
            },
        )
    });
    r.add(
        "Frozen: the blur shows on screen as saved",
        matches!((mixed, screen), (Some(a), Some(b)) if near(a, b, 4)),
        format!("saved {mixed:?}, on screen {screen:?}"),
    );
    ink.key('F');
    app.wait_for(" F", Duration::from_secs(3)).expect("F back");
}

/// Delete, undo and redo, and 110 strokes 100 deep.
fn history(app: &mut App, ink: &Ink, r: &mut Results) {
    ink.key_with(0x2E, false, false); // Delete
    let cleared = app.save(ink);
    ink.key_with('Z' as u32, true, false);
    let back = app.save(ink);
    ink.key_with('Y' as u32, true, false);
    let again = app.save(ink);
    let has = |img: &Option<image::RgbaImage>| {
        img.as_ref()
            .map(|i| inked(i, 340, 195, 360, 205) && inked(i, 640, 670, 660, 690))
    };
    r.add(
        "Delete clears; Ctrl+Z brings it back; Ctrl+Y clears it again",
        has(&cleared) == Some(false) && has(&back) == Some(true) && has(&again) == Some(false),
        format!(
            "after Delete {:?}, Ctrl+Z {:?}, Ctrl+Y {:?}",
            has(&cleared),
            has(&back),
            has(&again)
        ),
    );
    // 110 strokes, each its own step: 22 a row, five rows.
    let at = |k: i32| (200 + (k % 22) * 50, 940 + (k / 22) * 20);
    for k in 0..110 {
        let (x, y) = at(k);
        ink.drag(&line(x, y, x + 30, y));
    }
    for _ in 0..100 {
        ink.key_with('Z' as u32, true, false);
    }
    let undone = app.save(ink);
    for _ in 0..50 {
        ink.key_with('Y' as u32, true, false);
    }
    for _ in 0..50 {
        ink.key_with('Z' as u32, true, true); // Ctrl+Shift+Z
    }
    let redone = app.save(ink);
    let drawn = |img: &image::RgbaImage| -> Vec<bool> {
        (0..110)
            .map(|k| {
                let (x, y) = at(k);
                inked(img, x + 10, y - 3, x + 20, y + 3)
            })
            .collect()
    };
    match (undone, redone) {
        (Some(u), Some(d)) => {
            let u = drawn(&u);
            let d = drawn(&d);
            let first = u.iter().take(10).filter(|b| **b).count();
            let rest = u.iter().skip(10).filter(|b| **b).count();
            let all = d.iter().filter(|b| **b).count();
            r.add(
                "100 undos take back the last 100 of 110 strokes",
                first == 10 && rest == 0,
                format!("{first} of the first 10 left, {rest} of the last 100"),
            );
            r.add(
                "Ctrl+Y and Ctrl+Shift+Z redo all 100",
                all == 110,
                format!("{all} of 110"),
            );
        }
        _ => r.add("undo 100 deep", false, "no capture"),
    }
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
        eprintln!("draw_check: Overmark is already running; quit it first (tray icon › Quit)");
        std::process::exit(2);
    }
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let captures = temp.path().join("captures");
    settings_file(&appdata, &captures, false);
    let m = dpi::monitors()
        .into_iter()
        .find(|m| m.primary)
        .expect("a primary monitor");
    let back = backdrop(&m, AREA, BACK);
    let stripe = backdrop(&m, STRIPE_AT, STRIPE);
    let mut r = Results(Vec::new());

    let focus_before = window::foreground();
    let mut app = App::start(&exe, &appdata);
    let ink = app.open(&m);
    let fonts = app
        .wait_for("text fonts loaded", Duration::from_secs(5))
        .is_some();
    r.add("the system's text fonts loaded", fonts, "");
    tools(&mut app, &ink, &m, &mut r);
    history(&mut app, &ink, &mut r);
    window::post(ink.hwnd, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    // Windows completes a focus change on its own time: read until it's back, or a deadline.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut focus_after = window::foreground();
    while focus_after != focus_before && Instant::now() < deadline {
        window::pump();
        window::wait_for_message(10);
        focus_after = window::foreground();
    }
    r.add(
        "the focus goes back on close",
        focus_before.is_some() && focus_after == focus_before,
        format!("before {focus_before:?}, after {focus_after:?}"),
    );
    let timings: Vec<String> = app
        .log
        .iter()
        .filter(|l| l.contains("timings"))
        .cloned()
        .collect();
    // The samples here are posted hundreds at once, so their age when shown is mostly the
    // queue behind them (a real pointer's come a few at a time): it's reported, not judged.
    // What the app controls is the redraw: within a frame.
    let latency = timings.iter().find(|l| l.contains("input latency"));
    r.add(
        "input latency logged (posted in bursts: reported, not judged)",
        latency.is_some(),
        latency.cloned().unwrap_or_else(|| "not logged".into()),
    );
    let redraw = timings.iter().find(|l| l.contains("redraw"));
    let p95 = redraw.and_then(|l| field(l, "p95_ms"));
    r.add(
        "redraws take less than a frame (95th percentile)",
        p95.is_some_and(|ms| ms <= 16.0),
        redraw.cloned().unwrap_or_else(|| "not logged".into()),
    );
    let quit = app.owned.stop(&exe);

    // Disappearing ink: a stroke fades by itself, then nothing runs.
    settings_file(&appdata, &captures, true);
    let mut app = App::start(&exe, &appdata);
    let ink = app.open(&m);
    ink.drag(&line(200, 200, 500, 200));
    let first = app.save(&ink);
    let faded = app.wait_for(" still", Duration::from_secs(5)).is_some();
    let after = app.save(&ink);
    r.add(
        "disappearing ink: drawn, then gone by itself",
        first.as_ref().is_some_and(|i| inked(i, 340, 195, 360, 205))
            && faded
            && after
                .as_ref()
                .is_some_and(|i| !inked(i, 200, 190, 500, 210)),
        format!("animation stopped: {faded}"),
    );
    // Nothing moves now: the app waits with no timeout. (A span of time, the one thing that
    // has to be measured over one.)
    let p = Process::open(app.owned.pid()).expect("open the app's process");
    let cpu0 = p.cpu_seconds();
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        while app.lines.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(100));
    }
    let cpu_ms = (p.cpu_seconds() - cpu0) * 1000.0;
    r.add(
        "no CPU with the layer open and nothing moving",
        cpu_ms < 20.0,
        format!("{cpu_ms:.1} ms of CPU in 1 s"),
    );
    window::post(ink.hwnd, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    let quit2 = app.owned.stop(&exe);
    drop(stripe);
    drop(back);

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
    if !(quit && quit2) {
        println!("FAILED it didn't quit when asked");
        failed += 1;
    }
    println!(
        "\ndraw_check: {}",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
