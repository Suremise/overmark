//! Checks the backdrops and guides on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example backdrop_check
//! ```
//!
//! It starts `overmark.exe` with `%APPDATA%` and the captures folder in temp folders of its own,
//! over a window of a known colour, opens the layer through the app's own hotkey window, draws a
//! stroke, and then presses W, Shift+W, G, Shift+G and F as `WM_OVERMARK_KEY` messages posted to
//! the ink layer (a posted `WM_KEYDOWN` can't say Shift is down; it's the same path once read).
//! Nothing global is sent. It checks:
//!
//! - each key's backdrop and guides, both ways round, from the app's log;
//! - a save over each backdrop: the screen (Live), the frozen frame (Frozen), white, the
//!   blackboard's grey, each with the stroke; the grid in a save;
//! - **the frozen frame is kept** across the boards: the window behind changes colour while on
//!   the whiteboard, and W back to Frozen still shows the old one; Live then shows the new;
//! - no dim over a board (the veil window is hidden);
//! - **what a share sees** (Windows Graphics Capture, judged after a barrier): the whiteboard,
//!   and the grid on it; then, with "Show guides on screen shares" off, the whiteboard without
//!   the grid, which a save still has;
//! - an optional board hotkey from Settings opens the layer straight onto the whiteboard.
//!
//! Run it with no Overmark running (it stops if one is), when you're not typing: the layer
//! takes the screen and the focus for a few seconds.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark_core::hotkey::Hotkey;
use overmark_core::settings::{AfterCapture, DEFAULT_BLACKBOARD, DEFAULT_PALETTE, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::overlay::{
    Command as Capture, Phase, WM_OVERMARK_COMMAND, WM_OVERMARK_KEY, WM_OVERMARK_POINTER,
    key_message, pointer_message,
};
use overmark_win::win32::Rect;
use overmark_win::win32::affinity;
use overmark_win::win32::capture::wgc::{Observer, frame_after_barrier};
use overmark_win::win32::dpi::{self, Monitor};
use overmark_win::win32::pointer::{PointerKind, Sample};
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{WM_ERASEBKGND, WM_HOTKEY};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// The window behind: RGB (200, 100, 50), then (40, 160, 90) once it changes.
const BACK: [u8; 3] = [200, 100, 50];
const BACK2: [u8; 3] = [40, 160, 90];
/// Where it is on the primary monitor, and the stroke drawn over it.
const AREA: (i32, i32, i32, i32) = (100, 100, 900, 800);
const STROKE_Y: i32 = 600;
/// The board hotkey the check sets.
const BOARD_HOTKEY: &str = "Win+Shift+F11";
/// How long a share may take to show a change.
const SEEN_WITHIN_MS: u32 = 3000;

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
                    "backdrop_check: stopping the Overmark it started (pid {})",
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
    fn start(exe: &Path, appdata: &Path, args: &[&str]) -> App {
        let mut child = Command::new(exe)
            .args(args)
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

    /// Opens the layer with `hotkey` through the app's own hotkey window; the `opened` line.
    fn open(&self, m: &Monitor, hotkey: &str) -> (HWND, String) {
        let hotkeys = self
            .window(Duration::from_secs(5), |w| w.class == "global_hotkey_app")
            .expect("the hotkey window");
        let h = Hotkey::parse(hotkey).unwrap().unwrap();
        let id = to_hotkey(&h).id() as usize;
        window::post(hotkeys.hwnd, WM_HOTKEY, id, 0);
        let line = self
            .wait_for(" opened", Duration::from_secs(3))
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
        (ink.hwnd, line)
    }

    /// A key to the ink layer, and the log line it leads to (`needle`).
    fn key(&self, ink: HWND, vk: char, shift: bool, needle: &str) -> Option<String> {
        let (wp, lp) = key_message(vk as u32, false, shift, false, false);
        window::post(ink, WM_OVERMARK_KEY, wp, lp);
        let (wp, lp) = key_message(vk as u32, false, shift, false, true);
        window::post(ink, WM_OVERMARK_KEY, wp, lp);
        self.wait_for(needle, Duration::from_secs(5))
    }

    fn save(&self, ink: HWND) -> Option<image::RgbaImage> {
        window::post(ink, WM_OVERMARK_COMMAND, Capture::Save.wparam(), 0);
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

/// The saved pixel at `at` (monitor pixels), and the screen's there, read until they match or
/// a few seconds pass (the save comes after the redraw; the screen shows it within a frame).
fn blur_on_screen_and_saved(
    app: &App,
    ink: HWND,
    m: &Monitor,
    at: (i32, i32),
) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
    let saved = app.save(ink).map(|i| px(&i, at.0, at.1));
    let screen_px = || {
        let (x, y) = (m.rect.left + at.0, m.rect.top + at.1);
        overmark_win::win32::capture::gdi_screen(Rect::new(x, y, x + 1, y + 1)).map(|f| {
            let p = f.pixel(0, 0);
            [p[2], p[1], p[0]]
        })
    };
    let matched = |a: Option<[u8; 3]>, b: Option<[u8; 3]>| matches!((a, b), (Some(a), Some(b)) if near(a, b, 1));
    let until = Instant::now() + Duration::from_secs(3);
    let mut shown = screen_px();
    while Instant::now() < until && !matched(saved, shown) {
        window::pump();
        window::wait_for_message(10);
        shown = screen_px();
    }
    (saved, shown)
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

fn settings_file(appdata: &Path, captures: &Path, on_shares: bool) {
    let mut settings = Settings::default();
    settings.capture.save_folder = captures.display().to_string();
    settings.capture.after_capture = AfterCapture::Save;
    settings.capture.toast = false;
    settings.guides.show_on_shares = on_shares;
    settings.backdrops.hotkey_whiteboard = BOARD_HOTKEY.into();
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings
        .save(&appdata.join("Overmark").join("settings.toml"))
        .unwrap();
}

struct Results(Vec<(String, bool, String)>);

impl Results {
    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        self.0.push((name.into(), ok, detail.into()));
    }
}

/// The share: the monitor captured as Teams or Zoom would, with probes on a grid line, beside
/// it, and at the barrier marker.
struct Share {
    observer: Observer,
    mark: Window,
    shows: std::cell::Cell<usize>,
}

impl Share {
    /// Probes at a vertical grid line (the 10th, at the monitor's scale), between lines, and at
    /// the marker.
    fn start(m: &Monitor) -> Share {
        let step = 32.0 * f64::from(m.scale());
        let line_x = (10.0 * step).round() as i32;
        let off_x = (10.5 * step).round() as i32;
        let y = (12.5 * step).round() as i32;
        let mark = window_of(m, (8, 8, 14, 14), [0, 0, 0]);
        let observer = Observer::start(m, vec![(line_x, y), (off_x, y), (11, 11)], false, false)
            .expect("a capture of the monitor");
        Share {
            observer,
            mark,
            shows: std::cell::Cell::new(0),
        }
    }

    /// What a share sees at the line and beside it, in the first frame after the scene is
    /// ready (the marker raised above the layer and changed now).
    fn sees(&self) -> Option<([u8; 3], [u8; 3])> {
        let marks: [([u8; 3], [u8; 4]); 2] = [
            ([0, 255, 0], [0, 255, 0, 255]),
            ([255, 0, 255], [255, 0, 255, 255]),
        ];
        let i = (self.shows.get() + 1) % 2;
        self.shows.set(i);
        let (rgb, bgra) = marks[i];
        self.mark.show(false);
        self.mark.fill_gdi(rgb[0], rgb[1], rgb[2]);
        let pump = || {
            window::pump();
            window::wait_for_message(5);
        };
        let f = frame_after_barrier(
            || self.observer.take_frames(),
            2,
            bgra,
            SEEN_WITHIN_MS,
            pump,
        )?;
        let rgb = |p: [u8; 4]| [p[2], p[1], p[0]];
        Some((rgb(f.probes[0]), rgb(f.probes[1])))
    }
}

fn main() {
    let _ = dpi::enable_per_monitor_v2();
    let exe = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|d| d.parent())
        .unwrap()
        .join("overmark.exe");
    assert!(exe.exists(), "build it first ({} missing)", exe.display());
    if overmark_win::win32::tray::is_running(overmark::APP_ID) {
        eprintln!("backdrop_check: Overmark is already running; quit it first (tray icon › Quit)");
        std::process::exit(2);
    }
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let captures = temp.path().join("captures");
    settings_file(&appdata, &captures, true);
    let m = dpi::monitors()
        .into_iter()
        .find(|m| m.primary)
        .expect("a primary monitor");
    let behind = window_of(&m, AREA, BACK);
    let white = [255u8, 255, 255];
    let black = [
        DEFAULT_BLACKBOARD.r,
        DEFAULT_BLACKBOARD.g,
        DEFAULT_BLACKBOARD.b,
    ];
    let red = [
        DEFAULT_PALETTE[0].r,
        DEFAULT_PALETTE[0].g,
        DEFAULT_PALETTE[0].b,
    ];
    let probe = (150, 150);
    let ink_at = (500, STROKE_Y);
    let mut r = Results(Vec::new());

    let mut app = App::start(&exe, &appdata, &[]);
    let (ink, opened) = app.open(&m, "Win+Shift+D");
    r.add(
        "opens Live",
        opened.contains("backdrop=\"Live\""),
        opened.clone(),
    );
    stroke(ink, &m);
    let save_check = |r: &mut Results, app: &App, name: &str, want: [u8; 3]| {
        let img = app.save(ink);
        let (bg, at_ink) = img.as_ref().map_or(([0; 3], [0; 3]), |i| {
            (px(i, probe.0, probe.1), px(i, ink_at.0, ink_at.1))
        });
        r.add(
            name,
            img.is_some() && near(bg, want, 3) && near(at_ink, red, 30),
            format!("backdrop {bg:?} (want {want:?}), stroke {at_ink:?} (want about {red:?})"),
        );
    };
    save_check(
        &mut r,
        &app,
        "Live: a save is the screen, with the stroke",
        BACK,
    );

    // W round: Frozen, Whiteboard, Blackboard, Live; then Shift+W back.
    let w = |app: &App, shift: bool, want: &str| -> (bool, String) {
        let line = app.key(ink, 'W', shift, " W ").unwrap_or_default();
        (line.contains(&format!("backdrop=\"{want}\"")), line)
    };
    let (ok, line) = w(&app, false, "Frozen");
    r.add("W: Live to Frozen", ok, line);
    save_check(
        &mut r,
        &app,
        "Frozen: a save is the frozen frame, with the stroke",
        BACK,
    );
    let (ok, line) = w(&app, false, "Whiteboard");
    r.add("W: Frozen to the whiteboard", ok, line);
    // The screen behind changes now; the frozen frame mustn't.
    behind.fill_gdi(BACK2[0], BACK2[1], BACK2[2]);
    save_check(
        &mut r,
        &app,
        "Whiteboard: a save is white, with the stroke",
        white,
    );
    let veil = app.window(Duration::from_millis(0), |w| {
        w.class == "OvermarkSurface" && w.visible && w.alpha.is_some_and(|a| a > 0 && a < 255)
    });
    r.add("no dim over the whiteboard", veil.is_none(), "");
    let share = Share::start(&m);
    let seen = share.sees();
    r.add(
        "a share sees the whiteboard",
        seen.is_some_and(|(a, b)| near(a, white, 2) && near(b, white, 2)),
        format!("{seen:?}"),
    );
    // G round, on the whiteboard: Grid, Dots, Lines, Off; Shift+G back to Lines, then Grid.
    let g = |app: &App, shift: bool, want: &str| -> (bool, String) {
        let line = app.key(ink, 'G', shift, " G ").unwrap_or_default();
        (line.contains(&format!("guides=\"{want}\"")), line)
    };
    for (shift, want) in [
        (false, "Grid"),
        (false, "Dots"),
        (false, "Lines"),
        (false, "Off"),
        (true, "Lines"),
        (true, "Dots"),
        (true, "Grid"),
    ] {
        let (ok, line) = g(&app, shift, want);
        r.add(
            &format!("{}G: {want}", if shift { "Shift+" } else { "" }),
            ok,
            line,
        );
    }
    let seen = share.sees();
    r.add(
        "a share sees the grid on the whiteboard",
        seen.is_some_and(|(line, off)| !near(line, white, 4) && near(off, white, 2)),
        format!("on a line, beside it: {seen:?}"),
    );
    let step = 32.0 * f64::from(m.scale());
    let (gx, gy) = ((10.0 * step).round() as i32, (12.5 * step).round() as i32);
    let img = app.save(ink);
    let in_save = img
        .as_ref()
        .map(|i| (px(i, gx, gy), px(i, gx + (step / 2.0) as i32, gy)));
    r.add(
        "a save has the grid",
        in_save.is_some_and(|(line, off)| !near(line, white, 4) && near(off, white, 2)),
        format!("{in_save:?}"),
    );
    let (ok, line) = w(&app, false, "Blackboard");
    r.add("W: the whiteboard to the blackboard", ok, line);
    let _ = g(&app, true, "Off");
    save_check(
        &mut r,
        &app,
        "Blackboard: a save is its grey, with the stroke",
        black,
    );
    let (ok, line) = w(&app, true, "Whiteboard");
    r.add("Shift+W: the blackboard to the whiteboard", ok, line);
    let (ok, line) = w(&app, true, "Frozen");
    r.add("Shift+W: the whiteboard to Frozen", ok, line);
    save_check(
        &mut r,
        &app,
        "Frozen again: the same frozen frame (the old colour), with the stroke",
        BACK,
    );
    let (ok, line) = w(&app, true, "Live");
    r.add("Shift+W: Frozen to Live", ok, line);
    save_check(&mut r, &app, "Live again: the screen as it is now", BACK2);
    let (ok, line) = w(&app, true, "Blackboard");
    r.add("Shift+W: Live round to the blackboard", ok, line);
    // F from a board goes to Frozen (a new frame: the screen was Live since), and back to Live.
    let line = app.key(ink, 'F', false, " F ").unwrap_or_default();
    r.add(
        "F: the blackboard to Frozen",
        line.contains("backdrop=\"Frozen\""),
        line,
    );
    save_check(
        &mut r,
        &app,
        "Frozen from F: the screen as frozen now",
        BACK2,
    );
    let line = app.key(ink, 'F', false, " F ").unwrap_or_default();
    r.add(
        "F: Frozen to Live",
        line.contains("backdrop=\"Live\""),
        line,
    );
    // A blur on the frozen frame with the grid on: what the screen shows is what's saved, and
    // still after G changes the guides under it.
    let _ = app.key(ink, 'F', false, " F ");
    let _ = app.key(ink, 'G', false, " G ");
    let _ = app.key(ink, 'B', false, "tool=\"blur\"");
    drag(ink, &m, (300, 250), (500, 450));
    let _ = app.key(ink, 'P', false, "tool=\"pen\"");
    for step in ["grid", "dots (after G)"] {
        if step != "grid" {
            let _ = app.key(ink, 'G', false, " G ");
        }
        let (saved, shown) = blur_on_screen_and_saved(&app, ink, &m, (400, 350));
        r.add(
            &format!("Frozen blur over the {step}: the screen shows what's saved"),
            // Over the grid the block has a line in it, so it isn't the plain screen; after G
            // the screen still matching the save shows the blur was drawn again.
            matches!((saved, shown), (Some(a), Some(b))
                if near(a, b, 1) && (step != "grid" || !near(a, BACK2, 1))),
            format!("saved {saved:?}, on screen {shown:?}"),
        );
    }
    let _ = app.key(ink, 'G', false, " G ");
    let _ = app.key(ink, 'G', false, " G "); // Off
    window::post(ink, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    // The board hotkey from Settings opens the layer on the whiteboard.
    let (ink2, opened) = app.open(&m, BOARD_HOTKEY);
    r.add(
        &format!("{BOARD_HOTKEY} (Settings' hotkey_whiteboard) opens on the whiteboard"),
        opened.contains("backdrop=\"Whiteboard\""),
        opened,
    );
    window::post(ink2, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    drop(share);
    let quit = app.owned.stop(&exe);

    // Guides kept off shares: the share sees the plain whiteboard, a save still has the grid.
    settings_file(&appdata, &captures, false);
    app = App::start(&exe, &appdata, &[]);
    let (ink, _) = app.open(&m, BOARD_HOTKEY);
    let _ = app.key(ink, 'G', false, " G ");
    let share = Share::start(&m);
    let seen = share.sees();
    r.add(
        "\"Show guides on screen shares\" off: a share sees no grid",
        seen.is_some_and(|(line, off)| near(line, white, 2) && near(off, white, 2)),
        format!("on a line, beside it: {seen:?}"),
    );
    let excluded = app
        .window(Duration::from_secs(1), |w| {
            w.class == "OvermarkSurface" && w.visible && affinity::is_excluded(w.hwnd)
        })
        .is_some();
    r.add("the guides' layer is excluded from capture", excluded, "");
    let img = app.save(ink);
    let in_save = img
        .as_ref()
        .map(|i| (px(i, gx, gy), px(i, gx + (step / 2.0) as i32, gy)));
    r.add(
        "and a save still has the grid (include_in_saves)",
        in_save.is_some_and(|(line, off)| !near(line, white, 4) && near(off, white, 2)),
        format!("{in_save:?}"),
    );
    window::post(ink, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    drop(share);
    let quit2 = app.owned.stop(&exe);

    // Without the grabber (the GDI fallback): Frozen from a board grabs the screen under it,
    // not the board.
    settings_file(&appdata, &captures, true);
    app = App::start(&exe, &appdata, &["--no-grabber"]);
    let (ink, _) = app.open(&m, BOARD_HOTKEY);
    let line = app.key(ink, 'F', false, " F ").unwrap_or_default();
    r.add(
        "--no-grabber: F from the whiteboard to Frozen",
        line.contains("backdrop=\"Frozen\""),
        line,
    );
    let img = app.save(ink);
    let bg = img.as_ref().map(|i| px(i, probe.0, probe.1));
    r.add(
        "--no-grabber: the frozen frame is the screen under the board, not the board",
        bg.is_some_and(|b| near(b, BACK2, 3)),
        format!("{bg:?} (want {BACK2:?})"),
    );
    window::post(ink, WM_OVERMARK_KEY, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    let quit3 = app.owned.stop(&exe);
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
    if !(quit && quit2 && quit3) {
        println!("FAILED it didn't quit when asked");
        failed += 1;
    }
    println!(
        "\nbackdrop_check: {}",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
