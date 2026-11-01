//! Where settings live at start-up: the portable marker, a `settings.toml`
//! beside the exe, installed in `%APPDATA%` (a temp folder here), a folder that can't be
//! written to, and a file that can't be read. Each test works in temp folders of its own.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use overmark::config::Startup;
use overmark_core::settings::Settings;
use overmark_win::paths::{AppPaths, Mode, PORTABLE_MARKER};

const APP: &str = "Overmark";

/// An exe folder and an `%APPDATA%`, in a temp folder of the test's own.
fn folders() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let exe_dir = temp.path().join("app");
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&exe_dir).unwrap();
    fs::create_dir_all(&appdata).unwrap();
    (temp, exe_dir, appdata)
}

fn start(exe_dir: &Path, appdata: &Path) -> Startup {
    Startup::load(AppPaths::resolve(APP, exe_dir, Some(appdata)))
}

/// Everything under `dir`, as relative paths, sorted.
fn listing(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() {
                out.push(format!("{name}\\"));
                out.extend(
                    listing(&e.path())
                        .into_iter()
                        .map(|n| format!("{name}\\{n}")),
                );
            } else {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_marker_makes_it_portable_and_makes_settings_beside_the_exe() {
    let (_t, exe_dir, appdata) = folders();
    fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
    let s = start(&exe_dir, &appdata);
    let about = s.about();
    assert_eq!(about.mode, Some(Mode::Portable));
    assert_eq!(about.mode_label().as_deref(), Some("Portable"));
    assert_eq!(about.settings_file, Some(exe_dir.join("settings.toml")));
    assert!(s.created && s.can_save && s.notices.is_empty());
    assert_eq!(
        Settings::load(&exe_dir.join("settings.toml"))
            .unwrap()
            .settings,
        Settings::default()
    );
    // Nothing in %APPDATA%.
    assert!(listing(&appdata).is_empty(), "{:?}", listing(&appdata));
    assert_eq!(listing(&exe_dir), ["portable", "settings.toml"]);
}

#[test]
fn settings_beside_the_exe_make_it_portable_and_are_read() {
    let (_t, exe_dir, appdata) = folders();
    let mut custom = Settings::default();
    custom.drawing.dim = 0.5;
    custom.save(&exe_dir.join("settings.toml")).unwrap();
    let s = start(&exe_dir, &appdata);
    assert_eq!(s.about().mode, Some(Mode::Portable));
    assert!(!s.created && s.can_save && s.notices.is_empty());
    assert_eq!(s.settings.drawing.dim, 0.5);
    assert!(listing(&appdata).is_empty());
}

#[test]
fn installed_lives_in_appdata() {
    let (_t, exe_dir, appdata) = folders();
    let s = start(&exe_dir, &appdata);
    let about = s.about();
    let root = appdata.join("Overmark");
    assert_eq!(about.mode, Some(Mode::Installed));
    assert_eq!(about.mode_label().as_deref(), Some("Installed"));
    assert_eq!(about.settings_file, Some(root.join("settings.toml")));
    assert_eq!(about.logs_dir, Some(root.join("logs")));
    assert!(s.created && s.can_save && s.notices.is_empty());
    assert!(root.join("settings.toml").is_file());
    // Nothing beside the exe: it stays installed next time too.
    assert!(listing(&exe_dir).is_empty(), "{:?}", listing(&exe_dir));
    let again = start(&exe_dir, &appdata);
    assert!(!again.created && again.about().mode == Some(Mode::Installed));
}

#[test]
fn installed_without_appdata_runs_in_memory_and_says_so() {
    let (_t, exe_dir, _) = folders();
    let s = Startup::load(AppPaths::resolve(APP, &exe_dir, None));
    assert!(s.paths.is_none() && !s.can_save);
    assert_eq!(s.notices.len(), 1);
    assert!(
        s.notices[0].body.contains("%APPDATA%"),
        "{:?}",
        s.notices[0]
    );
    assert!(listing(&exe_dir).is_empty());
}

/// The SID of the account this test runs as (`whoami /user`): the principal to deny, which
/// `%USERNAME%` needn't be (a sandbox or service account runs under another).
/// A Windows tool by its full path, so a `whoami` or `icacls` elsewhere on PATH (Git's
/// coreutils have a `whoami`) is never the one run.
fn system32(tool: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join(tool)
}

fn my_sid() -> String {
    let out = Command::new(system32("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("whoami");
    let text = String::from_utf8_lossy(&out.stdout);
    // "domain\user","S-1-5-21-…"
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned();
    assert!(sid.starts_with("S-1-"), "no SID from whoami: {text:?}");
    sid
}

/// A folder this test's own account can't write to: its SID is denied creating files and
/// folders in it (`icacls /deny *SID`), as on a locked stick or a read-only share. The same SID
/// is restored when dropped.
struct Unwritable {
    dir: PathBuf,
    sid: String,
}

impl Unwritable {
    fn deny(dir: &Path) -> Unwritable {
        let sid = my_sid();
        let out = Command::new(system32("icacls.exe"))
            .arg(dir)
            .args(["/deny", &format!("*{sid}:(OI)(CI)(WD,AD,WA,WEA,DC)")])
            .output()
            .expect("icacls");
        assert!(out.status.success(), "icacls /deny failed: {out:?}");
        Unwritable {
            dir: dir.to_path_buf(),
            sid,
        }
    }
}

impl Drop for Unwritable {
    fn drop(&mut self) {
        let _ = Command::new(system32("icacls.exe"))
            .arg(&self.dir)
            .args(["/remove:d", &format!("*{}", self.sid)])
            .output();
    }
}

#[test]
fn a_portable_folder_that_cant_be_written_runs_in_memory_and_says_so() {
    let (_t, exe_dir, appdata) = folders();
    fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
    let before = listing(&exe_dir);
    let _locked = Unwritable::deny(&exe_dir);
    // The deny holds for this process: a file can't be made there now.
    assert!(
        fs::write(exe_dir.join("try.txt"), "x").is_err(),
        "the folder is still writable to this account"
    );
    let s = start(&exe_dir, &appdata);
    assert_eq!(s.about().mode, Some(Mode::Portable));
    assert!(!s.can_save && !s.created);
    assert_eq!(s.logs_dir(), None, "no logs where nothing can be written");
    assert_eq!(s.settings, Settings::default());
    assert_eq!(s.notices.len(), 1, "{:?}", s.notices);
    assert!(s.notices[0].body.contains("portable"), "{:?}", s.notices[0]);
    assert!(
        s.notices[0].body.contains(&exe_dir.display().to_string()),
        "{:?}",
        s.notices[0]
    );
    drop(_locked);
    // Nothing written there, and nothing anywhere else instead.
    assert_eq!(listing(&exe_dir), before);
    assert!(listing(&appdata).is_empty(), "{:?}", listing(&appdata));
}

#[test]
fn a_corrupt_file_falls_back_to_the_defaults_with_a_notice_and_is_kept() {
    let (_t, exe_dir, appdata) = folders();
    let file = exe_dir.join("settings.toml");
    let broken = "[drawing]\ndim = \"very\"\n[general\n";
    fs::write(&file, broken).unwrap();
    let s = start(&exe_dir, &appdata);
    assert_eq!(s.about().mode, Some(Mode::Portable));
    assert_eq!(s.settings, Settings::default());
    assert!(!s.can_save, "a broken file isn't overwritten");
    assert_eq!(s.notices.len(), 1, "{:?}", s.notices);
    assert_eq!(s.notices[0].title, "Overmark's settings can't be read");
    assert!(
        s.notices[0].body.contains("settings.toml") && s.notices[0].body.contains("line"),
        "{:?}",
        s.notices[0]
    );
    // The user's file is left as it was, for them to fix.
    assert_eq!(fs::read_to_string(&file).unwrap(), broken);
}

#[test]
fn the_pictures_token_resolves_to_the_real_pictures_folder_in_both_modes() {
    let pictures = overmark_win::win32::known_folders::pictures().expect("a Pictures folder");
    for portable in [true, false] {
        let (_t, exe_dir, appdata) = folders();
        if portable {
            fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
        }
        let s = start(&exe_dir, &appdata);
        assert_eq!(s.captures_folder(&pictures), pictures.join("Overmark"));
    }
}
