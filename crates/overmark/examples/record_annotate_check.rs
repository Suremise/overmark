//! Checks recording while annotating on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example record_annotate_check
//! ```
//!
//! It starts `overmark.exe --test-drawing` with `%APPDATA%` and the captures folder in temp
//! folders of its own, over backdrop windows of a known colour, and opens the layer through the
//! app's own hotkey window. Record is pressed on the toolbar and the bar's buttons through UI
//! Automation on the app's own windows (what Narrator presses); the box is dragged with mouse
//! messages posted to the app's own selection window. Nothing global: no input is sent, and
//! the check activates no window itself. Each video is read back with Media Foundation:
//!
//! - **A box round the test strokes** (the bar above it): record, pause, resume, stop. The MP4
//!   is in the captures folder, its strokes there (the drawing is in the video), and the
//!   notice shows.
//! - **The whole screen** (the bar inside its top edge): the bar's and the outline's pixels in
//!   the video are the backdrop under them. Both are excluded from capture.
//! - **Esc before recording** cancels the box and leaves the layer open; **Discard** asks, then
//!   leaves no file; **Esc while recording** saves, then closes the layer.
//! - The foreground window stays the user's at every step: the app runs with `--no-activate`
//!   (no window of it is ever activated; the check posts its keys to the app's own windows) and
//!   `--no-grabber`, and the check activates nothing.
//!
//! Run it with no Overmark running (it stops if one is). The layer is on the screen for some
//! seconds, but never takes the keyboard. It stops the app it started however it ends, and its temp
//! folders go with it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark::{TEST_MARKER_Y, TEST_PEN_Y, TEST_X0, TEST_X1};
use overmark_core::hotkey::Hotkey;
use overmark_core::i18n::tr;
use overmark_core::settings::{AfterCapture, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::win32::uia::Uia;
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use overmark_win::win32::{Rect, affinity, dpi, mf};
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// The backdrop: RGB (200, 100, 50).
const BACK: [u8; 3] = [200, 100, 50];

/// The app this check started, stopped however the check ends.
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
                    "record_annotate_check: stopping the Overmark it started (pid {})",
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
    uia: Uia,
    /// The foreground when the check began, and every step after which it wasn't that.
    foreground: Option<windows::Win32::Foundation::HWND>,
    took: std::cell::RefCell<Vec<String>>,
}

impl App {
    /// After `step`: the foreground must still be the user's (the app runs with
    /// `--no-activate`, and the check activates nothing).
    fn watch(&self, step: &str) {
        let now = window::foreground();
        if now != self.foreground {
            self.took
                .borrow_mut()
                .push(format!("{step}: {}", window::describe(now)));
        }
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

    /// One of the app's own windows (found by its process id).
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

    fn titled(&self, key: &str) -> Option<WindowInfo> {
        let title = tr(key);
        self.window(Duration::from_secs(5), |w| w.title == title && w.visible)
    }

    fn ink(&self) -> Option<WindowInfo> {
        self.window(Duration::from_secs(3), |w| {
            w.class == "OvermarkSurface"
                && w.visible
                && w.ex_style & WS_EX_TRANSPARENT == 0
                && w.title == "Overmark"
        })
    }

    /// Presses the control named `name` on the app's window `hwnd` (UI Automation's Invoke),
    /// waiting for it to be there.
    fn press(&self, hwnd: windows::Win32::Foundation::HWND, name: &str) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if self.uia.invoke(hwnd, name).unwrap_or(false) {
                self.watch(&format!("pressed {name}"));
                return true;
            }
            window::pump();
            window::wait_for_message(20);
        }
        false
    }

    /// Drags `rect` (its monitor's pixels) on the selection window over `monitor`.
    fn drag(&self, monitor: Rect, rect: Rect) -> bool {
        let title = tr("window-selection-name");
        let Some(sel) = self.window(Duration::from_secs(5), |w| {
            w.title == title && w.visible && w.rect == monitor
        }) else {
            return false;
        };
        let at = |x: i32, y: i32| ((y as isize) << 16) | (x as isize & 0xFFFF);
        window::post(sel.hwnd, WM_LBUTTONDOWN, 1, at(rect.left, rect.top));
        window::post(
            sel.hwnd,
            WM_MOUSEMOVE,
            1,
            at(rect.right - 1, rect.bottom - 1),
        );
        window::post(
            sel.hwnd,
            WM_LBUTTONUP,
            0,
            at(rect.right - 1, rect.bottom - 1),
        );
        self.watch("dragged a box");
        true
    }

    /// The toolbar's Record, then the box dragged: the bar's window once it's up.
    fn record_box(&self, monitor: Rect, rect: Rect) -> Option<WindowInfo> {
        let toolbar = self.titled("toolbar-name")?;
        if !self.press(toolbar.hwnd, &tr("toolbar-record")) {
            return None;
        }
        if !self.drag(monitor, rect) {
            return None;
        }
        self.wait_for("a box to record", Duration::from_secs(5))?;
        self.titled("window-record-bar-name")
    }

    /// Lets `secs` of real time pass, keeping this thread's messages moving (a recording needs
    /// real time to pass: this is what's being recorded).
    fn record_for(&self, secs: f64) {
        let until = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < until {
            window::pump();
            window::wait_for_message(20);
            self.watch("recording");
        }
    }

    /// Posts Esc to the app's ink layer (its keys come as messages: the layer has no keyboard
    /// of its own with `--no-activate`).
    fn esc(&self, ink: &WindowInfo) {
        window::post_key(ink.hwnd, 0x1B);
        self.watch("Esc");
    }

    /// The bar's visible controls on the screen: the union of its UI Automation elements.
    fn bar_controls(&self, bar: &WindowInfo) -> Option<Rect> {
        let elements = self.uia.elements(bar.hwnd).ok()?;
        elements
            .iter()
            .filter(|e| !e.rect.is_empty())
            .map(|e| e.rect)
            .reduce(|a, b| a.union(&b))
    }
}

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tol)
}

/// Half-alpha blue over the backdrop, once.
fn pen_once() -> [u8; 3] {
    let mix = |b: u8, c: u8| ((u16::from(b) * 127 + u16::from(c) * 128) / 255) as u8;
    [mix(BACK[0], 0), mix(BACK[1], 0), mix(BACK[2], 255)]
}

/// The MP4s in `folder`, and anything else left there.
fn files(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// The one file in `after` that isn't in `before`.
fn new_file(before: &[PathBuf], after: &[PathBuf]) -> Option<PathBuf> {
    let new: Vec<&PathBuf> = after.iter().filter(|p| !before.contains(p)).collect();
    (new.len() == 1).then(|| new[0].clone())
}

/// The pixel (`x`, `y`) of a decoded BGRA frame `w` wide, as RGB.
fn rgb(frame: &(u32, u32, Vec<u8>), x: i32, y: i32) -> [u8; 3] {
    let i = ((y as u32 * frame.0 + x as u32) * 4) as usize;
    [frame.2[i + 2], frame.2[i + 1], frame.2[i]]
}

fn backdrop(rect: Rect) -> Window {
    let back = Window::new(
        rect,
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .expect("a backdrop window");
    back.fill_gdi(BACK[0], BACK[1], BACK[2]);
    back.show(false);
    back.fill_gdi(BACK[0], BACK[1], BACK[2]);
    back
}

fn main() {
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
        eprintln!(
            "record_annotate_check: Overmark is already running; quit it first (tray icon › Quit)"
        );
        std::process::exit(2);
    }
    let foreground_before = window::foreground();
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let captures = temp.path().join("captures");
    std::fs::create_dir_all(&captures).unwrap();
    let mut settings = Settings::default();
    settings.capture.save_folder = captures.display().to_string();
    settings.capture.after_capture = AfterCapture::Save;
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings
        .save(&appdata.join("Overmark").join("settings.toml"))
        .unwrap();

    let m = dpi::monitors().first().unwrap().clone();
    let scale = m.scale();
    // The backdrops: under the test strokes, and along the monitor's top edge, where a
    // full-screen box has its outline and its bar.
    let _strokes = backdrop(Rect::new(
        m.rect.left + TEST_X0 as i32 - 60,
        m.rect.top + TEST_PEN_Y as i32 - 60,
        m.rect.left + TEST_X1 as i32 + 60,
        m.rect.top + TEST_MARKER_Y as i32 + 60,
    ));
    let top_strip = Rect::new(
        m.rect.left,
        m.rect.top,
        m.rect.left + (480.0 * scale) as i32,
        m.rect.top + (80.0 * scale) as i32,
    );
    let _top = backdrop(top_strip);

    // `--no-activate`: no window of the app ever takes the foreground (the check posts its keys
    // to the app's own windows); `--no-grabber`: no helper process, which would make windows of
    // its own.
    let mut child = Command::new(&exe)
        .args(["--test-drawing", "--no-activate", "--no-grabber"])
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
    let mut app = App {
        owned,
        lines,
        uia: Uia::new().expect("UI Automation"),
        foreground: foreground_before,
        took: std::cell::RefCell::new(Vec::new()),
    };
    let mut results: Vec<(String, bool, String)> = Vec::new();
    let started = app
        .wait_for("resident in the tray", Duration::from_secs(10))
        .is_some();
    results.push(("the app started".into(), started, String::new()));
    let hotkeys = app.window(Duration::from_secs(5), |w| w.class == "global_hotkey_app");
    let id = |t: &str| to_hotkey(&Hotkey::parse(t).unwrap().unwrap()).id() as usize;
    let lparam = |vk: u32| ((vk << 16) | 0x0008 | 0x0004) as isize;
    let open = |app: &App| -> bool {
        let Some(h) = &hotkeys else { return false };
        window::post(h.hwnd, WM_HOTKEY, id("Win+Shift+D"), lparam(0x44));
        app.wait_for(" opened", Duration::from_secs(3)).is_some()
    };

    if started && open(&app) {
        // 1. A box round the strokes: record, pause, resume, stop.
        let around = Rect::new(
            TEST_X0 as i32 - 40,
            TEST_PEN_Y as i32 - 40,
            TEST_X1 as i32 + 40,
            TEST_MARKER_Y as i32 + 40,
        );
        let before = files(&captures);
        match app.record_box(m.rect, around) {
            Some(bar) => {
                let outline_title = tr("window-record-outline-name");
                let outline: Vec<WindowInfo> = process_windows(app.owned.pid())
                    .into_iter()
                    .filter(|w| w.title == outline_title && w.visible)
                    .collect();
                let excluded = affinity::is_excluded(bar.hwnd)
                    && outline.len() == 4
                    && outline.iter().all(|w| affinity::is_excluded(w.hwnd));
                results.push((
                    "the bar and outline are excluded from capture".into(),
                    excluded,
                    format!("bar {:?}, {} outline strips", bar.rect, outline.len()),
                ));
                // The visible controls, not the window round them (larger, and transparent).
                let controls = app.bar_controls(&bar);
                let (top, left) = (m.rect.top + around.top, m.rect.left + around.left);
                results.push((
                    "the visible bar sits just above the box, at its left edge".into(),
                    controls.is_some_and(|c| {
                        c.bottom <= top
                            && c.bottom >= top - (16.0 * scale) as i32
                            && c.left >= left
                            && c.left <= left + (16.0 * scale) as i32
                    }),
                    format!("controls {controls:?}, box {around:?}"),
                ));
                let steps = [
                    (tr("record-bar-record"), "recording started", 1.5),
                    (tr("record-bar-pause"), "record bar", 1.0),
                    (tr("record-bar-resume"), "record bar", 1.0),
                ];
                let mut ok = true;
                for (name, log, secs) in steps {
                    ok &= app.press(bar.hwnd, &name);
                    ok &= app.wait_for(log, Duration::from_secs(5)).is_some();
                    app.record_for(secs);
                }
                ok &= app.press(bar.hwnd, &tr("record-bar-stop"));
                let saved = app.wait_for("recording saved", Duration::from_secs(10));
                let notice = app
                    .titled("window-notice-name")
                    .is_some_and(|t| affinity::is_excluded(t.hwnd));
                let file = new_file(&before, &files(&captures));
                let detail = match &file {
                    Some(f) => match (mf::probe(f), mf::decode_frame_at(f, 10_000_000)) {
                        (Ok(info), Ok(frame)) => {
                            let mid = around.width() / 2;
                            let pen = rgb(&frame, mid, TEST_PEN_Y as i32 - around.top);
                            let clear = rgb(
                                &frame,
                                mid,
                                ((TEST_PEN_Y + TEST_MARKER_Y) / 2.0) as i32 - around.top,
                            );
                            let secs = info.duration_100ns as f64 / 1e7;
                            // 1.5 s, then 1 s paused (left out), then 1 s: about 2.5 s.
                            ok &= info.h264
                                && (info.width as i32, info.height as i32)
                                    == (around.width(), around.height())
                                && (2.2..3.4).contains(&secs)
                                && near(pen, pen_once(), 12)
                                && near(clear, BACK, 8);
                            format!(
                                "{} ({}x{}, {secs:.2} s, {} frames); pen {pen:?} (want {:?}); between the strokes {clear:?} (want {BACK:?})",
                                f.file_name().unwrap().to_string_lossy(),
                                info.width,
                                info.height,
                                info.frames,
                                pen_once()
                            )
                        }
                        (a, b) => {
                            ok = false;
                            format!("unreadable: {:?} {:?}", a.err(), b.err().map(|_| ()))
                        }
                    },
                    None => {
                        ok = false;
                        format!("no new MP4 in {}", captures.display())
                    }
                };
                results.push((
                    "record, pause, resume, stop: the MP4 has the drawing, the pause left out"
                        .into(),
                    ok && saved.is_some(),
                    detail,
                ));
                results.push(("the notice after Stop".into(), notice, String::new()));
            }
            None => results.push(("a box to record".into(), false, "no bar".into())),
        }

        // 2. The whole screen: the bar inside its top edge, and neither it nor the outline in
        // the video.
        let before = files(&captures);
        let full = Rect::new(0, 0, m.rect.width(), m.rect.height());
        match app.record_box(m.rect, full) {
            Some(bar) => {
                let controls = app.bar_controls(&bar);
                let inside = controls.is_some_and(|c| {
                    c.top > m.rect.top && c.top <= m.rect.top + (24.0 * scale) as i32
                });
                let mut ok = app.press(bar.hwnd, &tr("record-bar-record"));
                ok &= app
                    .wait_for("recording started", Duration::from_secs(5))
                    .is_some();
                app.record_for(1.5);
                ok &= app.press(bar.hwnd, &tr("record-bar-stop"));
                ok &= app
                    .wait_for("recording saved", Duration::from_secs(15))
                    .is_some();
                let file = new_file(&before, &files(&captures));
                let detail = match file.as_deref().map(|f| mf::decode_frame_at(f, 5_000_000)) {
                    Some(Ok(frame)) => {
                        // Where the bar's buttons are, and the outline's top strip: the
                        // backdrop under them.
                        let b = (bar.rect.left - m.rect.left, bar.rect.top - m.rect.top);
                        let probes = [
                            (b.0 + (25.0 * scale) as i32, b.1 + (25.0 * scale) as i32),
                            (b.0 + (100.0 * scale) as i32, b.1 + (25.0 * scale) as i32),
                            ((40.0 * scale) as i32, 1),
                            ((300.0 * scale) as i32, 0),
                        ];
                        let seen: Vec<[u8; 3]> =
                            probes.iter().map(|&(x, y)| rgb(&frame, x, y)).collect();
                        ok &= seen.iter().all(|p| near(*p, BACK, 10));
                        format!("under the bar and the outline: {seen:?} (backdrop {BACK:?})")
                    }
                    Some(Err(e)) => {
                        ok = false;
                        format!("unreadable: {e:?}")
                    }
                    None => {
                        ok = false;
                        "no new MP4".into()
                    }
                };
                results.push((
                    "the whole screen: the visible bar inside its top edge".into(),
                    inside,
                    format!("controls {controls:?}"),
                ));
                results.push((
                    "the whole screen: neither the bar nor the outline is in the video".into(),
                    ok,
                    detail,
                ));
            }
            None => results.push(("a full-screen box".into(), false, "no bar".into())),
        }

        // 3. Esc before recording: the box goes, the layer stays.
        let ink = app.ink();
        let small = Rect::new(100, 100, 500, 400);
        match (app.record_box(m.rect, small), &ink) {
            (Some(_), Some(ink)) => {
                app.esc(ink);
                let cancelled = app.wait_for("record box cancelled", Duration::from_secs(5));
                let still_open = app.ink().is_some();
                results.push((
                    "Esc before recording cancels the box, the layer stays".into(),
                    cancelled.is_some() && still_open,
                    String::new(),
                ));
            }
            _ => results.push(("Esc before recording".into(), false, "no box".into())),
        }

        // 4. Discard asks, then leaves nothing.
        let before = files(&captures);
        match app.record_box(m.rect, small) {
            Some(bar) => {
                let mut ok = app.press(bar.hwnd, &tr("record-bar-record"));
                ok &= app
                    .wait_for("recording started", Duration::from_secs(5))
                    .is_some();
                app.record_for(0.5);
                ok &= app.press(bar.hwnd, &tr("record-bar-discard"));
                // The bar asks, in itself: its Discard button now confirms.
                ok &= app.press(bar.hwnd, &tr("record-bar-discard-confirm"));
                ok &= app
                    .wait_for("recording discarded", Duration::from_secs(5))
                    .is_some();
                let left = files(&captures);
                results.push((
                    "Discard asks, then deletes the recording".into(),
                    ok && left == before,
                    format!("{} files before, {} after", before.len(), left.len()),
                ));
            }
            None => results.push(("Discard".into(), false, "no box".into())),
        }

        // 5. Esc while recording: saved, then the layer closes.
        let before = files(&captures);
        match (app.record_box(m.rect, small), app.ink()) {
            (Some(bar), Some(ink)) => {
                let mut ok = app.press(bar.hwnd, &tr("record-bar-record"));
                ok &= app
                    .wait_for("recording started", Duration::from_secs(5))
                    .is_some();
                app.record_for(1.0);
                app.esc(&ink);
                ok &= app
                    .wait_for("recording saved", Duration::from_secs(10))
                    .is_some();
                ok &= app.wait_for(" closed", Duration::from_secs(5)).is_some();
                let file = new_file(&before, &files(&captures));
                let playable = file
                    .as_deref()
                    .is_some_and(|f| mf::probe(f).is_ok_and(|i| i.frames > 0));
                results.push((
                    "Esc while recording saves, then closes the layer".into(),
                    ok && playable,
                    format!("{file:?}"),
                ));
            }
            _ => results.push(("Esc while recording".into(), false, "no box".into())),
        }
    } else {
        results.push(("the layer opened".into(), false, String::new()));
    }

    let quit = app.owned.stop(&exe);
    app.watch("after quitting");
    // At every step, not only at the end: nothing waits for the foreground to come back.
    let took = app.took.borrow().clone();
    results.push((
        "the foreground was the user's throughout".into(),
        took.is_empty(),
        format!(
            "{} the whole run{}",
            window::describe(foreground_before),
            if took.is_empty() {
                String::new()
            } else {
                format!("; taken: {took:?}")
            }
        ),
    ));
    let leftovers: Vec<PathBuf> = files(&captures)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "part"))
        .collect();
    results.push((
        "no partial files left".into(),
        leftovers.is_empty(),
        format!("{leftovers:?}"),
    ));

    println!();
    let mut failed = 0;
    for (name, ok, detail) in &results {
        println!("{} {name}: {detail}", if *ok { "ok    " } else { "FAILED" });
        failed += usize::from(!ok);
    }
    if !quit {
        println!("FAILED it didn't quit when asked");
        failed += 1;
    }
    println!("{} checks, {failed} failed", results.len());
    // Exiting skips destructors: the app, the backdrops and the temp folders go first.
    drop(app);
    drop(temp);
    if failed > 0 {
        std::process::exit(1);
    }
}
