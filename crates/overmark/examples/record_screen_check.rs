//! Checks screen-capture recording (no drawing layer) on the real exe, driving it from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example record_screen_check
//! ```
//!
//! It starts `overmark.exe --no-activate --no-grabber` with `%APPDATA%` and the captures folder
//! in temp folders of its own, over a window of the check's own standing in for "the app under
//! the box", whose colour it changes while recording. Record a region's hotkey is posted to the
//! app's own hotkey window; the box is dragged with mouse messages posted to the app's own
//! selection window; the bar's buttons are pressed through UI Automation on the app's own
//! window. Nothing global: no input is sent, and nothing is activated (the app runs with
//! `--no-activate`, and the foreground is checked after every step).
//!
//! - **A box at the monitor's top** (so the bar sits inside its top edge): record, the app
//!   under it changes colour, pause, resume, then **the hotkey again** stops and saves. The video
//!   shows both colours, and under the bar and the outline it shows the app, not them.
//! - The outline lets the pointer through to the app under it (`WindowFromPoint` there finds
//!   the app's window).
//! - **Kept on top:** a top-most window of the check's own put over the bar is under it again
//!   within a second.
//! - Esc on the selection cancels, and so does the hotkey pressed during it; the hotkey on a box
//!   not yet recording closes it; Discard asks, then leaves no file; with the drawing layer
//!   open, the hotkey opens nothing and the layer stays, and says so even with capture notices
//!   turned off (which the check runs with: a plain save then shows no notice).
//!
//! Run it with no Overmark running (it stops if one is). It stops the app it started however it
//! ends, and its temp folders go with it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark_core::hotkey::Hotkey;
use overmark_core::i18n::tr;
use overmark_core::settings::{DEFAULT_RECORD_REGION_HOTKEY, Settings};
use overmark_win::hotkeys::to_hotkey;
use overmark_win::win32::uia::Uia;
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use overmark_win::win32::{Rect, affinity, dpi, mf};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

/// The app under the box: RGB (200, 100, 50), then (40, 160, 90) while recording.
const FIRST: [u8; 3] = [200, 100, 50];
const SECOND: [u8; 3] = [40, 160, 90];

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
                    "record_screen_check: stopping the Overmark it started (pid {})",
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
    hotkeys: Option<WindowInfo>,
    /// The foreground when the check began, and every step after which it wasn't that.
    foreground: Option<HWND>,
    took: std::cell::RefCell<Vec<String>>,
}

impl App {
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

    /// Record a region's hotkey, posted to the app's own hotkey window.
    fn hotkey(&self) {
        let Some(h) = &self.hotkeys else { return };
        let hk = Hotkey::parse(DEFAULT_RECORD_REGION_HOTKEY)
            .unwrap()
            .unwrap();
        let id = to_hotkey(&hk).id() as usize;
        // MOD_ALT | MOD_WIN, and V.
        let lparam = ((0x56u32 << 16) | 0x0008 | 0x0001) as isize;
        window::post(h.hwnd, WM_HOTKEY, id, lparam);
        self.watch("the hotkey");
    }

    fn press(&self, hwnd: HWND, name: &str) -> bool {
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

    /// The selection window over `monitor`, once it's up.
    fn selection(&self, monitor: Rect) -> Option<WindowInfo> {
        let title = tr("window-selection-name");
        self.window(Duration::from_secs(5), |w| {
            w.title == title && w.visible && w.rect == monitor
        })
    }

    /// The hotkey, then `rect` (its monitor's pixels) dragged: the bar's window once it's up.
    fn record_box(&self, monitor: Rect, rect: Rect) -> Option<WindowInfo> {
        self.hotkey();
        let sel = self.selection(monitor)?;
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
        self.wait_for("a box to record", Duration::from_secs(5))?;
        self.titled("window-record-bar-name")
    }

    fn record_for(&self, secs: f64) {
        let until = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < until {
            window::pump();
            window::wait_for_message(20);
            self.watch("recording");
        }
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

fn files(folder: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn new_file(before: &[PathBuf], after: &[PathBuf]) -> Option<PathBuf> {
    let new: Vec<&PathBuf> = after.iter().filter(|p| !before.contains(p)).collect();
    (new.len() == 1).then(|| new[0].clone())
}

fn rgb(frame: &(u32, u32, Vec<u8>), x: i32, y: i32) -> [u8; 3] {
    let i = ((y as u32 * frame.0 + x as u32) * 4) as usize;
    [frame.2[i + 2], frame.2[i + 1], frame.2[i]]
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
            "record_screen_check: Overmark is already running; quit it first (tray icon › Quit)"
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
    // Capture notices off: a plain "saved" then shows none, but a problem or an instruction
    // (the drawing layer is open) still does.
    settings.capture.toast = false;
    std::fs::create_dir_all(appdata.join("Overmark")).unwrap();
    settings
        .save(&appdata.join("Overmark").join("settings.toml"))
        .unwrap();

    let m = dpi::monitors().first().unwrap().clone();
    let scale = m.scale();
    // "The app under the box": a window of the check's own, never activated.
    let app_rect = Rect::new(
        m.rect.left + 60,
        m.rect.top,
        m.rect.left + 1000,
        m.rect.top + 480,
    );
    let under = Window::new(
        app_rect,
        Options {
            no_activate: true,
            title: Some("record_screen_check app"),
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .expect("the app's window");
    under.show(false);
    under.fill_gdi(FIRST[0], FIRST[1], FIRST[2]);

    let mut child = Command::new(&exe)
        .args(["--no-activate", "--no-grabber"])
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
        hotkeys: None,
        foreground: foreground_before,
        took: std::cell::RefCell::new(Vec::new()),
    };
    let mut results: Vec<(String, bool, String)> = Vec::new();
    let started = app
        .wait_for("resident in the tray", Duration::from_secs(10))
        .is_some();
    app.hotkeys = app.window(Duration::from_secs(5), |w| w.class == "global_hotkey_app");
    results.push((
        "the app started, its hotkeys registered".into(),
        started && app.hotkeys.is_some(),
        String::new(),
    ));

    if started {
        // 1. A box at the top of the monitor: the bar inside its top edge.
        let boxed = Rect::new(100, 0, 900, 400);
        let before = files(&captures);
        match app.record_box(m.rect, boxed) {
            Some(bar) => {
                let controls = app.bar_controls(&bar);
                let outline_title = tr("window-record-outline-name");
                let outline: Vec<WindowInfo> = process_windows(app.owned.pid())
                    .into_iter()
                    .filter(|w| w.title == outline_title && w.visible)
                    .collect();
                results.push((
                    "the bar and outline are excluded from capture".into(),
                    affinity::is_excluded(bar.hwnd)
                        && outline.len() == 4
                        && outline.iter().all(|w| affinity::is_excluded(w.hwnd)),
                    format!("{} outline strips", outline.len()),
                ));
                results.push((
                    "the visible bar is inside the box's top edge".into(),
                    controls.is_some_and(|c| {
                        c.top > m.rect.top
                            && c.top <= m.rect.top + (24.0 * scale) as i32
                            && c.left >= m.rect.left + boxed.left
                    }),
                    format!("controls {controls:?}"),
                ));
                // The outline lets the pointer through: on its left strip (just outside the
                // box), the window found is the app's.
                let on_outline = (m.rect.left + boxed.left - 1, m.rect.top + 200);
                let found = window::window_at(on_outline.0, on_outline.1);
                results.push((
                    "the outline lets the pointer through to the app".into(),
                    found == under.hwnd(),
                    format!("at {on_outline:?}: {}", window::describe(Some(found))),
                ));
                let mut ok = app.press(bar.hwnd, &tr("record-bar-record"));
                ok &= app
                    .wait_for("recording started", Duration::from_secs(5))
                    .is_some();
                app.record_for(1.0);
                // The app under the box changes, as it would while you use it.
                under.fill_gdi(SECOND[0], SECOND[1], SECOND[2]);
                app.record_for(0.6);
                // Another always-on-top window over the bar: the bar comes back over it.
                let cover = Window::new(
                    bar.rect,
                    Options {
                        no_activate: true,
                        title: Some("record_screen_check cover"),
                        ..Default::default()
                    },
                    Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
                )
                .expect("a cover window");
                cover.show(false);
                let covered = window::is_above(cover.hwnd(), bar.hwnd);
                let until = Instant::now() + Duration::from_millis(1500);
                while window::is_above(cover.hwnd(), bar.hwnd) && Instant::now() < until {
                    window::pump();
                    window::wait_for_message(20);
                }
                let back = window::is_above(bar.hwnd, cover.hwnd());
                drop(cover);
                results.push((
                    "the bar goes back over another top-most window".into(),
                    covered && back,
                    format!("covered at first: {covered}; back on top within 1.5 s: {back}"),
                ));
                ok &= app.press(bar.hwnd, &tr("record-bar-pause"));
                app.record_for(0.6);
                ok &= app.press(bar.hwnd, &tr("record-bar-resume"));
                app.record_for(0.6);
                // The hotkey again stops and saves.
                app.hotkey();
                ok &= app
                    .wait_for("recording saved", Duration::from_secs(10))
                    .is_some();
                let quiet = app
                    .window(Duration::from_secs(1), |w| {
                        w.title == tr("window-notice-name") && w.visible
                    })
                    .is_none();
                results.push((
                    "with capture notices off, a plain save shows none".into(),
                    quiet,
                    String::new(),
                ));
                let bar_gone = app
                    .window(Duration::from_secs(2), |w| {
                        w.title == tr("window-record-bar-name") && w.visible
                    })
                    .is_none();
                let file = new_file(&before, &files(&captures));
                let detail = match &file {
                    Some(f) => match (
                        mf::probe(f),
                        mf::decode_frame_at(f, 5_000_000),
                        mf::decode_frame_at(f, 25_000_000),
                    ) {
                        (Ok(info), Ok(early), Ok(late)) => {
                            let mid = (boxed.width() / 2, boxed.height() / 2);
                            let (a, b) = (rgb(&early, mid.0, mid.1), rgb(&late, mid.0, mid.1));
                            // Where the bar's controls are, and the outline's top strip: the
                            // app under them.
                            let c = controls.unwrap_or_default();
                            let under_bar = rgb(
                                &early,
                                c.left - m.rect.left - boxed.left + (20.0 * scale) as i32,
                                c.top - m.rect.top + (15.0 * scale) as i32,
                            );
                            let under_outline = rgb(&early, 300, 0);
                            let secs = info.duration_100ns as f64 / 1e7;
                            ok &= near(a, FIRST, 10)
                                && near(b, SECOND, 10)
                                && near(under_bar, FIRST, 10)
                                && near(under_outline, FIRST, 10)
                                // 1.6 s, the cover's wait, 0.6 paused (left out), 0.6 s.
                                && (2.0..4.5).contains(&secs);
                            format!(
                                "{} ({}x{}, {secs:.2} s); the app {a:?} then {b:?}; under the bar {under_bar:?}, under the outline {under_outline:?}",
                                f.file_name().unwrap().to_string_lossy(),
                                info.width,
                                info.height
                            )
                        }
                        (a, b, c) => {
                            ok = false;
                            format!(
                                "unreadable: {:?} {:?} {:?}",
                                a.err(),
                                b.err().map(|_| ()),
                                c.err().map(|_| ())
                            )
                        }
                    },
                    None => {
                        ok = false;
                        format!("no new MP4 in {}", captures.display())
                    }
                };
                results.push((
                    "record, the app changes, pause, resume, the hotkey again saves".into(),
                    ok && bar_gone,
                    detail,
                ));
            }
            None => results.push(("a box to record".into(), false, "no bar".into())),
        }

        // 2. Esc on the selection cancels.
        app.hotkey();
        match app.selection(m.rect) {
            Some(sel) => {
                window::post_key(sel.hwnd, 0x1B);
                app.watch("Esc");
                let cancelled = app
                    .wait_for("choosing a box to record cancelled", Duration::from_secs(5))
                    .is_some();
                results.push((
                    "Esc on the selection cancels".into(),
                    cancelled,
                    String::new(),
                ));
            }
            None => results.push(("Esc on the selection".into(), false, "no selection".into())),
        }

        // 2b. The hotkey pressed again during the selection cancels it.
        app.hotkey();
        match app.selection(m.rect) {
            Some(_) => {
                app.hotkey();
                let cancelled = app
                    .wait_for("choosing a box to record cancelled", Duration::from_secs(5))
                    .is_some();
                results.push((
                    "the hotkey during the selection cancels it".into(),
                    cancelled,
                    String::new(),
                ));
            }
            None => results.push((
                "the hotkey during the selection".into(),
                false,
                "no selection".into(),
            )),
        }

        // 2c. With the drawing layer open, the hotkey only says so: no selection, the layer stays.
        if let Some(h) = &app.hotkeys {
            let draw = to_hotkey(&Hotkey::parse("Win+Shift+D").unwrap().unwrap()).id() as usize;
            window::post(
                h.hwnd,
                WM_HOTKEY,
                draw,
                ((0x44u32 << 16) | 0x0008 | 0x0004) as isize,
            );
            app.watch("opened the layer");
        }
        let opened = app.wait_for(" opened", Duration::from_secs(3)).is_some();
        app.hotkey();
        let said = app
            .wait_for("SayCloseTheLayer", Duration::from_secs(3))
            .is_some();
        let told = app
            .window(Duration::from_secs(2), |w| {
                w.title == tr("window-notice-name") && w.visible
            })
            .is_some();
        let no_selection = app
            .window(Duration::from_millis(500), |w| {
                w.title == tr("window-selection-name") && w.visible
            })
            .is_none();
        let ink = app.window(Duration::from_secs(2), |w| {
            w.class == "OvermarkSurface" && w.visible && w.title == "Overmark"
        });
        results.push((
            "with the drawing layer open, the hotkey opens nothing and the layer stays".into(),
            opened && said && told && no_selection && ink.is_some(),
            format!(
                "opened {opened}, said {said}, the notice shown with capture notices off {told}, no selection {no_selection}"
            ),
        ));
        if let Some(ink) = &ink {
            window::post_key(ink.hwnd, 0x1B);
            app.watch("Esc");
            let _ = app.wait_for(" closed", Duration::from_secs(3));
        }

        // 3. The hotkey on a box not yet recording closes it.
        let small = Rect::new(150, 100, 550, 380);
        match app.record_box(m.rect, small) {
            Some(_) => {
                app.hotkey();
                let closed = app
                    .wait_for("record box cancelled", Duration::from_secs(5))
                    .is_some();
                results.push((
                    "the hotkey closes a box not yet recording".into(),
                    closed,
                    String::new(),
                ));
            }
            None => results.push(("the hotkey before recording".into(), false, "no box".into())),
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
                ok &= app.press(bar.hwnd, &tr("record-bar-discard-confirm"));
                ok &= app
                    .wait_for("recording discarded", Duration::from_secs(5))
                    .is_some();
                let after = files(&captures);
                results.push((
                    "Discard asks, then deletes the recording".into(),
                    ok && after == before,
                    format!("{} files before, {} after", before.len(), after.len()),
                ));
            }
            None => results.push(("Discard".into(), false, "no box".into())),
        }
    }

    let quit = app.owned.stop(&exe);
    app.watch("after quitting");
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
    drop(app);
    drop(under);
    drop(temp);
    if failed > 0 {
        std::process::exit(1);
    }
}
