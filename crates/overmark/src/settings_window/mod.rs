//! The Settings window: General, Drawing, Capture, and About and advanced. Every
//! change applies at once: it's saved to `settings.toml` and the tray is asked to read it
//! again, which takes it up (hotkeys registered afresh, the next drawing's dim, and so on).
//!
//! **A process of its own** (`overmark.exe --settings`), started by the tray and gone once
//! closed, as the grabber is: egui with its GL context and the driver behind it keeps tens of
//! megabytes, which the resident app's 2 MB idle (docs/ARCHITECTURE.md, The budgets) can't
//! carry once Settings has been opened. The two talk through named objects in the session:
//! - the window asks the tray to read the settings again ([`tray::signal_reload`]);
//! - the tray publishes what became of each hotkey ([`HOTKEY_STATUS`], read here to say a
//!   hotkey isn't working);
//! - one Settings window at a time: a second launch brings the first forward
//!   ([`SHOW_EVENT`], [`ABOUT_EVENT`]).
//!
//! What the window does to the world goes through [`Host`]: the real one here, a fake in the
//! tests, which drive the window through its accessibility tree.

mod pages;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use overmark_core::hotkey::Hotkey;
use overmark_core::hotkey_status::Status;
use overmark_core::i18n::tr;
use overmark_core::settings::{Settings, Warning};
use overmark_core::tr;
use overmark_win::autostart::{AutoStart, VALUE_NAME};
use overmark_win::paths::{AppPaths, Mode};
use overmark_win::win32::tray::{self, InstanceLock, NamedEvent};

use crate::APP_ID;
use crate::config::{About, Startup};
use crate::ui;

/// The shared memory where the tray publishes its hotkeys' state (core's `hotkey_status`).
pub const HOTKEY_STATUS: &str = r"Local\Overmark.HotkeyStatus";
/// One Settings window per session.
const LOCK: &str = r"Local\Overmark.Settings";
/// A second launch asks the open window to come forward, on its page or on About.
pub const SHOW_EVENT: &str = r"Local\Overmark.SettingsShow";
pub const ABOUT_EVENT: &str = r"Local\Overmark.SettingsAbout";

/// The window's pages, in its tab strip's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Drawing,
    Capture,
    About,
}

impl Page {
    pub const ALL: [Page; 4] = [Page::General, Page::Drawing, Page::Capture, Page::About];

    fn index(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    fn title(self) -> String {
        tr(match self {
            Page::General => "settings-general",
            Page::Drawing => "settings-drawing",
            Page::Capture => "settings-capture",
            Page::About => "settings-about",
        })
    }
}

/// What the window does to the world: Windows' in the app, a recording fake in the tests.
pub trait Host {
    /// Whether Overmark starts with Windows (the Run value, which is what counts).
    fn autostart(&self) -> bool;
    fn set_autostart(&self, on: bool) -> Result<(), String>;
    /// Opens a folder in Explorer, or a link in the browser.
    fn open(&self, target: &str) -> bool;
    /// Asks for a folder, starting in `start`.
    fn pick_folder(&self, start: &Path) -> Option<PathBuf>;
    /// Asks the tray to read the settings again.
    fn reload(&self);
    /// What the tray published about its hotkeys, if it's running.
    fn hotkey_status(&self) -> Option<Status>;
    /// Whether `hotkey` is free (no app has it, Overmark included).
    fn is_free(&self, hotkey: &Hotkey) -> bool;
    /// The user's Pictures folder, for `{pictures}`.
    fn pictures(&self) -> PathBuf;
}

/// The real world.
pub struct WinHost {
    pub pictures: PathBuf,
}

impl Host for WinHost {
    fn autostart(&self) -> bool {
        match std::env::current_exe() {
            Ok(exe) => AutoStart::new(VALUE_NAME).starts(&exe).unwrap_or(false),
            Err(_) => false,
        }
    }

    fn set_autostart(&self, on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        AutoStart::new(VALUE_NAME)
            .apply(on, &exe)
            .map_err(|e| e.to_string())
    }

    fn open(&self, target: &str) -> bool {
        overmark_win::win32::shell::open(Path::new(target))
    }

    fn pick_folder(&self, start: &Path) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr("settings-save-location"))
            .set_directory(start)
            .pick_folder()
    }

    fn reload(&self) {
        if !tray::signal_reload(APP_ID) {
            tracing::info!("the tray isn't running: nothing to tell");
        }
    }

    fn hotkey_status(&self) -> Option<Status> {
        overmark_win::win32::shared_text::read(HOTKEY_STATUS).and_then(|t| Status::decode(&t))
    }

    fn is_free(&self, hotkey: &Hotkey) -> bool {
        overmark_win::hotkeys::is_free(hotkey)
    }

    fn pictures(&self) -> PathBuf {
        self.pictures.clone()
    }
}

/// How often the window looks for changes made elsewhere (the tray's Start with Windows, a
/// hand edit) and for the tray's hotkey answers.
const LOOK_EVERY: Duration = Duration::from_millis(750);

/// The window's state.
pub struct SettingsApp {
    pub(crate) settings: Settings,
    /// What the file has, as last saved or read.
    saved: Settings,
    file: Option<PathBuf>,
    can_save: bool,
    about: About,
    warnings: Vec<Warning>,
    pub(crate) host: Box<dyn Host>,
    page: Page,
    /// A page another launch asked for (index + 1; 0 for none).
    asked: Arc<AtomicU8>,
    /// The hotkey being changed, if any.
    pub(crate) edit: Option<pages::HotkeyEdit>,
    /// A file name pattern being typed that can't be used yet; `None` while the field shows
    /// the setting. Dropped whenever the settings are replaced.
    pub(crate) pattern_draft: Option<String>,
    /// The last save's error.
    save_error: Option<String>,
    autostart_error: Option<String>,
    /// The tray's hotkey answers, and the generation the last save left (to wait for news).
    status: Option<Status>,
    /// The file's modified time when last read or written.
    file_time: Option<SystemTime>,
    last_look: Instant,
    confirm_reset: bool,
    /// The third-party notices, split into lines once, for showing in rows.
    notices: Option<Vec<&'static str>>,
}

/// The notices, generated by `scripts/third-party-notices.ps1` and shipped beside the exe;
/// embedded so About shows them wherever the exe is.
pub const NOTICES: &str = include_str!("../../../../THIRD-PARTY-NOTICES.txt");

impl SettingsApp {
    /// The window over `startup`'s settings and paths.
    pub fn new(startup: &Startup, host: Box<dyn Host>, page: Page) -> SettingsApp {
        let file = startup.paths.as_ref().map(AppPaths::settings_file);
        let file_time = file.as_deref().and_then(modified);
        SettingsApp {
            settings: startup.settings.clone(),
            saved: startup.settings.clone(),
            file,
            can_save: startup.can_save,
            about: startup.about(),
            warnings: startup.warnings.clone(),
            host,
            page,
            asked: Arc::default(),
            edit: None,
            pattern_draft: None,
            save_error: None,
            autostart_error: None,
            status: None,
            file_time,
            last_look: Instant::now() - LOOK_EVERY,
            confirm_reset: false,
            notices: None,
        }
    }

    /// The page showing.
    pub fn page(&self) -> Page {
        self.page
    }

    /// Whether changes can be saved this run.
    pub fn can_save(&self) -> bool {
        self.can_save
    }

    /// Saves what changed, keeping what the window doesn't edit (the toolbar's places, which
    /// the tray writes) as the file has it now, and asks the tray to take it up.
    fn save_if_changed(&mut self) {
        if self.settings == self.saved {
            return;
        }
        let Some(file) = self.file.clone() else {
            self.saved = self.settings.clone();
            return;
        };
        if !self.can_save {
            // Kept in this window only, as the tray does.
            self.saved = self.settings.clone();
            return;
        }
        if let Ok(on_disk) = Settings::load(&file) {
            self.settings.toolbar = on_disk.settings.toolbar;
        }
        match self.settings.save(&file) {
            Ok(()) => {
                tracing::info!(file = %file.display(), "settings saved");
                self.saved = self.settings.clone();
                self.save_error = None;
                self.file_time = modified(&file);
                self.host.reload();
            }
            Err(error) => {
                tracing::warn!(%error, "settings not saved");
                self.save_error = Some(tr!(
                    "error-settings-not-saved",
                    path = file.display().to_string(),
                    detail = error.to_string()
                ));
            }
        }
    }

    /// Takes up a change to the file made elsewhere (the tray's Start with Windows, a hand
    /// edit), and the tray's hotkey answers. Called between frames.
    fn look(&mut self) {
        if self.last_look.elapsed() < LOOK_EVERY {
            return;
        }
        self.last_look = Instant::now();
        self.status = self.host.hotkey_status();
        let Some(file) = self.file.clone() else {
            return;
        };
        let now = modified(&file);
        if now != self.file_time && self.settings == self.saved {
            self.file_time = now;
            match Settings::load(&file) {
                Ok(loaded) => {
                    tracing::info!("settings changed on disk: read again");
                    self.settings = loaded.settings.clone();
                    self.saved = loaded.settings;
                    self.warnings = loaded.warnings;
                    self.edit = None;
                    self.pattern_draft = None;
                }
                Err(error) => tracing::warn!(%error, "the changed settings can't be read"),
            }
        }
    }

    /// Puts every setting back to its default (and Start with Windows off).
    pub(crate) fn reset(&mut self) {
        self.settings = Settings::default();
        self.edit = None;
        self.pattern_draft = None;
        if self.host.autostart()
            && let Err(e) = self.host.set_autostart(false)
        {
            self.autostart_error = Some(tr!("settings-autostart-failed", reason = e));
        }
    }

    /// Turns Start with Windows on or off, and keeps the setting beside it in step.
    pub(crate) fn set_autostart(&mut self, on: bool) {
        match self.host.set_autostart(on) {
            Ok(()) => {
                self.autostart_error = None;
                self.settings.general.start_with_windows = on;
            }
            Err(e) => self.autostart_error = Some(tr!("settings-autostart-failed", reason = e)),
        }
    }

    /// One frame: the tab strip and the page, then saving what changed.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let asked = self.asked.swap(0, Ordering::SeqCst);
        if asked > 0
            && let Some(p) = Page::ALL.get(usize::from(asked) - 1)
        {
            self.page = *p;
        }
        self.look();
        pages::show(self, ui);
        // Not while something is held (a slider dragged): once, when it's let go.
        if !ui.ctx().input(|i| i.pointer.any_down()) {
            self.save_if_changed();
        }
        ui.ctx().request_repaint_after(LOOK_EVERY);
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
        ui::theme::paint_focus(ui.ctx());
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The app's icon for the window's title bar and taskbar button.
fn window_icon() -> Option<egui::IconData> {
    let png = include_bytes!("../../../../assets/icon/overmark-64.png");
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    Some(egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

/// Runs the Settings window as this process (`--settings`, `--about` for About), until it's
/// closed. Another one open: it's brought forward instead.
pub fn run(page: Page) -> i32 {
    let _lock = match InstanceLock::acquire(LOCK) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            let event = if page == Page::About {
                ABOUT_EVENT
            } else {
                SHOW_EVENT
            };
            let _ = NamedEvent::signal(event);
            return 0;
        }
        Err(error) => {
            tracing::error!(%error, "couldn't check for an open Settings window");
            return 1;
        }
    };
    let startup = Startup::load(AppPaths::detect(APP_ID));
    tracing::info!(
        mode = startup.about().mode.map_or("unknown", Mode::as_str),
        can_save = startup.can_save,
        "Settings opening"
    );
    let pictures = overmark_win::win32::known_folders::pictures().unwrap_or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Pictures")
    });
    let asked: Arc<AtomicU8> = Arc::default();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(tr("settings-title"))
        .with_inner_size([780.0, 640.0])
        .with_min_inner_size([560.0, 420.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        persist_window: false,
        ..Default::default()
    };
    let no_font: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let no_font_seen = Arc::clone(&no_font);
    let result = eframe::run_native(
        "Overmark Settings",
        options,
        Box::new(move |cc| {
            if let Err(e) = ui::install(&cc.egui_ctx) {
                let text = tr!(
                    "settings-no-font",
                    dir = e.dir.display().to_string(),
                    files = e.missing.join(", ")
                );
                *no_font_seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.clone());
                return Err(text.into());
            }
            // Another launch's request: this page, to the front.
            for (name, page) in [(SHOW_EVENT, None), (ABOUT_EVENT, Some(Page::About))] {
                let ctx = cc.egui_ctx.clone();
                let asked_here = Arc::clone(&asked);
                match NamedEvent::create(name) {
                    Ok(event) => {
                        std::thread::spawn(move || {
                            while event.wait(None) {
                                if let Some(p) = page {
                                    asked_here.store(p.index() as u8 + 1, Ordering::SeqCst);
                                }
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                                ctx.request_repaint();
                            }
                        });
                    }
                    Err(error) => tracing::warn!(%error, name, "no event to be shown by"),
                }
            }
            let mut app = SettingsApp::new(&startup, Box::new(WinHost { pictures }), page);
            app.asked = asked;
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => {
            tracing::info!("Settings closed");
            0
        }
        Err(error) => {
            let text = no_font
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .unwrap_or_else(|| error.to_string());
            tracing::error!(error = %text, "Settings couldn't open");
            1
        }
    }
}
