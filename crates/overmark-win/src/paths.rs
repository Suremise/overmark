//! Where Overmark keeps its settings and logs: portable (beside the exe) or installed
//! (`%APPDATA%\Overmark\`), the usual choice for Windows apps.
//!
//! Copied from Replica's `replica-win/src/paths.rs` ("ready to copy"), with its tests. A file
//! named `portable` (empty, no extension), or a `settings.toml`, beside the exe makes the copy
//! portable, as with ShareX, Notepad++, Everything and KeePass.
//!
//! A portable folder that can't be written to stays the app's folder: [`AppPaths::is_writable`]
//! is then false, and the app says so and keeps its settings in memory. Nothing falls back to
//! another folder silently.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// The file beside the exe that turns portable mode on (empty, no extension).
pub const PORTABLE_MARKER: &str = "portable";
/// The settings file. Beside the exe, it also turns portable mode on.
pub const SETTINGS_FILE: &str = crate::core::settings::FILE_NAME;
/// The logs folder, inside the app's folder.
pub const LOGS_DIR: &str = "logs";

/// Portable or installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Everything beside the exe.
    Portable,
    /// Everything in `%APPDATA%\<App>\`.
    Installed,
}

impl Mode {
    /// `portable` or `installed`, for logs (not translated).
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Portable => "portable",
            Mode::Installed => "installed",
        }
    }
}

/// Why the app's folder couldn't be worked out.
#[derive(Debug)]
pub enum PathsError {
    /// The exe's own path isn't known.
    NoExe(io::Error),
    /// Installed, and `%APPDATA%` isn't set.
    NoAppData,
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathsError::NoExe(e) => f.write_str(&crate::core::tr!(
                "error-exe-folder-unknown",
                reason = e.to_string()
            )),
            PathsError::NoAppData => f.write_str(&crate::core::i18n::tr("error-no-appdata")),
        }
    }
}

impl std::error::Error for PathsError {}

/// Where the app's files live, and whether it can write there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    mode: Mode,
    root: PathBuf,
    exe_dir: PathBuf,
    writable: bool,
}

impl AppPaths {
    /// Works out the folders for the running exe, reading `%APPDATA%` for an installed copy.
    pub fn detect(app_name: &str) -> Result<Self, PathsError> {
        let exe = std::env::current_exe().map_err(PathsError::NoExe)?;
        let exe_dir = exe.parent().ok_or_else(|| {
            PathsError::NoExe(io::Error::new(io::ErrorKind::NotFound, "no parent folder"))
        })?;
        let appdata = std::env::var_os("APPDATA").filter(|v| !v.is_empty());
        Self::resolve(app_name, exe_dir, appdata.map(PathBuf::from).as_deref())
    }

    /// Works out the folders for an exe in `exe_dir`, with `%APPDATA%` given. Creates the
    /// installed folder if it's missing; writes nothing anywhere else.
    pub fn resolve(
        app_name: &str,
        exe_dir: &Path,
        appdata: Option<&Path>,
    ) -> Result<Self, PathsError> {
        let (mode, root) = if is_portable(exe_dir) {
            (Mode::Portable, exe_dir.to_path_buf())
        } else {
            let root = appdata.ok_or(PathsError::NoAppData)?.join(app_name);
            // A failure shows as not writable, below.
            let _ = fs::create_dir_all(&root);
            (Mode::Installed, root)
        };
        let writable = can_write(&root);
        Ok(Self {
            mode,
            root,
            exe_dir: exe_dir.to_path_buf(),
            writable,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_portable(&self) -> bool {
        self.mode == Mode::Portable
    }

    /// The app's folder: settings and logs live here.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder the exe runs from.
    pub fn exe_dir(&self) -> &Path {
        &self.exe_dir
    }

    /// Whether the app's folder could be written to when the paths were worked out. When it's
    /// false, keep settings in memory and tell the user; don't write elsewhere.
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join(SETTINGS_FILE)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }
}

fn is_portable(exe_dir: &Path) -> bool {
    exe_dir.join(PORTABLE_MARKER).is_file() || exe_dir.join(SETTINGS_FILE).is_file()
}

/// Creates and removes a probe file: the only reliable test on Windows, where a folder's
/// read-only attribute doesn't stop files being created but its permissions do.
///
/// The probe is a new file this call makes (`create_new`, atomically): a name that's already
/// taken (a file, a folder, a hard link to the settings) is left alone and another name tried,
/// and only the file this call created is removed. Nothing that was there is ever opened for
/// writing: a truncating open could empty a file that held the name.
fn can_write(dir: &Path) -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for _ in 0..64 {
        let mut name = OsString::from(".write-probe-");
        name.push(format!(
            "{}-{nanos:08x}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let probe = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                let _ = fs::remove_file(&probe);
                return true;
            }
            // Someone else's file by that name: try another name.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let exe_dir = temp.path().join("app");
        let appdata = temp.path().join("appdata");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&appdata).unwrap();
        (temp, exe_dir, appdata)
    }

    #[test]
    fn the_marker_makes_it_portable() {
        let (_temp, exe_dir, appdata) = folders();
        fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        assert_eq!(paths.mode(), Mode::Portable);
        assert_eq!(paths.root(), exe_dir);
        assert_eq!(paths.settings_file(), exe_dir.join("settings.toml"));
        assert_eq!(paths.logs_dir(), exe_dir.join("logs"));
        assert!(paths.is_writable());
        assert!(!appdata.join("Overmark").exists());
    }

    #[test]
    fn settings_beside_the_exe_make_it_portable() {
        let (_temp, exe_dir, appdata) = folders();
        fs::write(exe_dir.join(SETTINGS_FILE), "").unwrap();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        assert_eq!(paths.mode(), Mode::Portable);
    }

    #[test]
    fn a_folder_named_portable_isnt_the_marker() {
        let (_temp, exe_dir, appdata) = folders();
        fs::create_dir(exe_dir.join(PORTABLE_MARKER)).unwrap();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        assert_eq!(paths.mode(), Mode::Installed);
    }

    #[test]
    fn installed_lives_in_appdata() {
        let (_temp, exe_dir, appdata) = folders();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        assert_eq!(paths.mode(), Mode::Installed);
        assert_eq!(paths.root(), appdata.join("Overmark"));
        assert!(paths.root().is_dir());
        assert_eq!(paths.exe_dir(), exe_dir);
        assert!(paths.is_writable());
    }

    #[test]
    fn installed_needs_appdata() {
        let (_temp, exe_dir, _) = folders();
        let error = AppPaths::resolve("Overmark", &exe_dir, None).unwrap_err();
        assert!(matches!(error, PathsError::NoAppData));
    }

    const SETTINGS_TEXT: &str = "[drawing]\ndim = 0.5\n";

    /// Files that hold names a probe might use are never touched: one with data at the old
    /// fixed name, and a hard link to the settings file.
    #[test]
    fn the_probe_never_touches_a_file_that_was_there() {
        let (_temp, exe_dir, appdata) = folders();
        fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
        let settings = exe_dir.join(SETTINGS_FILE);
        fs::write(&settings, SETTINGS_TEXT).unwrap();
        let pid = std::process::id();
        let squatter = exe_dir.join(format!(".write-probe-{pid}"));
        fs::write(&squatter, "someone's data").unwrap();
        let linked = exe_dir.join(format!(".write-probe-{pid}-link"));
        fs::hard_link(&settings, &linked).unwrap();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        assert!(paths.is_writable());
        assert_eq!(fs::read_to_string(&squatter).unwrap(), "someone's data");
        assert_eq!(fs::read_to_string(&settings).unwrap(), SETTINGS_TEXT);
        assert_eq!(fs::read_to_string(&linked).unwrap(), SETTINGS_TEXT);
        // Nothing added (the probe is gone) or removed.
        assert_eq!(fs::read_dir(&exe_dir).unwrap().count(), 4);
    }

    /// Names that look like probes, here hard links to the settings, are skipped, never
    /// opened: the probe makes a name of its own, and the settings stay as they were.
    #[test]
    fn a_taken_name_is_skipped_not_reused() {
        let (_temp, exe_dir, _) = folders();
        let precious = exe_dir.join(SETTINGS_FILE);
        fs::write(&precious, SETTINGS_TEXT).unwrap();
        let pid = std::process::id();
        for n in 0..8 {
            fs::hard_link(&precious, exe_dir.join(format!(".write-probe-{pid}-{n}"))).unwrap();
        }
        assert!(can_write(&exe_dir));
        assert_eq!(fs::read_to_string(&precious).unwrap(), SETTINGS_TEXT);
        assert_eq!(fs::read_dir(&exe_dir).unwrap().count(), 9);
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let (_temp, exe_dir, _) = folders();
        assert!(can_write(&exe_dir));
        assert_eq!(fs::read_dir(&exe_dir).unwrap().count(), 0);
    }
}
