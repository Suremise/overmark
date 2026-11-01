//! The Settings window driven through its accessibility tree (what Narrator reads and what a
//! UI Automation client would press), in a temp folder of each test's own, with a fake
//! [`Host`]: every change lands in `settings.toml` at once and the tray is asked to read it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use overmark_core::hotkey::Hotkey;
use overmark_core::hotkey_status::{State, Status};
use overmark_core::i18n::tr;
use overmark_core::settings::{
    AfterCapture, ImageFormat, PrintScreen, Settings, ToolbarPosition, TrayClick,
};
use overmark_core::tool::ToolKind;
use overmark_core::tr;
use overmark_win::paths::AppPaths;

use super::pages::folder_setting;
use super::{Host, Page, SettingsApp};
use crate::config::Startup;

/// What the window did to the (fake) world.
#[derive(Default)]
struct Record {
    autostart: bool,
    opened: Vec<String>,
    reloads: u32,
    /// Hotkeys other apps have.
    taken: Vec<Hotkey>,
    status: Option<Status>,
    pick: Option<PathBuf>,
    pictures: PathBuf,
}

#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Record>>);

impl Host for Fake {
    fn autostart(&self) -> bool {
        self.0.borrow().autostart
    }
    fn set_autostart(&self, on: bool) -> Result<(), String> {
        self.0.borrow_mut().autostart = on;
        Ok(())
    }
    fn open(&self, target: &str) -> bool {
        self.0.borrow_mut().opened.push(target.to_owned());
        true
    }
    fn pick_folder(&self, _: &Path) -> Option<PathBuf> {
        self.0.borrow().pick.clone()
    }
    fn reload(&self) {
        self.0.borrow_mut().reloads += 1;
    }
    fn hotkey_status(&self) -> Option<Status> {
        self.0.borrow().status.clone()
    }
    fn is_free(&self, hotkey: &Hotkey) -> bool {
        !self.0.borrow().taken.contains(hotkey)
    }
    fn pictures(&self) -> PathBuf {
        self.0.borrow().pictures.clone()
    }
}

/// An installed copy's settings in a temp folder, and the window over them.
struct Setup {
    _temp: tempfile::TempDir,
    file: PathBuf,
    fake: Fake,
}

impl Setup {
    fn new() -> Setup {
        let temp = tempfile::tempdir().unwrap();
        let exe_dir = temp.path().join("app");
        let appdata = temp.path().join("appdata");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&appdata).unwrap();
        let paths = AppPaths::resolve("Overmark", &exe_dir, Some(&appdata)).unwrap();
        let file = paths.settings_file();
        let fake = Fake::default();
        fake.0.borrow_mut().pictures = temp.path().join("Pictures");
        Setup {
            _temp: temp,
            file,
            fake,
        }
    }

    fn startup(&self) -> Startup {
        // <temp>ppdata\Overmark\settings.toml, the exe in <temp>pp.
        let appdata = self.file.parent().unwrap().parent().unwrap();
        let exe = appdata.parent().unwrap().join("app");
        Startup::load(AppPaths::resolve("Overmark", &exe, Some(appdata)))
    }

    fn window(&self, page: Page) -> Harness<'static, SettingsApp> {
        let app = SettingsApp::new(&self.startup(), Box::new(self.fake.clone()), page);
        let mut harness = Harness::builder()
            .with_size([780.0, 2600.0])
            .with_max_steps(64)
            .with_context_setup(|ctx| crate::ui::install(ctx).unwrap())
            .build_ui_state(|ui, app: &mut SettingsApp| app.show(ui), app);
        harness.run();
        harness
    }

    /// What the file has now.
    fn saved(&self) -> Settings {
        Settings::load(&self.file).unwrap().settings
    }
}

/// Picks `choice` from the dropdown named `name`: the open list's button of that name (a row
/// may have the same name: it isn't a button).
fn choose(harness: &mut Harness<'_, SettingsApp>, name: &str, choice: &str) {
    harness.get_by_label(name).click();
    harness.run();
    // As a screen reader picks it: a long list scrolls, and the choice may be out of view.
    harness
        .get_by_role_and_label(Role::Button, choice)
        .click_accesskit();
    harness.run();
}

#[test]
fn general_changes_are_saved_at_once_and_the_tray_told() {
    let s = Setup::new();
    let mut h = s.window(Page::General);
    assert_eq!(s.fake.0.borrow().reloads, 0, "nothing changed yet");

    h.get_by_label(&tr("settings-start-with-windows")).click();
    h.run();
    assert!(s.fake.0.borrow().autostart, "the Run value");
    assert!(
        s.saved().general.start_with_windows,
        "and the setting beside it"
    );
    assert_eq!(s.fake.0.borrow().reloads, 1);

    choose(
        &mut h,
        &tr("settings-tray-click"),
        &tr("tray-click-settings"),
    );
    assert_eq!(s.saved().general.tray_click, TrayClick::Settings);

    choose(
        &mut h,
        &tr("settings-print-screen"),
        &tr("tray-draw-frozen"),
    );
    assert_eq!(s.saved().general.print_screen, PrintScreen::Frozen);
    assert_eq!(s.fake.0.borrow().reloads, 3);
}

/// A hotkey is changed with the modifier toggles and the key list; one another app has is
/// refused with the nearest free one offered; another Overmark hotkey's key is refused too.
#[test]
fn hotkeys_are_changed_and_conflicts_are_said() {
    let s = Setup::new();
    let mut h = s.window(Page::General);
    let draw = tr("settings-hotkey-draw");
    let change = tr!("settings-hotkey-change-named", action = draw.as_str());

    // Win+Shift+K: free.
    h.get_by_label(&change).click();
    h.run();
    choose(&mut h, &tr("settings-hotkey-key"), "K");
    h.get_by_label(&tr("settings-hotkey-use")).click();
    h.run();
    assert_eq!(s.saved().general.hotkey_draw, "Win+Shift+K");

    // Win+Shift+J: another app has it. Win+Alt+J is offered, and taken.
    let j = Hotkey::parse("Win+Shift+J").unwrap().unwrap();
    s.fake.0.borrow_mut().taken.push(j);
    h.get_by_label(&change).click();
    h.run();
    choose(&mut h, &tr("settings-hotkey-key"), "J");
    h.get_by_label(&tr("settings-hotkey-use")).click();
    h.run();
    h.get_by_label_contains(&tr!("hotkey-taken", hotkey = j.display()));
    assert_eq!(s.saved().general.hotkey_draw, "Win+Shift+K", "not taken");
    h.get_by_label(&tr!("settings-hotkey-use-instead", hotkey = "Win+Alt+J"))
        .click();
    h.run();
    assert_eq!(s.saved().general.hotkey_draw, "Win+Alt+J");

    // Draw on a frozen screen can't have Draw's key.
    let frozen = tr("settings-hotkey-draw-frozen");
    h.get_by_label(&tr!(
        "settings-hotkey-change-named",
        action = frozen.as_str()
    ))
    .click();
    h.run();
    h.get_by_label(&tr("key-shift")).click();
    h.get_by_label(&tr("key-alt")).click();
    h.run();
    choose(&mut h, &tr("settings-hotkey-key"), "J");
    h.get_by_label(&tr("settings-hotkey-use")).click();
    h.run();
    h.get_by_label_contains(&tr!(
        "hotkey-duplicate",
        hotkey = "Win+Alt+J",
        action = draw.as_str()
    ));
    assert_eq!(s.saved().general.hotkey_draw_frozen, "Win+Shift+F");
    h.get_by_label(&tr("settings-cancel")).click();
    h.run();

    // A whiteboard hotkey can be set and removed (none by default).
    let board = tr("settings-hotkey-whiteboard");
    h.get_by_label(&tr!(
        "settings-hotkey-change-named",
        action = board.as_str()
    ))
    .click();
    h.run();
    h.get_by_label(&tr("settings-hotkey-use")).click();
    h.run();
    assert_eq!(s.saved().backdrops.hotkey_whiteboard, "Win+Shift+W");
    h.get_by_label(&tr!("settings-hotkey-clear-named", action = board.as_str()))
        .click();
    h.run();
    assert_eq!(s.saved().backdrops.hotkey_whiteboard, "");
}

/// What the tray found when it registered the hotkeys shows beside each.
#[test]
fn the_trays_hotkey_problems_show() {
    let s = Setup::new();
    let mut status = Status::default();
    status.set("general.hotkey_draw", State::Registered);
    status.set("general.hotkey_draw_frozen", State::Taken);
    s.fake.0.borrow_mut().status = Some(status);
    let h = s.window(Page::General);
    h.get_by_label_contains(&tr!("hotkey-taken", hotkey = "Win+Shift+F"));
    assert!(
        h.query_by_label_contains(&tr!("hotkey-taken", hotkey = "Win+Shift+D"))
            .is_none()
    );
}

#[test]
fn drawing_changes_are_saved() {
    let s = Setup::new();
    let mut h = s.window(Page::Drawing);
    choose(
        &mut h,
        &tr("settings-default-tool"),
        &ToolKind::Highlighter.name(),
    );
    h.get_by_label(&tr("settings-dim")).set_value_accesskit(0.0);
    h.run();
    h.get_by_label(&tr("settings-disappearing-ink")).click();
    h.run();
    h.get_by_label(&tr("settings-disappearing-ink-delay"))
        .set_value_accesskit(5.0);
    h.get_by_label(&tr("settings-laser-trail"))
        .set_value_accesskit(2000.0);
    h.get_by_label(&tr("settings-width-pen"))
        .set_value_accesskit(12.0);
    h.get_by_label(&tr("settings-width-shapes"))
        .set_value_accesskit(9.0);
    h.get_by_label(&tr("settings-guide-spacing"))
        .set_value_accesskit(48.0);
    h.get_by_label(&tr("settings-guides-on-shares")).click();
    h.run();
    h.get_by_label(&tr("settings-right-click-undoes")).click();
    h.run();
    choose(
        &mut h,
        &tr("settings-default-colour"),
        &tr!("settings-colour-n", n = 5),
    );
    let d = s.saved().drawing;
    assert_eq!(d.default_tool, ToolKind::Highlighter);
    assert_eq!(d.dim, 0.0, "0 is off");
    assert!(d.disappearing_ink);
    assert_eq!(d.disappearing_ink_seconds, 5.0);
    assert_eq!(d.laser_trail_ms, 2000);
    assert!(!d.right_click_undoes, "on by default, turned off");
    assert_eq!(d.widths.pen, 12.0);
    let w = d.widths;
    assert_eq!(
        (w.arrow, w.rectangle, w.ellipse, w.line),
        (9.0, 9.0, 9.0, 9.0)
    );
    assert_eq!(d.default_colour, 4);
    let g = s.saved().guides;
    assert_eq!(g.spacing, 48);
    assert!(!g.show_on_shares && g.include_in_saves);
}

#[test]
fn capture_changes_are_saved() {
    let s = Setup::new();
    let pictures = s.fake.0.borrow().pictures.clone();
    s.fake.0.borrow_mut().pick = Some(pictures.join("Shots"));
    let mut h = s.window(Page::Capture);
    h.get_by_label(&tr("settings-save-location-browse")).click();
    h.run();
    assert_eq!(s.saved().capture.save_folder, r"{pictures}\Shots");
    h.get_by_label(&tr("settings-after-capture-copy")).click();
    h.run();
    h.get_by_label(&tr("settings-format-jpg")).click();
    h.run();
    h.get_by_label(&tr("settings-jpg-quality"))
        .set_value_accesskit(70.0);
    h.get_by_label(&tr("settings-include-drawing")).click();
    h.get_by_label(&tr("settings-toast")).click();
    h.run();
    let name = tr("settings-file-name");
    h.get_by_label(&name).focus();
    h.get_by_label(&name)
        .set_text_value_accesskit("shot {hour}{minute}");
    h.run();
    let c = s.saved().capture;
    assert_eq!(c.after_capture, AfterCapture::Copy);
    assert_eq!(c.format, ImageFormat::Jpg);
    assert_eq!(c.jpg_quality, 70);
    assert!(!c.include_drawing && !c.toast);
    assert_eq!(c.file_name_pattern, "shot {hour}{minute}");

    // A pattern that can't be used isn't saved; the last one that could stays.
    h.get_by_label(&name)
        .set_text_value_accesskit("shot {hours}");
    h.run();
    assert_eq!(s.saved().capture.file_name_pattern, "shot {hour}{minute}");

    h.get_by_label(&tr("settings-save-location-reset")).click();
    h.run();
    assert_eq!(
        s.saved().capture.save_folder,
        overmark_core::settings::CaptureSettings::default().save_folder
    );
}

#[test]
fn a_folder_in_pictures_follows_pictures() {
    let pictures = Path::new(r"C:\Users\A\Pictures");
    assert_eq!(folder_setting(pictures, pictures), "{pictures}");
    assert_eq!(
        folder_setting(&pictures.join(r"Overmark\2026"), pictures),
        r"{pictures}\Overmark\2026"
    );
    assert_eq!(
        folder_setting(Path::new(r"D:\Shots"), pictures),
        r"D:\Shots"
    );
}

/// About: the version, mode, author, the repository's link (opened), notices (embedded, the C runtime named), the folders opened, the
/// update check turned on, and Reset after asking.
#[test]
fn about_shows_and_resets() {
    let s = Setup::new();
    let mut h = s.window(Page::About);
    h.get_by_label(&tr!(
        "settings-version-value",
        version = env!("CARGO_PKG_VERSION")
    ));
    h.get_by_label(&tr("settings-mode-installed"));
    h.get_by_label_contains("Darren Marshall");
    h.get_by_label_contains("THIRD-PARTY NOTICES FOR OVERMARK");
    assert!(super::NOTICES.contains("PART 3: MICROSOFT C RUNTIME"));
    h.get_by_label(&crate::ui::widgets::flow(&tr("settings-third-party-crt")));

    h.get_by_label(&tr("settings-open-log-folder")).click();
    h.run();
    let logs = s.file.parent().unwrap().join("logs");
    assert_eq!(s.fake.0.borrow().opened, [logs.display().to_string()]);

    // The repository's link shows its address and opens it.
    let repo = "https://github.com/Suremise/overmark";
    h.get_by_role_and_label(Role::Link, repo).click();
    h.run();
    assert_eq!(s.fake.0.borrow().opened.last().unwrap(), repo);

    h.get_by_label(&tr("settings-check-updates")).click();
    h.run();
    assert!(s.saved().advanced.check_for_updates);

    s.fake.0.borrow_mut().autostart = true;
    h.get_by_role_and_label(Role::Button, &tr("settings-reset"))
        .click();
    h.run();
    h.get_by_label(&tr("settings-reset-yes")).click();
    h.run();
    assert_eq!(s.saved(), Settings::default());
    assert!(!s.fake.0.borrow().autostart, "Start with Windows off too");
}

/// The tray's own part of the file (where the toolbar was left) is kept as the file has it,
/// whatever the window read at start.
#[test]
fn what_the_tray_wrote_meanwhile_is_kept() {
    let s = Setup::new();
    let mut h = s.window(Page::General);
    let mut on_disk = s.saved();
    on_disk
        .toolbar
        .positions
        .insert("\\\\.\\DISPLAY1".into(), ToolbarPosition { x: 40, y: 50 });
    on_disk.save(&s.file).unwrap();
    choose(&mut h, &tr("settings-tray-click"), &tr("tray-click-frozen"));
    let saved = s.saved();
    assert_eq!(saved.general.tray_click, TrayClick::Frozen);
    assert_eq!(saved.toolbar, on_disk.toolbar);
}

/// A change made elsewhere (the tray's Start with Windows) shows without reopening.
#[test]
fn a_change_on_disk_is_read_again() {
    let s = Setup::new();
    let mut h = s.window(Page::Drawing);
    let mut on_disk = s.saved();
    on_disk.drawing.dim = 0.5;
    on_disk.save(&s.file).unwrap();
    // A modified time the window hasn't seen, whatever the clock's resolution.
    let f = std::fs::File::options().write(true).open(&s.file).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    drop(f);
    h.state_mut().last_look -= super::LOOK_EVERY;
    h.run();
    assert_eq!(h.state().settings.drawing.dim, 0.5);
}

/// A folder that can't be written to: said at the top, and nothing written.
#[test]
fn unsaveable_settings_are_said_and_not_written() {
    let s = Setup::new();
    let mut startup = s.startup();
    startup.can_save = false;
    let before = std::fs::read(&s.file).unwrap();
    let app = SettingsApp::new(&startup, Box::new(s.fake.clone()), Page::General);
    let mut h = Harness::builder()
        .with_size([780.0, 2600.0])
        .with_context_setup(|ctx| crate::ui::install(ctx).unwrap())
        .build_ui_state(|ui, app: &mut SettingsApp| app.show(ui), app);
    h.run();
    h.get_by_label_contains(&tr("error-folder-not-writable-title"));
    choose(&mut h, &tr("settings-tray-click"), &tr("tray-click-frozen"));
    assert_eq!(std::fs::read(&s.file).unwrap(), before);
    assert_eq!(s.fake.0.borrow().reloads, 0);
}

/// Writes a file name pattern into the file, as a hand edit or an earlier run would.
fn with_pattern(s: &Setup, pattern: &str) {
    let mut on_disk = Settings::load_or_default(&s.file).unwrap().settings;
    on_disk.capture.file_name_pattern = pattern.to_owned();
    on_disk.save(&s.file).unwrap();
}

/// Capture visited (its field shown),
/// then Reset on About, then Capture again: the default pattern stays, the old one isn't
/// written back.
#[test]
fn reset_from_another_page_is_kept_when_capture_shows_again() {
    let s = Setup::new();
    with_pattern(&s, "CUSTOM {hour}{minute}");
    let mut h = s.window(Page::Capture);
    assert_eq!(
        h.get_by_label(&tr("settings-file-name")).value().as_deref(),
        Some("CUSTOM {hour}{minute}")
    );
    h.get_by_label(&tr("settings-about")).click();
    h.run();
    h.get_by_role_and_label(Role::Button, &tr("settings-reset"))
        .click();
    h.run();
    h.get_by_label(&tr("settings-reset-yes")).click();
    h.run();
    let default = Settings::default().capture.file_name_pattern;
    assert_eq!(s.saved().capture.file_name_pattern, default);
    h.get_by_label(&tr("settings-capture")).click();
    h.run();
    assert_eq!(s.saved().capture.file_name_pattern, default);
    assert_eq!(
        h.get_by_label(&tr("settings-file-name")).value().as_deref(),
        Some(default.as_str())
    );
}

/// A pattern being typed that can't be used yet is a draft: it isn't saved, and a change to
/// the file meanwhile replaces it rather than being overwritten by it.
#[test]
fn a_change_on_disk_replaces_a_draft_pattern() {
    let s = Setup::new();
    let mut h = s.window(Page::Capture);
    let name = tr("settings-file-name");
    h.get_by_label(&name).focus();
    h.get_by_label(&name).set_text_value_accesskit("half {hou");
    h.run();
    assert_eq!(
        h.get_by_label(&name).value().as_deref(),
        Some("half {hou"),
        "the draft stays in the field"
    );
    let default = Settings::default().capture.file_name_pattern;
    assert_eq!(s.saved().capture.file_name_pattern, default, "not saved");

    with_pattern(&s, "FROM DISK {day}");
    let f = std::fs::File::options().write(true).open(&s.file).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    drop(f);
    h.state_mut().last_look -= super::LOOK_EVERY;
    h.run();
    h.get_by_label(&tr("settings-drawing")).click();
    h.run();
    h.get_by_label(&tr("settings-capture")).click();
    h.run();
    assert_eq!(s.saved().capture.file_name_pattern, "FROM DISK {day}");
    assert_eq!(
        h.get_by_label(&name).value().as_deref(),
        Some("FROM DISK {day}")
    );
}

/// The box beside the slider named `setting`, to type its value in.
fn value_box(h: &Harness<'_, SettingsApp>, setting: &str) -> eframe::egui::Rect {
    h.get_by_label(&tr!("settings-value-box", setting = tr(setting)))
        .rect()
}

/// A card's value boxes are one width, whatever values they show, so its sliders line up; and
/// no box changes width as its number grows.
#[test]
fn a_cards_value_boxes_are_one_width_whatever_their_values() {
    let s = Setup::new();
    let mut h = s.window(Page::Drawing);
    let widths = [
        "settings-width-pen",
        "settings-width-highlighter",
        "settings-width-shapes",
        "settings-width-text",
        "settings-width-stamp",
        "settings-width-blur",
    ];
    let drawing = [
        "settings-dim",
        "settings-disappearing-ink-delay",
        "settings-laser-trail",
    ];
    let measure = |h: &Harness<'_, SettingsApp>, group: &[&str]| -> Vec<(f32, f32, f32)> {
        group
            .iter()
            .map(|s| {
                let b = value_box(h, s);
                let slider = h.get_by_label(&tr(s)).rect();
                (b.width(), b.left(), slider.left())
            })
            .collect()
    };
    let first = [measure(&h, &widths), measure(&h, &drawing)];
    for group in &first {
        let (w, x, slider) = group[0];
        assert!(w >= 40.0, "{group:?}");
        for &(w2, x2, slider2) in group {
            assert_eq!((w2, x2, slider2), (w, x, slider), "{group:?}");
        }
    }
    // One digit, three, and the widest texts each card shows.
    h.get_by_label(&tr("settings-width-pen"))
        .set_value_accesskit(1.0);
    h.get_by_label(&tr("settings-width-highlighter"))
        .set_value_accesskit(200.0);
    h.get_by_label(&tr("settings-dim")).set_value_accesskit(0.0);
    h.get_by_label(&tr("settings-laser-trail"))
        .set_value_accesskit(5000.0);
    h.run();
    let d = s.saved().drawing;
    assert_eq!((d.widths.pen, d.widths.highlighter), (1.0, 200.0));
    assert_eq!((d.dim, d.laser_trail_ms), (0.0, 5000));
    assert_eq!([measure(&h, &widths), measure(&h, &drawing)], first);
}

/// Typing in a value box keeps its width (the text scrolls inside it), reads a number with its
/// unit or without, and keeps what's typed in the slider's range.
#[test]
fn typing_in_a_value_box_keeps_its_width_and_the_range() {
    use eframe::egui::Key;
    let s = Setup::new();
    let mut h = s.window(Page::Drawing);
    let label = tr!("settings-value-box", setting = tr("settings-width-pen"));
    let before = value_box(&h, "settings-width-pen");
    for (typed, kept) in [("12 px", 12.0), ("123456789", 200.0), ("7", 7.0)] {
        h.get_by_label(&label).focus();
        h.run();
        h.get_by_label(&label).type_text(typed);
        h.run();
        let editing = value_box(&h, "settings-width-pen");
        assert_eq!(editing.width(), before.width(), "typing {typed}");
        assert_eq!(editing.left(), before.left(), "typing {typed}");
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(s.saved().drawing.widths.pen, kept, "typed {typed}");
        assert_eq!(value_box(&h, "settings-width-pen"), before);
    }
}
