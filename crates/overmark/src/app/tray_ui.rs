//! The tray's side of the app: its icon's states, the clicks, the
//! right-click menu, the balloons' clicks, Settings (a process of its own), the settings read
//! again when Settings saves them, Start with Windows and the opt-in update check.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overmark_core::hotkey::Hotkey;
use overmark_core::hotkey_status::Status;
use overmark_core::i18n::tr;
use overmark_core::settings::{Settings, TrayClick};
use overmark_core::tr;
use overmark_core::update::Outcome;
use overmark_win::autostart::{AutoStart, VALUE_NAME};
use overmark_win::overlay::{Backdrop, Command};
use overmark_win::update::CheckError;
use overmark_win::win32::shared_text::SharedText;
use overmark_win::win32::tray::{BalloonKind, IconFile, MenuItem, ThemedIcon, Tray};
use overmark_win::win32::{clock, shell};

use super::App;
use crate::settings_window::{HOTKEY_STATUS, Page};

const MENU_DRAW: u32 = 1;
const MENU_DRAW_FROZEN: u32 = 2;
const MENU_CAPTURE_REGION: u32 = 3;
const MENU_OPEN_CAPTURES: u32 = 4;
const MENU_SETTINGS: u32 = 5;
const MENU_START_WITH_WINDOWS: u32 = 6;
const MENU_CHECK_UPDATES: u32 = 7;
const MENU_ABOUT: u32 = 8;
const MENU_QUIT: u32 = 9;
const MENU_RECORD_REGION: u32 = 10;

/// A [`overmark_win::win32::tray::Waker`] code: the update check has an answer.
const WAKE_UPDATE: u32 = 1;

/// The tray icon in each state (assets/tray), for a light and a dark taskbar.
pub const ICON_IDLE: ThemedIcon = ThemedIcon {
    light_taskbar: IconFile(include_bytes!("../../../../assets/tray/idle-on-light.ico")),
    dark_taskbar: IconFile(include_bytes!("../../../../assets/tray/idle-on-dark.ico")),
};
pub const ICON_LIVE: ThemedIcon = ThemedIcon {
    light_taskbar: IconFile(include_bytes!("../../../../assets/tray/live-on-light.ico")),
    dark_taskbar: IconFile(include_bytes!("../../../../assets/tray/live-on-dark.ico")),
};
pub const ICON_FROZEN: ThemedIcon = ThemedIcon {
    light_taskbar: IconFile(include_bytes!(
        "../../../../assets/tray/frozen-on-light.ico"
    )),
    dark_taskbar: IconFile(include_bytes!("../../../../assets/tray/frozen-on-dark.ico")),
};

/// The icon for the layer's state: idle when it's closed, Frozen's over a frozen screen, and
/// Live's over everything else (the boards are drawn on, as Live is).
pub fn icon_for(backdrop: Option<Backdrop>) -> ThemedIcon {
    match backdrop {
        None => ICON_IDLE,
        Some(Backdrop::Frozen) => ICON_FROZEN,
        Some(_) => ICON_LIVE,
    }
}

/// What clicking the last balloon does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BalloonAction {
    None,
    /// A hotkey notice: open Settings to pick another key.
    Settings,
    /// A new release: open its page.
    Open(String),
}

/// What the update check came back with, and whether it was asked for from the menu.
type UpdateAnswer = (Result<Outcome, CheckError>, bool);

/// The tray's own state, beside the app's.
pub struct TrayUi {
    /// A click waiting to see whether it's the first of a double-click, until then.
    click_until: Option<Instant>,
    /// The click a double-click sends after itself is ignored until then.
    ignore_clicks_until: Option<Instant>,
    pub balloon: BalloonAction,
    update: Arc<Mutex<Option<UpdateAnswer>>>,
    update_running: bool,
    status: Option<SharedText>,
    pub status_generation: u64,
    settings_file: Option<PathBuf>,
    can_save: bool,
    /// The layer was opened for "Capture a region…": close it once the region is chosen.
    pub close_after_region: bool,
    /// The setting keys whose hotkey notice may show at the next registration: `None`, all.
    pub notify_keys: Option<Vec<&'static str>>,
}

impl TrayUi {
    pub fn new(settings_file: Option<PathBuf>, can_save: bool) -> TrayUi {
        let status = match SharedText::create(HOTKEY_STATUS, 4096) {
            Ok(s) => Some(s),
            Err(error) => {
                tracing::warn!(%error, "no hotkey status for Settings");
                None
            }
        };
        TrayUi {
            click_until: None,
            ignore_clicks_until: None,
            balloon: BalloonAction::None,
            update: Arc::default(),
            update_running: false,
            status,
            status_generation: 0,
            settings_file,
            can_save,
            close_after_region: false,
            notify_keys: None,
        }
    }

    /// Publishes what became of each hotkey, for Settings.
    pub fn publish(&mut self, mut status: Status) {
        self.status_generation += 1;
        status.generation = self.status_generation;
        if let Some(shared) = self.status.as_mut() {
            shared.write(&status.encode());
        }
    }

    /// How long the loop may wait before a pending click is due (`None`: no click pending).
    pub fn wait_ms(&self) -> Option<u32> {
        self.click_until.map(|t| {
            t.saturating_duration_since(Instant::now())
                .as_millis()
                .min(u128::from(u32::MAX)) as u32
        })
    }
}

impl App {
    /// Shows the layer's state in the icon.
    pub(super) fn refresh_icon(&mut self) {
        let icon = icon_for(self.overlay.as_ref().map(|o| o.backdrop()));
        if let Err(error) = self.tray.set_icon(icon) {
            tracing::warn!(%error, "couldn't change the tray icon");
        }
    }

    /// A click on the icon. With Settings on a click, it opens at once; otherwise it waits
    /// for the double-click time, so a double-click opens Settings and nothing else.
    pub(super) fn on_click(&mut self) {
        let now = Instant::now();
        if self.ui.ignore_clicks_until.is_some_and(|t| now < t) || self.ui.click_until.is_some() {
            return;
        }
        if self.settings.general.tray_click == TrayClick::Settings {
            self.open_settings(Page::General);
            return;
        }
        let wait = Duration::from_millis(u64::from(overmark_win::win32::tray::double_click_ms()));
        self.ui.click_until = Some(now + wait);
    }

    pub(super) fn on_double_click(&mut self) {
        self.ui.click_until = None;
        let wait = Duration::from_millis(u64::from(overmark_win::win32::tray::double_click_ms()));
        self.ui.ignore_clicks_until = Some(Instant::now() + wait);
        self.open_settings(Page::General);
    }

    /// Does what a click does, now (Enter on the icon, or a click whose double-click time is
    /// up).
    pub(super) fn click_action(&mut self) {
        self.ui.click_until = None;
        match self.settings.general.tray_click {
            TrayClick::Live => self.open(Backdrop::Live, clock::now_100ns()),
            TrayClick::Frozen => self.open(Backdrop::Frozen, clock::now_100ns()),
            TrayClick::Settings => self.open_settings(Page::General),
        }
    }

    /// A pending click whose double-click time is up.
    pub(super) fn click_if_due(&mut self) {
        if self.ui.click_until.is_some_and(|t| Instant::now() >= t) {
            self.click_action();
        }
    }

    pub(super) fn on_balloon_clicked(&mut self) {
        match std::mem::replace(&mut self.ui.balloon, BalloonAction::None) {
            BalloonAction::None => {}
            BalloonAction::Settings => self.open_settings(Page::General),
            BalloonAction::Open(url) => {
                if !shell::open(std::path::Path::new(&url)) {
                    tracing::warn!(%url, "couldn't open the release page");
                }
            }
        }
    }

    /// Shows a balloon, and what clicking it does.
    pub(super) fn balloon(
        &mut self,
        title: &str,
        body: &str,
        kind: BalloonKind,
        action: BalloonAction,
    ) {
        match self.tray.balloon(title, body, kind) {
            Ok(()) => self.ui.balloon = action,
            Err(error) => tracing::warn!(%error, "couldn't show a tray notice"),
        }
    }

    /// Opens the Settings window, a process of its own (or brings the open one forward).
    pub(super) fn open_settings(&mut self, page: Page) {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(error) => {
                tracing::error!(%error, "no exe to start Settings with");
                return;
            }
        };
        let mut command = std::process::Command::new(exe);
        command.arg("--settings");
        if page == Page::About {
            command.arg("--about");
        }
        match command.spawn() {
            Ok(child) => {
                Tray::allow_foreground(child.id());
                tracing::info!(pid = child.id(), ?page, "Settings started");
            }
            Err(error) => {
                tracing::error!(%error, "couldn't start Settings");
                self.balloon(
                    &tr("tray-settings-failed-title"),
                    &tr!("tray-settings-failed", reason = error.to_string()),
                    BalloonKind::Error,
                    BalloonAction::None,
                );
            }
        }
    }

    /// The right-click menu, in its set order, with the hotkeys beside the first
    /// two. Returns false when Quit was chosen.
    pub(super) fn menu(&mut self, x: i32, y: i32) -> bool {
        let label = |key: &str, hotkey: &str| match Hotkey::parse(hotkey) {
            Ok(Some(h)) => format!("{}\t{}", tr(key), h.display()),
            _ => tr(key),
        };
        let starts = autostart_on();
        let items = menu_items(&self.settings, starts, label);
        match self.tray.show_menu(&items, x, y) {
            Ok(Some(MENU_DRAW)) => self.open(Backdrop::Live, clock::now_100ns()),
            Ok(Some(MENU_DRAW_FROZEN)) => self.open(Backdrop::Frozen, clock::now_100ns()),
            Ok(Some(MENU_CAPTURE_REGION)) => self.capture_region(),
            Ok(Some(MENU_RECORD_REGION)) => self.record_region_hotkey(),
            Ok(Some(MENU_OPEN_CAPTURES)) => self.open_captures(),
            Ok(Some(MENU_SETTINGS)) => self.open_settings(Page::General),
            Ok(Some(MENU_START_WITH_WINDOWS)) => self.toggle_autostart(!starts),
            Ok(Some(MENU_CHECK_UPDATES)) => self.check_updates(true),
            Ok(Some(MENU_ABOUT)) => self.open_settings(Page::About),
            Ok(Some(MENU_QUIT)) => return false,
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "couldn't show the tray menu"),
        }
        true
    }

    /// "Capture a region…": the screen frozen as it is, then a region chosen on it; the layer
    /// closes again once it's chosen or cancelled.
    pub(super) fn capture_region(&mut self) {
        let was_open = self.overlay.is_some();
        if !was_open {
            self.open(Backdrop::Frozen, clock::now_100ns());
        }
        if self.overlay.is_some() {
            self.ui.close_after_region = !was_open;
            self.capture(Command::SaveRegion);
            if self.pending_region.is_none() && self.ui.close_after_region {
                // It couldn't start: don't leave the layer up for nothing.
                self.ui.close_after_region = false;
                self.close();
            }
        }
    }

    /// Opens the captures folder in Explorer, making it first if no capture has made it yet.
    fn open_captures(&mut self) {
        let folder = self.captures_folder();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            tracing::warn!(%error, folder = %folder.display(), "couldn't make the captures folder");
        }
        if !shell::open(&folder) {
            tracing::warn!(folder = %folder.display(), "couldn't open the captures folder");
            self.balloon(
                &tr("tray-open-captures-failed-title"),
                &tr!(
                    "tray-open-captures-failed",
                    folder = folder.display().to_string()
                ),
                BalloonKind::Error,
                BalloonAction::None,
            );
        }
    }

    /// Start with Windows from the menu: the Run value (the MSI's), and the setting beside it.
    fn toggle_autostart(&mut self, on: bool) {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| {
                AutoStart::new(VALUE_NAME)
                    .apply(on, &exe)
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(()) => {
                tracing::info!(on, "Start with Windows");
                self.settings.general.start_with_windows = on;
                self.save_setting(|s| s.general.start_with_windows = on);
            }
            Err(error) => {
                tracing::warn!(%error, on, "couldn't change Start with Windows");
                self.balloon(
                    &tr("tray-start-with-windows"),
                    &tr!("settings-autostart-failed", reason = error),
                    BalloonKind::Error,
                    BalloonAction::None,
                );
            }
        }
    }

    /// Changes one setting in the file as it is now, for a change made from the tray.
    pub(super) fn save_setting(&self, change: impl FnOnce(&mut Settings)) {
        let Some(file) = self.ui.settings_file.as_ref().filter(|_| self.ui.can_save) else {
            return;
        };
        let mut on_disk = match Settings::load_or_default(file) {
            Ok(l) => l.settings,
            Err(error) => {
                tracing::warn!(%error, "the settings can't be read: not changed");
                return;
            }
        };
        change(&mut on_disk);
        if let Err(error) = on_disk.save(file) {
            tracing::warn!(%error, "couldn't save the settings");
        }
    }

    /// Reads the settings again (Settings saved them) and takes them up: the hotkeys are
    /// registered afresh if any changed, with a notice only for the ones that changed.
    pub(super) fn reload_settings(&mut self) {
        let Some(file) = self.ui.settings_file.clone() else {
            return;
        };
        let loaded = match Settings::load(&file) {
            Ok(l) => l,
            Err(error) => {
                tracing::warn!(%error, "the saved settings can't be read: kept as they were");
                return;
            }
        };
        for w in &loaded.warnings {
            tracing::warn!(warning = %w, "settings");
        }
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        let (a, b) = (&old, &self.settings);
        let changed: Vec<&'static str> = [
            (
                "general.hotkey_draw",
                a.general.hotkey_draw != b.general.hotkey_draw,
            ),
            (
                "general.hotkey_draw_frozen",
                a.general.hotkey_draw_frozen != b.general.hotkey_draw_frozen,
            ),
            (
                "general.hotkey_capture_region",
                a.general.hotkey_capture_region != b.general.hotkey_capture_region,
            ),
            (
                "general.hotkey_record_region",
                a.general.hotkey_record_region != b.general.hotkey_record_region,
            ),
            (
                "general.print_screen",
                a.general.print_screen != b.general.print_screen,
            ),
            (
                "backdrops.hotkey_whiteboard",
                a.backdrops.hotkey_whiteboard != b.backdrops.hotkey_whiteboard,
            ),
            (
                "backdrops.hotkey_blackboard",
                a.backdrops.hotkey_blackboard != b.backdrops.hotkey_blackboard,
            ),
        ]
        .into_iter()
        .filter_map(|(k, c)| c.then_some(k))
        .collect();
        // First, so turning it on adds this read's diagnostic detail too.
        if self.settings.advanced.diagnostic_logging != old.advanced.diagnostic_logging {
            overmark_win::logging::set_diagnostic(self.settings.advanced.diagnostic_logging);
        }
        tracing::info!(hotkeys_changed = ?changed, "settings read again");
        let sections: Vec<&str> = [
            ("general", a.general != b.general),
            ("drawing", a.drawing != b.drawing),
            ("capture", a.capture != b.capture),
            ("advanced", a.advanced != b.advanced),
            ("backdrops", a.backdrops != b.backdrops),
            ("guides", a.guides != b.guides),
            ("toolbar", a.toolbar != b.toolbar),
        ]
        .into_iter()
        .filter_map(|(k, c)| c.then_some(k))
        .collect();
        tracing::debug!(?sections, "the sections changed");
        if !changed.is_empty() {
            // Let go of the old ones first, so a key moved between actions can be taken.
            tracing::debug!(count = self.registered.len(), "the old hotkeys let go");
            self.hotkeys = None;
            self.registered.clear();
            self.ui.notify_keys = Some(changed);
            self.register_hotkeys();
            self.ui.notify_keys = None;
        }
        self.drawing_changed(&old.drawing);
        if let Some(overlay) = self.overlay.as_mut() {
            let look = overmark_win::overlay::Look {
                backdrops: self.settings.backdrops.clone(),
                guides: self.settings.guides.clone(),
            };
            if let Err(error) = overlay.set_look(look) {
                tracing::warn!(%error, "couldn't show the new backdrop or guides");
            }
            // A blur on the frozen frame was made over the old guides: draw the ink again.
            if self.ink.look_changed(&old, &self.settings) {
                self.flush();
            }
        }
        if self.settings.advanced.check_for_updates && !old.advanced.check_for_updates {
            self.check_updates(false);
        }
        self.refresh_tooltip();
    }

    /// Takes up changed drawing settings now (every change applies at once): the tool,
    /// colour and widths of the drawing state, which stays from one opening to the next, and
    /// the dim of an open layer. The drawing itself stays.
    pub(super) fn drawing_changed(&mut self, old: &overmark_core::settings::DrawingSettings) {
        let new = &self.settings.drawing;
        if *new == *old {
            return;
        }
        self.ink.settings_changed(old, new);
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.set_dim(new.dim);
        }
        tracing::info!(dim = new.dim, "drawing settings taken up");
    }

    /// Asks GitHub for the latest release, off the loop's thread, only when the user has
    /// turned "Check for updates" on (otherwise Overmark makes no network calls).
    /// From the menu (`asked`), every answer is told; at start, only a newer release.
    pub(super) fn check_updates(&mut self, asked: bool) {
        if !self.settings.advanced.check_for_updates {
            tracing::info!("update check is off: nothing asked");
            return;
        }
        if self.ui.update_running {
            return;
        }
        self.ui.update_running = true;
        let slot = Arc::clone(&self.ui.update);
        let waker = self.tray.waker();
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let answer = overmark_win::update::check_latest(env!("CARGO_PKG_VERSION"));
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((answer, asked));
                waker.wake(WAKE_UPDATE);
            });
        if let Err(error) = spawned {
            self.ui.update_running = false;
            tracing::warn!(%error, "couldn't start the update check");
        }
    }

    /// A [`overmark_win::win32::tray::Waker`]'s code came.
    pub(super) fn on_woken(&mut self, code: u32) {
        if code != WAKE_UPDATE {
            return;
        }
        self.ui.update_running = false;
        let answer = self
            .ui
            .update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let Some((answer, asked)) = answer else {
            return;
        };
        match answer {
            Ok(Outcome::Newer { version, url }) => {
                tracing::info!(%version, %url, "a newer release");
                self.balloon(
                    &tr("update-available-title"),
                    &tr!("update-available-body", version = version.to_string()),
                    BalloonKind::Info,
                    BalloonAction::Open(url),
                );
            }
            Ok(Outcome::UpToDate) => {
                tracing::info!("up to date");
                if asked {
                    self.balloon(
                        &tr("update-latest-title"),
                        &tr!("update-latest-body", version = env!("CARGO_PKG_VERSION")),
                        BalloonKind::Info,
                        BalloonAction::None,
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the update check didn't work");
                if asked {
                    self.balloon(
                        &tr("update-failed-title"),
                        &tr!("update-failed-body", reason = error.to_string()),
                        BalloonKind::Warning,
                        BalloonAction::None,
                    );
                }
            }
        }
    }
}

/// Whether the Run value starts this exe.
fn autostart_on() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| AutoStart::new(VALUE_NAME).starts(&exe).ok())
        .unwrap_or(false)
}

/// The menu's items: "Check for updates…" only when it's turned on.
pub fn menu_items(
    settings: &Settings,
    starts_with_windows: bool,
    label: impl Fn(&str, &str) -> String,
) -> Vec<MenuItem> {
    let command = |id, text| MenuItem::Command { id, text };
    let mut items = vec![
        command(MENU_DRAW, label("tray-draw", &settings.general.hotkey_draw)),
        command(
            MENU_DRAW_FROZEN,
            label("tray-draw-frozen", &settings.general.hotkey_draw_frozen),
        ),
        command(
            MENU_CAPTURE_REGION,
            label(
                "tray-capture-region",
                &settings.general.hotkey_capture_region,
            ),
        ),
        command(
            MENU_RECORD_REGION,
            label("tray-record-region", &settings.general.hotkey_record_region),
        ),
        MenuItem::Separator,
        command(MENU_OPEN_CAPTURES, tr("tray-open-captures")),
        command(MENU_SETTINGS, tr("tray-settings")),
        MenuItem::Check {
            id: MENU_START_WITH_WINDOWS,
            text: tr("tray-start-with-windows"),
            checked: starts_with_windows,
        },
        MenuItem::Separator,
    ];
    if settings.advanced.check_for_updates {
        items.push(command(MENU_CHECK_UPDATES, tr("tray-check-updates")));
    }
    items.push(command(MENU_ABOUT, tr("tray-about")));
    items.push(command(MENU_QUIT, tr("tray-quit")));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Command { text, .. } => text.clone(),
                MenuItem::Check { text, checked, .. } => {
                    format!("{text}{}", if *checked { " ✓" } else { "" })
                }
                MenuItem::Separator => "---".to_owned(),
            })
            .collect()
    }

    fn label(key: &str, hotkey: &str) -> String {
        match Hotkey::parse(hotkey) {
            Ok(Some(h)) => format!("{}\t{}", tr(key), h.display()),
            _ => tr(key),
        }
    }

    /// The menu is in its set order, with the hotkeys beside the first three; Check
    /// for updates… is there only when it's turned on.
    #[test]
    fn the_menu_is_in_its_order() {
        let mut settings = Settings::default();
        let off = texts(&menu_items(&settings, true, label));
        assert_eq!(
            off,
            [
                "Draw\tWin+Shift+D",
                "Draw on a frozen screen\tWin+Shift+F",
                "Capture a region…\tWin+Shift+X",
                "Record a region…\tWin+Alt+V",
                "---",
                "Open captures folder",
                "Settings…",
                "Start with Windows ✓",
                "---",
                "About Overmark",
                "Quit",
            ]
        );
        settings.advanced.check_for_updates = true;
        settings.general.hotkey_draw.clear();
        settings.general.hotkey_capture_region.clear();
        let on = texts(&menu_items(&settings, false, label));
        assert_eq!(on[0], "Draw");
        assert_eq!(on[2], "Capture a region…");
        assert_eq!(on[7], "Start with Windows");
        assert_eq!(&on[9..], ["Check for updates…", "About Overmark", "Quit"]);
    }

    #[test]
    fn the_icon_follows_the_layer() {
        assert_eq!(icon_for(None), ICON_IDLE);
        assert_eq!(icon_for(Some(Backdrop::Live)), ICON_LIVE);
        assert_eq!(icon_for(Some(Backdrop::Frozen)), ICON_FROZEN);
        assert_eq!(icon_for(Some(Backdrop::Whiteboard)), ICON_LIVE);
        assert_eq!(icon_for(Some(Backdrop::Blackboard)), ICON_LIVE);
        for icon in [ICON_IDLE, ICON_LIVE, ICON_FROZEN] {
            for light in [true, false] {
                assert!(icon.pick(light).image_for(16).is_some());
            }
        }
    }
}
