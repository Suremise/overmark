//! The screenshot harness for Overmark's egui windows (the toolbar and Settings):
//! renders named screens with `egui_kittest`'s wgpu renderer and writes PNGs to the
//! git-ignored `target/ui-shots/<name>/`, for reviewing the look without running the app. Each
//! screen is shot in light and dark, at 100% and 150%: `<screen>-<theme>-<scale>.png`.
//!
//! The shots are ignored by a plain `cargo test`; run them with
//!
//! ```text
//! $env:OVERMARK_UI_SHOTS = "before"   # the folder under target/ui-shots (default: latest)
//! cargo test -p overmark --test ui_shots -- --ignored
//! ```
//!
//! The screens: `gallery`, the shared components ([`gallery`]), and Settings' pages
//! (`settings-general`, `settings-drawing`, `settings-capture`, `settings-about`) over
//! default settings in a temp folder, and the toolbar's ([`screens`]). The gallery is
//! also driven through the accessibility tree, as a plain test.
//!
//! `Variant` and `save` are Replica's (`crates/replica/tests/ui_shots.rs`), copied and adapted.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use eframe::egui::{self, Theme};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, HarnessBuilder};
use overmark::config::Startup;
use overmark::settings_window::{Host, Page, SettingsApp};
use overmark::ui::{self, icons::Icon, widgets};
use overmark_core::hotkey::Hotkey;
use overmark_core::hotkey_status::Status;
use overmark_core::i18n::tr;
use overmark_win::paths::AppPaths;

/// One way a screen is shot.
#[derive(Debug, Clone, Copy)]
struct Variant {
    theme: Theme,
    scale: f32,
}

impl Variant {
    fn all() -> Vec<Variant> {
        let mut all = Vec::new();
        for theme in [Theme::Light, Theme::Dark] {
            for scale in [1.0, 1.5] {
                all.push(Variant { theme, scale });
            }
        }
        all
    }

    fn file(self, screen: &str) -> String {
        format!(
            "{screen}-{}-{}.png",
            match self.theme {
                Theme::Light => "light",
                Theme::Dark => "dark",
            },
            (self.scale * 100.0) as u32
        )
    }

    /// A harness of `width` × `height` points; it puts its own margin round the screen, which
    /// [`save`] crops off.
    fn sized<S>(self, width: f32, height: f32) -> HarnessBuilder<S> {
        Harness::builder()
            .with_size([width + 2.0 * MARGIN, height + 2.0 * MARGIN])
            .with_pixels_per_point(self.scale)
            .with_theme(self.theme)
            .with_os(egui::os::OperatingSystem::Windows)
            .with_max_steps(64)
            .with_render_cursor(false)
            .with_context_setup(|ctx| ui::install(ctx).unwrap())
            .wgpu()
    }
}

/// `egui_kittest`'s margin round a screen, in points.
const MARGIN: f32 = 8.0;

/// Where the PNGs go: `target/ui-shots/<OVERMARK_UI_SHOTS or latest>/`.
fn out_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let name = std::env::var("OVERMARK_UI_SHOTS").unwrap_or_else(|_| "latest".into());
    target.join("ui-shots").join(name)
}

/// Lets `harness` settle, then writes it, without the harness's margin.
fn save<S>(harness: &mut Harness<'_, S>, scale: f32, path: &Path) {
    for _ in 0..8 {
        if harness.try_run().is_ok() {
            break;
        }
    }
    let image = harness.render().expect("rendered with wgpu");
    let m = (MARGIN * scale).round() as u32;
    let (w, h) = image.dimensions();
    image::imageops::crop_imm(&image, m, m, w - 2 * m, h - 2 * m)
        .to_image()
        .save(path)
        .unwrap();
}

/// What the gallery holds, kept between frames.
struct Gallery {
    start_with_windows: bool,
    tab: usize,
    format: usize,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery {
            start_with_windows: true,
            tab: 0,
            format: 0,
        }
    }
}

/// The shared components, as a Settings-like page, in Overmark's own words.
fn gallery(ui: &mut egui::Ui, g: &mut Gallery) {
    egui::Frame::new()
        .fill(ui::theme::of(ui).background)
        .inner_margin(16.0)
        .show(ui, |ui| {
            widgets::page_title(ui, &tr("settings-title"));
            let tabs = [
                tr("settings-general"),
                tr("settings-drawing"),
                tr("settings-capture"),
                tr("settings-about"),
            ];
            let tabs: Vec<&str> = tabs.iter().map(String::as_str).collect();
            widgets::tab_strip(ui, &tr("settings-title"), &tabs, &mut g.tab);
            ui.add_space(12.0);
            widgets::section_card(ui, &tr("settings-general"), None, |ui| {
                widgets::setting_row(ui, &tr("settings-start-with-windows"), None, |ui| {
                    widgets::toggle(ui, &mut g.start_with_windows)
                });
                widgets::setting_row(ui, &tr("settings-format"), None, |ui| {
                    let (response, chosen) = widgets::segmented(
                        ui,
                        &[&tr("settings-format-png"), &tr("settings-format-jpg")],
                        g.format,
                    );
                    if let Some(c) = chosen {
                        g.format = c;
                    }
                    response
                });
                widgets::path_row(
                    ui,
                    &tr("settings-save-location"),
                    Some(&tr("settings-save-location-default")),
                    r"C:\Users\Someone\Pictures\Overmark",
                );
            });
            ui.add_space(12.0);
            widgets::info_bar(
                ui,
                widgets::InfoKind::Info,
                None,
                &tr("settings-check-updates-help"),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                widgets::primary_button(ui, &tr("settings-close"));
                widgets::secondary_button(ui, &tr("settings-cancel"));
                widgets::danger_button(ui, Some(Icon::Delete), &tr("settings-reset"));
                widgets::icon_button(ui, Icon::Folder, &tr("settings-open-log-folder"));
            });
        });
}

/// A world that does nothing, for Settings' screens.
struct Still(PathBuf);

impl Host for Still {
    fn autostart(&self) -> bool {
        true
    }
    fn set_autostart(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn open(&self, _: &str) -> bool {
        true
    }
    fn pick_folder(&self, _: &Path) -> Option<PathBuf> {
        None
    }
    fn reload(&self) {}
    fn hotkey_status(&self) -> Option<Status> {
        None
    }
    fn is_free(&self, _: &Hotkey) -> bool {
        true
    }
    fn pictures(&self) -> PathBuf {
        self.0.clone()
    }
}

/// Settings on `page`, over default settings in a temp folder of its own (installed).
fn settings_screen(v: Variant, path: &Path, page: Page) {
    settings_screen_sized(v, path, page, 640.0);
}

/// [`settings_screen`], `height` points tall: a whole page, to see its rows line up.
fn settings_screen_sized(v: Variant, path: &Path, page: Page, height: f32) {
    let temp = tempfile::tempdir().unwrap();
    let (exe, appdata) = (temp.path().join("app"), temp.path().join("appdata"));
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::create_dir_all(&appdata).unwrap();
    let startup = Startup::load(AppPaths::resolve("Overmark", &exe, Some(&appdata)));
    let pictures = PathBuf::from(r"C:\Users\Someone\Pictures");
    let app = SettingsApp::new(&startup, Box::new(Still(pictures)), page);
    let mut harness = v
        .sized(780.0, height)
        .build_ui_state(|ui, app: &mut SettingsApp| app.show(ui), app);
    save(&mut harness, v.scale, path);
}

/// Shoots a screen in a variant to a file.
type Shoot = Box<dyn Fn(Variant, &Path)>;

/// The screens, by name, the toolbar's among them.
fn screens() -> Vec<(&'static str, Shoot)> {
    let mut screens: Vec<(&'static str, Shoot)> = vec![(
        "gallery",
        Box::new(|v: Variant, path: &Path| {
            let mut harness = v
                .sized(720.0, 560.0)
                .build_ui_state(gallery, Gallery::default());
            save(&mut harness, v.scale, path);
        }),
    )];
    for (name, page) in [
        ("settings-general", Page::General),
        ("settings-drawing", Page::Drawing),
        ("settings-capture", Page::Capture),
        ("settings-about", Page::About),
    ] {
        screens.push((
            name,
            Box::new(move |v: Variant, path: &Path| settings_screen(v, path, page)),
        ));
    }
    // The whole Drawing page, its value boxes and sliders lined up.
    screens.push((
        "settings-drawing-whole",
        Box::new(|v: Variant, path: &Path| settings_screen_sized(v, path, Page::Drawing, 1500.0)),
    ));
    // The toolbar, expanded and Frozen (its backdrop's button; Unfreeze), and
    // collapsed.
    screens.push((
        "toolbar",
        Box::new(|v: Variant, path: &Path| toolbar_shot(v, path, false, None)),
    ));
    // Each backdrop on its button, its strip open (Frozen is the toolbar's own).
    for (name, backdrop) in [
        (
            "toolbar-backdrop-live",
            overmark_core::backdrop::Backdrop::Live,
        ),
        (
            "toolbar-backdrop-frozen",
            overmark_core::backdrop::Backdrop::Frozen,
        ),
        (
            "toolbar-backdrop-whiteboard",
            overmark_core::backdrop::Backdrop::Whiteboard,
        ),
        (
            "toolbar-backdrop-blackboard",
            overmark_core::backdrop::Backdrop::Blackboard,
        ),
    ] {
        screens.push((
            name,
            Box::new(move |v: Variant, path: &Path| {
                toolbar_shot_of(v, path, Some("Backdrop"), |s| s.backdrop = backdrop)
            }),
        ));
    }
    // The freeze button, an amber pause in Live, a green play while Frozen and greyed
    // out on a board, with no strip open.
    for (name, backdrop) in [
        (
            "toolbar-freeze-live",
            overmark_core::backdrop::Backdrop::Live,
        ),
        (
            "toolbar-freeze-frozen",
            overmark_core::backdrop::Backdrop::Frozen,
        ),
        (
            "toolbar-freeze-whiteboard",
            overmark_core::backdrop::Backdrop::Whiteboard,
        ),
    ] {
        screens.push((
            name,
            Box::new(move |v: Variant, path: &Path| {
                toolbar_shot_of(v, path, None, |s| s.backdrop = backdrop)
            }),
        ));
    }
    // Live, with the eraser: the width's button greyed out, keeping its place.
    screens.push((
        "toolbar-eraser",
        Box::new(|v: Variant, path: &Path| {
            use overmark_core::backdrop::Backdrop;
            use overmark_core::tool::ToolKind;
            toolbar_shot_of(v, path, None, |s| {
                s.backdrop = Backdrop::Live;
                s.tool = ToolKind::Eraser;
                s.width = None;
            })
        }),
    ));
    screens.push((
        "toolbar-collapsed",
        Box::new(|v: Variant, path: &Path| toolbar_shot(v, path, true, None)),
    ));
    // Each group's strip, open below the toolbar (the slider is the width's).
    for (name, button) in [
        ("toolbar-strip-backdrop", "Backdrop"),
        ("toolbar-strip-guides", "Guides"),
        ("toolbar-strip-tool", "Tool"),
        ("toolbar-strip-colour", "Colour"),
        ("toolbar-strip-width", "Width: 4 px"),
    ] {
        screens.push((
            name,
            Box::new(move |v: Variant, path: &Path| toolbar_shot(v, path, false, Some(button))),
        ));
    }
    screens
}

fn toolbar_shot(v: Variant, path: &Path, collapsed: bool, open: Option<&str>) {
    toolbar_shot_of(v, path, open, |s| s.collapsed = collapsed);
}

/// The toolbar over its usual state (Frozen, the pen), changed by `tweak`, with `open`'s strip
/// open.
fn toolbar_shot_of(
    v: Variant,
    path: &Path,
    open: Option<&str>,
    tweak: impl FnOnce(&mut overmark::toolbar::State),
) {
    use overmark::toolbar::State;
    use overmark_core::backdrop::{Backdrop, Guides};
    use overmark_core::settings::DEFAULT_PALETTE;
    use overmark_core::tool::ToolKind;
    let state = State {
        backdrop: Backdrop::Frozen,
        guides: Guides::Grid,
        tool: ToolKind::Pen,
        colour: DEFAULT_PALETTE[0],
        palette: DEFAULT_PALETTE.to_vec(),
        width: Some(4.0),
        can_undo: true,
        can_redo: false,
        collapsed: false,
        disappearing_ink: false,
        right_click_undoes: true,
        recording: false,
    };
    let mut state = state;
    tweak(&mut state);
    let height = if open.is_some() { 120.0 } else { 70.0 };
    let mut harness = v.sized(1040.0, height).build_ui_state(
        |ui, state: &mut State| {
            overmark::toolbar::ui::show(ui.ctx(), state);
        },
        state,
    );
    if let Some(button) = open {
        harness.run();
        harness.get_by_label(button).click_accesskit();
        harness.run();
    }
    save(&mut harness, v.scale, path);
}

#[test]
#[ignore = "screenshots: run with --ignored (wgpu)"]
fn ui_shots() {
    let out = out_dir();
    std::fs::create_dir_all(&out).unwrap();
    let mut written = 0;
    for (name, shoot) in screens() {
        for v in Variant::all() {
            shoot(v, &out.join(v.file(name)));
            written += 1;
        }
    }
    assert!(written > 0);
    println!("{written} screenshots in {}", out.display());
}

/// The gallery through the accessibility tree (what Narrator reads): the components are named
/// in Overmark's words, and the toggle answers a click.
#[test]
fn the_components_are_named_and_answer() {
    let mut harness = Harness::builder()
        .with_size([720.0, 560.0])
        .with_max_steps(64)
        .with_context_setup(|ctx| ui::install(ctx).unwrap())
        .build_ui_state(gallery, Gallery::default());
    harness.run();
    for name in [
        tr("settings-close"),
        tr("settings-cancel"),
        tr("settings-reset"),
        tr("settings-open-log-folder"),
    ] {
        harness.get_by_label(&name);
    }
    assert!(harness.state().start_with_windows);
    harness
        .get_by_label(&tr("settings-start-with-windows"))
        .click();
    harness.run();
    assert!(
        !harness.state().start_with_windows,
        "the toggle switched off"
    );
}
