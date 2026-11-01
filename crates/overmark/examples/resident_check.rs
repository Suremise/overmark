//! Checks the resident app against its budgets, driving the real exe from outside:
//!
//! ```text
//! cargo build -p overmark --release --all-targets
//! cargo run -p overmark --release --example resident_check [-- --quick]
//! ```
//!
//! It starts `overmark.exe` (beside this example's folder), measures it idle in the tray (memory
//! and CPU), then opens the layer through its own hotkey window (a `WM_HOTKEY` posted to the
//! app's hidden hotkey window, which is what Windows sends on a key press) and drives it with
//! keys posted to its own ink layer (F, Esc): no global keyboard or mouse input. The app logs
//! each open's timing; this reads them. While the layer is open, Windows Graphics Capture (as a
//! share sees the screen) must see a backdrop of a known colour as it was: the dim is excluded,
//! and frozen frames hold nothing of ours. Then it measures idle again and stops the app.
//!
//! Every wait is for a condition (a log line, a window, a capture frame), bounded by a
//! deadline; only the idle measurements are spans of time. A capture is judged only after a
//! barrier: once the app logs the scene ready, a tiny marker of this check's changes colour, and
//! the first frame showing it is the one judged (`wgc::frame_after_barrier`), so a frame queued
//! before the scene was ready can't pass. The app it
//! starts is stopped however this ends, a failure included ([`Owned`]). `--quick` does 2 rounds
//! and 3 s idles instead of 8 and 10 s, for repeated runs under load. With
//! `OVERMARK_CHECK_FAIL=after-open` it fails on purpose with the layer open, to show the app and
//! its grabber don't outlive it.
//!
//! The layer covers the screen and takes the focus for about half a second each time: run it
//! when you're not typing, with no Overmark running (it stops if one is).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use overmark_core::hotkey::Hotkey;
use overmark_win::hotkeys::to_hotkey;
use overmark_win::win32::Rect;
use overmark_win::win32::capture::close;
use overmark_win::win32::capture::wgc::{Observer, frame_after_barrier};
use overmark_win::win32::dpi;
use overmark_win::win32::process::Process;
use overmark_win::win32::window::{self, Options, Window, WindowInfo, process_windows};
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::WindowsAndMessaging::{WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN};

const WS_EX_TRANSPARENT: u32 = 0x20;
/// How long a capture condition may take to show (a busy PC's capture pipeline included).
const SEEN_WITHIN_MS: u32 = 3000;

/// The app this check started, and only it: stopped when dropped, however the check ends (a
/// panic unwinds through it). Its grabber helper reads the app's pipe, so it ends with it.
struct Owned {
    child: Option<Child>,
}

impl Owned {
    fn pid(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }

    /// Asks the app to quit (the named request, while this child is still the one holding the
    /// name), waits up to `wait` for this child to exit, and kills it if it hasn't. Returns
    /// whether it quit by itself.
    fn stop(&mut self, exe: &std::path::Path, wait: Duration) -> bool {
        let Some(mut child) = self.child.take() else {
            return true;
        };
        if matches!(child.try_wait(), Ok(None)) {
            let _ = Command::new(exe).arg("--quit").status();
        }
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        false
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // A failure's path: no request by name (it might reach another instance), just this
            // child, killed and reaped.
            if matches!(child.try_wait(), Ok(None)) {
                eprintln!(
                    "resident_check: stopping the Overmark it started (pid {})",
                    child.id()
                );
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

struct App {
    owned: Owned,
    lines: mpsc::Receiver<String>,
}

impl App {
    /// The next log line with `needle`, within `timeout`.
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

    fn windows(&self) -> Vec<WindowInfo> {
        process_windows(self.owned.pid())
    }

    /// Polls the app's windows until one matches `want`, within `timeout`.
    fn wait_for_window(
        &self,
        timeout: Duration,
        want: impl Fn(&WindowInfo) -> bool,
    ) -> Option<WindowInfo> {
        let until = Instant::now() + timeout;
        loop {
            if let Some(w) = self.windows().into_iter().find(&want) {
                return Some(w);
            }
            if Instant::now() >= until {
                return None;
            }
            window::pump();
            window::wait_for_message(5);
        }
    }

    /// Polls until none of the app's windows matches `gone`, within `timeout`.
    fn wait_until_none(&self, timeout: Duration, gone: impl Fn(&WindowInfo) -> bool) -> bool {
        let until = Instant::now() + timeout;
        loop {
            if !self.windows().iter().any(&gone) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            window::pump();
            window::wait_for_message(5);
        }
    }
}

fn is_ink(w: &WindowInfo) -> bool {
    // The toolbar's window takes the pointer too; the ink layer is the one titled "Overmark".
    w.class == "OvermarkSurface"
        && w.visible
        && w.ex_style & WS_EX_TRANSPARENT == 0
        && w.title == "Overmark"
}

fn is_veil(w: &WindowInfo) -> bool {
    w.class == "OvermarkSurface" && w.visible && w.alpha.is_some_and(|a| a > 0 && a < 255)
}

fn is_surface(w: &WindowInfo) -> bool {
    w.class == "OvermarkSurface" && w.visible
}

/// A number after `key=` in a log line.
fn field(line: &str, key: &str) -> Option<f64> {
    let at = line.find(&format!("{key}="))? + key.len() + 1;
    line[at..]
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()?
        .parse()
        .ok()
}

/// Memory and CPU over a span of `seconds` idle: the one measurement that is a span of time.
fn idle(app: &App, seconds: u64) -> (f64, f64, f64) {
    let p = Process::open(app.owned.pid()).expect("open the app's process");
    let cpu0 = p.cpu_seconds();
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        while app.lines.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(100));
    }
    let cpu = (p.cpu_seconds() - cpu0) * 1000.0;
    let m = p.memory();
    (
        m.working_set as f64 / 1048576.0,
        m.private as f64 / 1048576.0,
        cpu,
    )
}

fn stats(v: &[f64]) -> String {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if s.is_empty() {
        return "none".into();
    }
    format!(
        "min {:.1}, median {:.1}, max {:.1} ms ({} opens)",
        s[0],
        s[s.len() / 2],
        s[s.len() - 1],
        s.len()
    )
}

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let fail_after_open = std::env::var("OVERMARK_CHECK_FAIL").is_ok_and(|v| v == "after-open");
    let (rounds, idle_s) = if quick { (2, 3) } else { (8, 10) };
    let _ = dpi::enable_per_monitor_v2();
    let exe = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|d| d.parent())
        .unwrap()
        .join("overmark.exe");
    assert!(
        exe.exists(),
        "build it first: cargo build -p overmark --release ({} missing)",
        exe.display()
    );
    // Another Overmark would take the check's launches as "open", over the user's screen.
    if overmark_win::win32::tray::is_running(overmark::APP_ID) {
        eprintln!("resident_check: Overmark is already running; quit it first (tray icon › Quit)");
        std::process::exit(2);
    }
    // Installed mode, with %APPDATA% in a temp folder of the check's own: the user's real
    // settings are never touched, and the check sees where the app writes.
    let appdata = tempfile::tempdir().expect("a temp folder");
    let home = appdata.path().join("Overmark");
    let mut child = Command::new(&exe)
        .env("APPDATA", appdata.path())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start overmark.exe");
    let stderr = child.stderr.take().unwrap();
    // From here on, the app is stopped whatever happens.
    let owned = Owned { child: Some(child) };
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            println!("    | {line}");
            let _ = tx.send(line);
        }
    });
    let mut app = App { owned, lines };
    let mut failures = Vec::new();
    let resident = app.wait_for("resident in the tray", Duration::from_secs(10));
    assert!(resident.is_some(), "the app didn't start");
    // Installed: its settings made in %APPDATA%\Overmark and its log beside them.
    let log = home.join("logs").join("overmark.log");
    let made = home.join("settings.toml").is_file()
        && std::fs::read_to_string(&log).is_ok_and(|t| t.contains("resident in the tray"));
    println!(
        "
Settings and log in the (temp) %APPDATA%: {} ({})",
        if made { "ok" } else { "MISSING" },
        home.display()
    );
    if !made {
        failures.push(format!("no settings.toml or log in {}", home.display()));
    }
    if exe.parent().unwrap().join("settings.toml").exists() {
        failures.push("an installed copy wrote settings beside the exe".into());
    }

    println!("\nIdle in the tray, {idle_s} s:");
    let (ws, private, cpu) = idle(&app, idle_s);
    println!("    working set {ws:.1} MB, private {private:.1} MB, CPU {cpu:.1} ms in {idle_s} s");
    if private > 30.0 || ws > 30.0 {
        failures.push(format!("idle memory {ws:.1} / {private:.1} MB over 30 MB"));
    }
    if cpu > 2.0 * idle_s as f64 {
        failures.push(format!("idle CPU {cpu:.1} ms in {idle_s} s"));
    }

    // The app's hotkey window, and the IDs global-hotkey gives its two hotkeys.
    let hotkey_window = app
        .wait_for_window(Duration::from_secs(5), |w| w.class == "global_hotkey_app")
        .expect("the app's hotkey window");
    let id = |text: &str| to_hotkey(&Hotkey::parse(text).unwrap().unwrap()).id();
    let (draw, frozen) = (id("Win+Shift+D"), id("Win+Shift+F"));
    // MOD_WIN | MOD_SHIFT in the low word, the key in the high one, as Windows sends it.
    let lparam = |vk: u32| ((vk << 16) | 0x0008 | 0x0004) as isize;

    // A backdrop of a known colour, ours, visible to capture: under the dim, a share must
    // still see it as it is.
    let m = dpi::monitors().first().unwrap().clone();
    let patch = Rect::new(
        m.rect.left + 40,
        m.rect.top + 300,
        m.rect.left + 200,
        m.rect.top + 400,
    );
    let backdrop = Window::new(
        patch,
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .unwrap();
    backdrop.fill_gdi(200, 100, 50);
    backdrop.show(false);
    backdrop.fill_gdi(200, 100, 50);
    let probe = (patch.left - m.rect.left + 80, patch.top - m.rect.top + 50);
    // The capture barrier: a tiny marker of ours, top-most, whose colour changes once the
    // scene to judge is ready (the app has logged it). The first frame showing the new colour
    // was composed after that, so it's the one judged; nothing queued before it counts.
    let mark_rect = Rect::new(
        m.rect.left + 8,
        m.rect.top + 8,
        m.rect.left + 14,
        m.rect.top + 14,
    );
    let mark = Window::new(
        mark_rect,
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .unwrap();
    let marks: [((u8, u8, u8), [u8; 4]); 2] = [
        ((0, 255, 0), [0, 255, 0, 255]),
        ((255, 0, 255), [255, 0, 255, 255]),
    ];
    let mark_shows = std::cell::Cell::new(0usize);
    mark.fill_gdi(0, 0, 0);
    mark.show(false);
    let observer = Observer::start(&m, vec![probe, (11, 11)], false, false).unwrap();
    let known = [50u8, 100, 200, 255];
    let pump = || {
        window::pump();
        window::wait_for_message(5);
    };
    // Once the scene is ready: the barrier, then what a share sees at the backdrop in the first
    // frame after it. `(clean, what it saw)`.
    let share_sees_backdrop = || {
        let i = (mark_shows.get() + 1) % 2;
        mark_shows.set(i);
        let ((r, g, b), bgra) = marks[i];
        mark.show(false);
        mark.fill_gdi(r, g, b);
        let f = frame_after_barrier(|| observer.take_frames(), 1, bgra, SEEN_WITHIN_MS, pump);
        let seen = f.map(|f| f.probes[0]);
        (seen.is_some_and(|p| close(p, known, 3)), seen)
    };

    // The control: a dim of our own over the backdrop, not excluded from capture, must be seen
    // (the check can tell a leaked dim), and the backdrop must be seen clean once it's gone.
    let control = Window::new(
        patch,
        Options {
            no_activate: true,
            click_through: true,
            ..Default::default()
        },
        Box::new(|_, msg, _, _| (msg == WM_ERASEBKGND).then_some(LRESULT(1))),
    )
    .unwrap();
    control.fill_gdi(0, 0, 0);
    control.set_opacity(89);
    control.show(false);
    control.fill_gdi(0, 0, 0);
    let (dimmed_clean, dimmed) = share_sees_backdrop();
    drop(control);
    let (clean, base) = share_sees_backdrop();
    println!(
        "
The control: a dim that isn't excluded shows as {dimmed:?} (caught: {}); without it the backdrop shows as {base:?} (clean: {})",
        !dimmed_clean && dimmed.is_some(),
        clean
    );
    if dimmed_clean || dimmed.is_none() || !clean {
        failures
            .push("the control: the check can't tell a leaked dim from a clean backdrop".into());
    }

    let mut live_ms = Vec::new();
    let mut frozen_ms = Vec::new();
    // What making the toolbar took; the visible time above includes it.
    let mut bar_live_ms = Vec::new();
    let mut bar_frozen_ms = Vec::new();
    let mut f_ms = Vec::new();
    let mut dim_results = Vec::new();
    let mut frozen_clean = Vec::new();
    for round in 0..rounds {
        for (name, hk_id, vk, list, bar_list) in [
            ("Live", draw, 0x44u32, &mut live_ms, &mut bar_live_ms),
            (
                "Frozen",
                frozen,
                0x46u32,
                &mut frozen_ms,
                &mut bar_frozen_ms,
            ),
        ] {
            window::post(hotkey_window.hwnd, WM_HOTKEY, hk_id as usize, lparam(vk));
            let Some(line) = app.wait_for(" opened", Duration::from_secs(3)) else {
                failures.push(format!("{name}: the layer didn't open"));
                continue;
            };
            list.push(field(&line, "visible_ms").unwrap_or(f64::MAX));
            match field(&line, "toolbar_ms") {
                Some(ms) => bar_list.push(ms),
                None => failures.push(format!("{name}: no toolbar")),
            }
            if !line.contains(&format!("backdrop=\"{name}\"")) {
                failures.push(format!("{name}: opened as {line}"));
            }
            let Some(ink) = app.wait_for_window(Duration::from_secs(3), is_ink) else {
                failures.push(format!("{name}: no ink layer"));
                continue;
            };
            if fail_after_open {
                panic!("OVERMARK_CHECK_FAIL=after-open: failing on purpose with the layer open");
            }
            if name == "Live" {
                // The veil is up (layered, excluded, dimming), and a share, in a frame composed
                // after the open was asked for, sees the backdrop as it is.
                let veil = app.wait_for_window(Duration::from_secs(3), is_veil);
                let (seen, at) = share_sees_backdrop();
                let ok = veil.is_some() && seen;
                dim_results.push(ok);
                if round == 0 || !ok {
                    println!(
                        "\nThe dim with the layer open: veil window {}, opacity {:?}; a share sees the backdrop as {:?} (it is {:?}): {}",
                        if veil.is_some() { "shown" } else { "MISSING" },
                        veil.as_ref().and_then(|v| v.alpha),
                        at,
                        known,
                        if ok { "excluded, ok" } else { "FAILED" }
                    );
                }
                // F toggles Frozen and back, keeping the layer open.
                for want in ["Frozen", "Live"] {
                    window::post(ink.hwnd, WM_KEYDOWN, 0x46, 0);
                    match app.wait_for(" F", Duration::from_secs(3)) {
                        Some(l) if l.contains(want) => f_ms.push(field(&l, "ms").unwrap_or(0.0)),
                        other => failures.push(format!("F to {want}: {other:?}")),
                    }
                    if want == "Frozen" {
                        frozen_clean.push(("F", share_sees_backdrop()));
                    }
                }
            } else if round == 0 {
                frozen_clean.push(("the Frozen hotkey", share_sees_backdrop()));
            }
            window::post(ink.hwnd, WM_KEYDOWN, 0x1B, 0);
            if app.wait_for("closed", Duration::from_secs(3)).is_none() {
                failures.push(format!("{name}: Esc didn't close it"));
            }
            if !app.wait_until_none(Duration::from_secs(3), is_surface) {
                failures.push(format!("{name}: a layer is still shown after Esc"));
            }
        }
    }
    drop(observer);
    drop(mark);
    drop(backdrop);
    let dirty: Vec<_> = frozen_clean.iter().filter(|(_, (ok, _))| !ok).collect();
    println!(
        "\nFrozen frames as a share sees them at the backdrop (it is {known:?}): {} of {} clean{}",
        frozen_clean.len() - dirty.len(),
        frozen_clean.len(),
        if dirty.is_empty() {
            String::new()
        } else {
            format!("; not: {dirty:?}")
        }
    );
    if !dirty.is_empty() || frozen_clean.is_empty() {
        failures.push("a frozen frame had something of ours in it".into());
    }
    if dim_results.is_empty() || dim_results.iter().any(|ok| !ok) {
        failures.push("the dim's capture check".into());
    }
    println!(
        "\nHotkey to a visible layer and toolbar (WM_HOTKEY's arrival to DWM's first frame with them):"
    );
    println!("    Live:   {}", stats(&live_ms));
    println!("    Frozen: {}", stats(&frozen_ms));
    println!("    F (Live to Frozen, Frozen to Live): {:?} ms", f_ms);
    println!("  of which making the toolbar (it comes in the same frame as the layer):");
    println!("    Live:   {}", stats(&bar_live_ms));
    println!("    Frozen: {}", stats(&bar_frozen_ms));
    for (name, v) in [("Live", &live_ms), ("Frozen", &frozen_ms)] {
        if v.iter().any(|&t| t >= 100.0) {
            failures.push(format!("{name}: an open took 100 ms or more"));
        }
    }

    println!("\nIdle again after {} opens, {idle_s} s:", 2 * rounds);
    let (ws, private, cpu) = idle(&app, idle_s);
    println!("    working set {ws:.1} MB, private {private:.1} MB, CPU {cpu:.1} ms in {idle_s} s");
    if private > 30.0 || ws > 30.0 {
        failures.push(format!(
            "idle memory after use {ws:.1} / {private:.1} MB over 30 MB"
        ));
    }
    if cpu > 2.0 * idle_s as f64 {
        failures.push(format!("idle CPU after use {cpu:.1} ms in {idle_s} s"));
    }

    if !app.owned.stop(&exe, Duration::from_secs(5)) {
        failures.push("it didn't quit when asked (stopped)".into());
    }
    if failures.is_empty() {
        println!("\nresident_check: ok");
    } else {
        println!("\nresident_check: FAILED");
        for f in &failures {
            println!("    {f}");
        }
        // `exit` skips destructors: the app is already stopped above.
        std::process::exit(1);
    }
}
