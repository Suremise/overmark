//! Where Overmark's settings come from at start-up, and what to tell the user about them
//! portable or installed, the settings file loaded or made, and a notice for a
//! folder that can't be written to or a file that can't be read. Nothing is ever written
//! anywhere but the app's own folder; when that can't be written, settings stay in memory.

use std::path::{Path, PathBuf};

use overmark_core::i18n::tr;
use overmark_core::settings::{Settings, Warning};
use overmark_core::tr;
use overmark_win::paths::{AppPaths, Mode, PathsError};

/// Something to tell the user on start, as a tray notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// For Settings' About section: how this copy runs and where its files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    /// Portable or installed; `None` if the app's folder couldn't be worked out.
    pub mode: Option<Mode>,
    /// The settings file (it may not be writable: see `can_save`).
    pub settings_file: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    /// Whether settings changes can be saved there this run.
    pub can_save: bool,
}

impl About {
    /// "Portable" or "Installed", for the About section.
    pub fn mode_label(&self) -> Option<String> {
        self.mode.map(|m| match m {
            Mode::Portable => tr("settings-mode-portable"),
            Mode::Installed => tr("settings-mode-installed"),
        })
    }
}

/// The settings and where they live, as worked out at start-up.
#[derive(Debug, Clone)]
pub struct Startup {
    pub paths: Option<AppPaths>,
    pub settings: Settings,
    /// What the file had that was ignored or changed (shown in Settings, logged).
    pub warnings: Vec<Warning>,
    /// What to tell the user now, most important first.
    pub notices: Vec<Notice>,
    /// Whether settings can be saved this run: false when the folder can't be written to or
    /// the file couldn't be read (so a mistake in it isn't overwritten).
    pub can_save: bool,
    /// Whether this run made the settings file (a first run).
    pub created: bool,
}

impl Startup {
    /// From the paths `AppPaths::detect` found, or its error.
    pub fn load(paths: Result<AppPaths, PathsError>) -> Startup {
        match paths {
            Ok(p) => Self::from_paths(p),
            Err(error) => Startup {
                paths: None,
                settings: Settings::default(),
                warnings: Vec::new(),
                notices: vec![Notice {
                    title: tr("error-folder-not-writable-title"),
                    body: tr!("error-app-folder-unknown", detail = error.to_string()),
                }],
                can_save: false,
                created: false,
            },
        }
    }

    /// Loads the settings for `paths`, making the file on a first run when the folder can be
    /// written to.
    pub fn from_paths(paths: AppPaths) -> Startup {
        let file = paths.settings_file();
        let mut notices = Vec::new();
        let mut can_save = paths.is_writable();
        if !paths.is_writable() {
            let path = paths.root().display().to_string();
            notices.push(Notice {
                title: tr("error-folder-not-writable-title"),
                body: if paths.is_portable() {
                    tr!("error-portable-not-writable", path = path)
                } else {
                    tr!("error-app-folder-not-writable", path = path)
                },
            });
        }
        let existed = file.exists();
        let (settings, warnings) = match Settings::load_or_default(&file) {
            Ok(loaded) => (loaded.settings, loaded.warnings),
            Err(error) => {
                // Defaults for this run, and the file left as it is for the user to fix.
                notices.push(Notice {
                    title: tr("error-settings-unreadable-title"),
                    body: error.to_string(),
                });
                can_save = false;
                (Settings::default(), Vec::new())
            }
        };
        let mut created = false;
        if !existed && can_save {
            match settings.save(&file) {
                Ok(()) => created = true,
                Err(error) => {
                    notices.push(Notice {
                        title: tr("error-folder-not-writable-title"),
                        body: error.to_string(),
                    });
                    can_save = false;
                }
            }
        }
        Startup {
            paths: Some(paths),
            settings,
            warnings,
            notices,
            can_save,
            created,
        }
    }

    pub fn about(&self) -> About {
        About {
            mode: self.paths.as_ref().map(AppPaths::mode),
            settings_file: self.paths.as_ref().map(AppPaths::settings_file),
            logs_dir: self.paths.as_ref().map(AppPaths::logs_dir),
            can_save: self.can_save,
        }
    }

    /// Where captures go: the setting, with its `{pictures}` token resolved to the user's real
    /// Pictures folder (`pictures`, from Windows' known folders), in both modes.
    pub fn captures_folder(&self, pictures: &Path) -> PathBuf {
        self.settings.capture.resolve_save_folder(pictures)
    }

    /// The logs folder, when logs can be written there.
    pub fn logs_dir(&self) -> Option<PathBuf> {
        self.paths
            .as_ref()
            .filter(|p| p.is_writable())
            .map(AppPaths::logs_dir)
    }
}
