//! Checks captures on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example capture_check [-- --clipboard]
//! ```
//!
//! It starts `overmark.exe --test-drawing` with `%APPDATA%` and the captures folder in temp
//! folders of its own, over a backdrop window of a known colour, opens the layer through the
//! app's own hotkey window, and asks for captures with `WM_OVERMARK_COMMAND` posted to the ink
//! layer (what Ctrl+S and the rest do; no global input). A region is dragged with mouse messages
//! posted to the app's own selection window. Each saved file is read back and checked:
//!
//! - its size is the monitor's (or the region's);
//! - away from the drawing it is the backdrop exactly: **no dim** (nor anything else of ours);
//! - the half-alpha pen stroke is there **once** (twice, from a grab with the ink in it plus the
//!   drawing again, would be darker), and the highlighter darkens what's under it;
//! - without the drawing, the strokes' places are the backdrop.
//!
//! Live and Frozen (from F, and from the Frozen hotkey) are both checked. The notice must show
//! and be excluded from capture. Saves are set to "save" only, so the user's clipboard is left
//! alone; `--clipboard` also checks the default copy-and-save (it replaces what's on the
//! clipboard). Run it with no Overmark running (it stops if one is), when you're not typing: the
//! layer takes the screen and the focus for a few seconds.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark::{TEST_MARKER_Y, TEST_PEN_Y, TEST_X0, TEST_X1};
use overmark_core::hotkey::Hotkey;
use overmark_core::settings::{AfterCapture, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::overlay::{Command as Capture, WM_OVERMARK_COMMAND};
use overmark_win::win32::Rect;
use overmark_win::win32::affinity;
use overmark_win::win32::dpi;
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// The backdrop: RGB (200, 100, 50).
const BACK: [u8; 3] = [200, 100, 50];

/// The app this check started, stopped however the check ends (as `resident_check`'s).
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
                    "capture_check: stopping the Overmark it started (pid {})",
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

    fn ink(&self) -> Option<WindowInfo> {
        self.window(Duration::from_secs(3), |w| {
            w.class == "OvermarkSurface"
                && w.visible
                && w.ex_style & WS_EX_TRANSPARENT == 0
                && w.title == "Overmark"
        })
    }

    /// Asks for a capture and returns the file it saved (from the app's log).
    fn capture(&self, ink: &WindowInfo, c: Capture) -> Option<PathBuf> {
        window::post(ink.hwnd, WM_OVERMARK_COMMAND, c.wparam(), 0);
        let line = self.wait_for(" delivered", Duration::from_secs(5))?;
        saved_file(&line)
    }
}

/// The path in a "delivered" line's `file=Some("…")`.
fn saved_file(line: &str) -> Option<PathBuf> {
    let at = line.find("file=Some(\"")? + "file=Some(\"".len();
    let end = line[at..].find("\")")?;
    Some(PathBuf::from(line[at..at + end].replace("\\\\", "\\")))
}

fn read_png(p: &Path) -> image::RgbaImage {
    image::load_from_memory(&std::fs::read(p).expect("the saved file"))
        .expect("a PNG")
        .to_rgba8()
}

fn near(a: [u8; 4], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tol)
}

/// Half-alpha blue over the backdrop, once.
fn pen_once() -> [u8; 3] {
    let mix = |b: u8, c: u8| ((u16::from(b) * 127 + u16::from(c) * 128) / 255) as u8;
    [mix(BACK[0], 0), mix(BACK[1], 0), mix(BACK[2], 255)]
}

/// Yellow (255, 255, 0) darkening the backdrop: each channel's minimum.
fn marker() -> [u8; 3] {
    let yellow = [255u8, 255, 0];
    [
        BACK[0].min(yellow[0]),
        BACK[1].min(yellow[1]),
        BACK[2].min(yellow[2]),
    ]
}

/// Checks a full-monitor save: `(name, ok, detail)` lines go to `out`.
fn check_save(
    name: &str,
    file: Option<&Path>,
    monitor: Rect,
    with_drawing: bool,
    out: &mut Vec<(String, bool, String)>,
) {
    let Some(file) = file else {
        out.push((name.into(), false, "no file".into()));
        return;
    };
    let img = read_png(file);
    let size_ok = (img.width() as i32, img.height() as i32) == (monitor.width(), monitor.height());
    let mid = ((TEST_X0 + TEST_X1) / 2.0) as u32;
    let px = |x: u32, y: f64| img.get_pixel(x, y as u32).0;
    let clear = px(mid, (TEST_PEN_Y + TEST_MARKER_Y) / 2.0);
    let pen = px(mid, TEST_PEN_Y);
    let hl = px(mid, TEST_MARKER_Y);
    let ok_clear = near(clear, BACK, 2);
    let (ok_pen, ok_hl) = if with_drawing {
        (near(pen, pen_once(), 4), near(hl, marker(), 4))
    } else {
        (near(pen, BACK, 2), near(hl, BACK, 2))
    };
    let ok = size_ok && ok_clear && ok_pen && ok_hl;
    out.push((
        name.into(),
        ok,
        format!(
            "{}x{}; between the strokes {:?} (backdrop {:?}); pen {:?} (want {:?}); highlighter {:?} (want {:?})",
            img.width(),
            img.height(),
            &clear[..3],
            BACK,
            &pen[..3],
            if with_drawing { pen_once() } else { BACK },
            &hl[..3],
            if with_drawing { marker() } else { BACK }
        ),
    ));
}

fn main() {
    // The app's windows are named in the UI language: look for them by the same names.
    overmark_core::i18n::init(&overmark_win::win32::language::ui_language());
    let clipboard = std::env::args().any(|a| a == "--clipboard");
    let _ = dpi::enable_per_monitor_v2();
    let exe = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|d| d.parent())
        .unwrap()
        .join("overmark.exe");
    assert!(exe.exists(), "build it first ({} missing)", exe.display());
    if overmark_win::win32::tray::is_running(overmark::APP_ID) {
        eprintln!("capture_check: Overmark is already running; quit it first (tray icon › Quit)");
        std::process::exit(2);
    }
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let captures = temp.path().join("captures");
    let mut settings = Settings::default();
    settings.capture.save_folder = captures.display().to_string();
    if !clipboard {
        settings.capture.after_capture = AfterCapture::Save;
    }
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings
        .save(&appdata.join("Overmark").join("settings.toml"))
        .unwrap();

    // The backdrop, under where the test strokes go, and the probes between them.
    let m = dpi::monitors().first().unwrap().clone();
    let back = Window::new(
        Rect::new(
            m.rect.left + TEST_X0 as i32 - 60,
            m.rect.top + TEST_PEN_Y as i32 - 60,
            m.rect.left + TEST_X1 as i32 + 60,
            m.rect.top + TEST_MARKER_Y as i32 + 60,
        ),
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .unwrap();
    back.fill_gdi(BACK[0], BACK[1], BACK[2]);
    back.show(false);
    back.fill_gdi(BACK[0], BACK[1], BACK[2]);

    let mut child = Command::new(&exe)
        .arg("--test-drawing")
        .env("APPDATA", &appdata)
        .stderr(Stdio::piped())
        .spawn()
        .expect("start overmark.exe");
    let stderr = child.stderr.take().unwrap();
    let owned = Owned(Some(child));
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            println!("    | {line}");
            let _ = tx.send(line);
        }
    });
    let mut app = App { owned, lines };
    assert!(
        app.wait_for("resident in the tray", Duration::from_secs(10))
            .is_some(),
        "the app didn't start"
    );
    let hotkeys = app
        .window(Duration::from_secs(5), |w| w.class == "global_hotkey_app")
        .expect("the hotkey window");
    let id = |t: &str| to_hotkey(&Hotkey::parse(t).unwrap().unwrap()).id() as usize;
    let lparam = |vk: u32| ((vk << 16) | 0x0008 | 0x0004) as isize;
    let mut results: Vec<(String, bool, String)> = Vec::new();
    let mut files = Vec::new();

    // Live: save, then save without the drawing.
    window::post(hotkeys.hwnd, WM_HOTKEY, id("Win+Shift+D"), lparam(0x44));
    app.wait_for(" opened", Duration::from_secs(3))
        .expect("the layer opened");
    let ink = app.ink().expect("the ink layer");
    // The ink is on screen (a GDI grab from here sees the stroke over the backdrop), so a save
    // with it exactly once shows the app's grab left it out.
    let pen_at = (
        m.rect.left + ((TEST_X0 + TEST_X1) / 2.0) as i32,
        m.rect.top + TEST_PEN_Y as i32,
    );
    let until = Instant::now() + Duration::from_secs(3);
    let mut on_screen = None;
    while Instant::now() < until {
        on_screen = overmark_win::win32::capture::gdi_screen(Rect::new(
            pen_at.0,
            pen_at.1,
            pen_at.0 + 1,
            pen_at.1 + 1,
        ))
        .map(|f| f.pixel(0, 0));
        if on_screen.is_some_and(|p| near([p[2], p[1], p[0], 255], pen_once(), 6)) {
            break;
        }
        window::pump();
        window::wait_for_message(10);
    }
    let shown = on_screen.is_some_and(|p| near([p[2], p[1], p[0], 255], pen_once(), 6));
    results.push((
        "the ink is on screen while saving".into(),
        shown,
        format!(
            "the screen at the pen stroke: {:?}",
            on_screen.map(|p| [p[2], p[1], p[0]])
        ),
    ));
    let f = app.capture(&ink, Capture::Save);
    check_save("Live: save", f.as_deref(), m.rect, true, &mut results);
    files.extend(f);
    // The notice: shown, and excluded from capture.
    let toast = app.window(Duration::from_secs(3), |w| {
        w.title == overmark_core::i18n::tr("window-notice-name") && w.visible
    });
    results.push((
        "the notice".into(),
        toast
            .as_ref()
            .is_some_and(|t| affinity::is_excluded(t.hwnd)),
        match &toast {
            Some(t) => format!(
                "shown, excluded from capture: {}",
                affinity::is_excluded(t.hwnd)
            ),
            None => "not shown".into(),
        },
    ));
    let f = app.capture(&ink, Capture::SaveWithoutDrawing);
    check_save(
        "Live: save without the drawing",
        f.as_deref(),
        m.rect,
        false,
        &mut results,
    );
    files.extend(f);

    // A region, dragged on the selection window; then a selection cancelled with Esc.
    window::post(
        ink.hwnd,
        WM_OVERMARK_COMMAND,
        Capture::SaveRegion.wparam(),
        0,
    );
    let region = Rect::new(
        TEST_X0 as i32 - 20,
        TEST_PEN_Y as i32 - 30,
        TEST_X1 as i32 + 20,
        TEST_MARKER_Y as i32 + 30,
    );
    match app.window(Duration::from_secs(3), |w| {
        w.title == overmark_core::i18n::tr("window-selection-name") && w.visible
    }) {
        Some(sel) => {
            let at = |x: i32, y: i32| ((y as isize) << 16) | (x as isize & 0xFFFF);
            window::post(sel.hwnd, WM_LBUTTONDOWN, 1, at(region.left, region.top));
            window::post(
                sel.hwnd,
                WM_MOUSEMOVE,
                1,
                at(region.right - 1, region.bottom - 1),
            );
            window::post(
                sel.hwnd,
                WM_LBUTTONUP,
                0,
                at(region.right - 1, region.bottom - 1),
            );
            let f = app
                .wait_for(" delivered", Duration::from_secs(5))
                .and_then(|l| saved_file(&l));
            match &f {
                Some(p) => {
                    let img = read_png(p);
                    let mid = (region.width() / 2) as u32;
                    let pen = img
                        .get_pixel(mid, (TEST_PEN_Y as i32 - region.top) as u32)
                        .0;
                    let corner = img.get_pixel(2, 2).0;
                    let ok = (img.width() as i32, img.height() as i32)
                        == (region.width(), region.height())
                        && near(pen, pen_once(), 4)
                        && near(corner, BACK, 2);
                    results.push((
                        "Live: a region".into(),
                        ok,
                        format!(
                            "{}x{} (dragged {}x{}); pen {:?}; corner {:?}",
                            img.width(),
                            img.height(),
                            region.width(),
                            region.height(),
                            &pen[..3],
                            &corner[..3]
                        ),
                    ));
                }
                None => results.push(("Live: a region".into(), false, "no file".into())),
            }
            files.extend(f);
        }
        None => results.push(("Live: a region".into(), false, "no selection window".into())),
    }
    window::post(
        ink.hwnd,
        WM_OVERMARK_COMMAND,
        Capture::SaveRegion.wparam(),
        0,
    );
    let cancelled = app
        .window(Duration::from_secs(3), |w| {
            w.title == overmark_core::i18n::tr("window-selection-name") && w.visible
        })
        .map(|sel| window::post(sel.hwnd, WM_KEYDOWN, 0x1B, 0))
        .is_some()
        && app
            .wait_for("region cancelled", Duration::from_secs(3))
            .is_some()
        && app.ink().is_some();
    results.push((
        "Esc cancels a region, and the layer stays".into(),
        cancelled,
        String::new(),
    ));

    // Frozen from F, then from the Frozen hotkey.
    window::post(ink.hwnd, WM_KEYDOWN, 0x46, 0);
    app.wait_for(" F", Duration::from_secs(3)).expect("F");
    let f = app.capture(&ink, Capture::Save);
    check_save("Frozen (F): save", f.as_deref(), m.rect, true, &mut results);
    files.extend(f);
    window::post(ink.hwnd, WM_KEYDOWN, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    window::post(hotkeys.hwnd, WM_HOTKEY, id("Win+Shift+F"), lparam(0x46));
    app.wait_for(" opened", Duration::from_secs(3))
        .expect("the layer opened Frozen");
    let ink = app.ink().expect("the ink layer");
    let f = app.capture(&ink, Capture::Save);
    check_save(
        "Frozen (hotkey): save",
        f.as_deref(),
        m.rect,
        true,
        &mut results,
    );
    files.extend(f);
    let f = app.capture(&ink, Capture::SaveWithoutDrawing);
    check_save(
        "Frozen: save without the drawing",
        f.as_deref(),
        m.rect,
        false,
        &mut results,
    );
    files.extend(f);
    if clipboard {
        // The default: copied and saved; the clipboard has the same picture. The clipboard's
        // text, if any, is put back afterwards (a picture on it isn't).
        let before = arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .ok();
        let f = app.capture(&ink, Capture::Save);
        let same = f.as_ref().is_some_and(|p| {
            let saved = read_png(p);
            arboard::Clipboard::new()
                .and_then(|mut c| c.get_image())
                .is_ok_and(|c| {
                    c.width as u32 == saved.width()
                        && c.height as u32 == saved.height()
                        && c.bytes.as_ref() == saved.as_raw().as_slice()
                })
        });
        results.push(("the default copies and saves".into(), same, String::new()));
        let f2 = app.capture(&ink, Capture::Copy);
        let copied = f2.is_none()
            && arboard::Clipboard::new()
                .and_then(|mut c| c.get_image())
                .is_ok_and(|c| c.width as i32 == m.rect.width());
        results.push(("Ctrl+C copies only".into(), copied, String::new()));
        if let Some(text) = before {
            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
        }
    }
    window::post(ink.hwnd, WM_KEYDOWN, 0x1B, 0);
    app.wait_for("closed", Duration::from_secs(3));
    drop(back);

    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let named = !names.is_empty()
        && names
            .iter()
            .all(|n| n.ends_with(".png") && n.len() >= "2026-11-01 00.00.00.png".len())
        && files.iter().all(|p| p.parent() == Some(captures.as_path()));
    results.push((
        "files named by the pattern, in the captures folder (made)".into(),
        named,
        format!("{names:?}"),
    ));
    let quit = app.owned.stop(&exe);
    println!();
    let mut failed = 0;
    for (name, ok, detail) in &results {
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
        "\ncapture_check: {}",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
