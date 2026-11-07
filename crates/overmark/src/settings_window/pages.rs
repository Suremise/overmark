//! The Settings window's pages, drawn with the shared components (`crate::ui`).

use eframe::egui::{self, Color32, RichText, Ui, color_picker::Alpha};
use overmark_core::colour::Colour;
use overmark_core::hotkey::{Hotkey, Key};
use overmark_core::hotkey_status::State;
use overmark_core::i18n::tr;
use overmark_core::settings::file_name::{self, CaptureTime};
use overmark_core::settings::{
    AfterCapture, DIM_RANGE, GuideSettings, INK_SECONDS_RANGE, ImageFormat, LASER_TRAIL_RANGE,
    PALETTE_SIZE, PICTURES_TOKEN, PrintScreen, TrayClick, Widths,
};
use overmark_core::tool::ToolKind;
use overmark_core::tr;
use overmark_core::update::RELEASES_URL;
use overmark_win::paths::Mode;

use super::{NOTICES, Page, SettingsApp};
use crate::ui::theme::{self, space};
use crate::ui::widgets::{
    InfoKind, danger_button, dropdown, dropdown_value, flow, info_bar, info_row, page_title,
    path_field, plain_card, primary_button, secondary_button, section_card, segmented, setting_row,
    tab_strip, toggle,
};

/// The project's links (About).
const WEBSITE_URL: &str = "https://surmise.it/tools/overmark/";
const SOURCE_URL: &str = "https://github.com/Suremise/overmark";
/// The author, shown in About as the family's other apps show it; a name, so not translated.
const AUTHOR: &str = "Darren Marshall";
const ISSUES_URL: &str = "https://github.com/Suremise/overmark/issues";

/// A hotkey Settings can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Draw,
    DrawFrozen,
    CaptureRegion,
    RecordRegion,
    Whiteboard,
    Blackboard,
}

impl Slot {
    pub const ALL: [Slot; 6] = [
        Slot::Draw,
        Slot::DrawFrozen,
        Slot::CaptureRegion,
        Slot::RecordRegion,
        Slot::Whiteboard,
        Slot::Blackboard,
    ];

    /// The setting's key in the file, as the tray's hotkey status names it.
    pub fn key(self) -> &'static str {
        match self {
            Slot::Draw => "general.hotkey_draw",
            Slot::DrawFrozen => "general.hotkey_draw_frozen",
            Slot::CaptureRegion => "general.hotkey_capture_region",
            Slot::RecordRegion => "general.hotkey_record_region",
            Slot::Whiteboard => "backdrops.hotkey_whiteboard",
            Slot::Blackboard => "backdrops.hotkey_blackboard",
        }
    }

    pub fn label(self) -> String {
        tr(match self {
            Slot::Draw => "settings-hotkey-draw",
            Slot::DrawFrozen => "settings-hotkey-draw-frozen",
            Slot::CaptureRegion => "settings-hotkey-capture-region",
            Slot::RecordRegion => "settings-hotkey-record-region",
            Slot::Whiteboard => "settings-hotkey-whiteboard",
            Slot::Blackboard => "settings-hotkey-blackboard",
        })
    }

    pub fn text(self, s: &overmark_core::settings::Settings) -> &String {
        match self {
            Slot::Draw => &s.general.hotkey_draw,
            Slot::DrawFrozen => &s.general.hotkey_draw_frozen,
            Slot::CaptureRegion => &s.general.hotkey_capture_region,
            Slot::RecordRegion => &s.general.hotkey_record_region,
            Slot::Whiteboard => &s.backdrops.hotkey_whiteboard,
            Slot::Blackboard => &s.backdrops.hotkey_blackboard,
        }
    }

    fn text_mut(self, s: &mut overmark_core::settings::Settings) -> &mut String {
        match self {
            Slot::Draw => &mut s.general.hotkey_draw,
            Slot::DrawFrozen => &mut s.general.hotkey_draw_frozen,
            Slot::CaptureRegion => &mut s.general.hotkey_capture_region,
            Slot::RecordRegion => &mut s.general.hotkey_record_region,
            Slot::Whiteboard => &mut s.backdrops.hotkey_whiteboard,
            Slot::Blackboard => &mut s.backdrops.hotkey_blackboard,
        }
    }
}

/// A hotkey being changed: the keys chosen so far, and what's wrong with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyEdit {
    pub slot: Slot,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
    /// Why the last try wasn't taken, and a free key near it.
    pub problem: Option<String>,
    pub suggestion: Option<Hotkey>,
}

impl HotkeyEdit {
    fn new(slot: Slot, current: Option<Hotkey>) -> HotkeyEdit {
        let h = current.unwrap_or(Hotkey {
            ctrl: false,
            alt: false,
            shift: true,
            win: true,
            key: Key::Letter('W'),
        });
        HotkeyEdit {
            slot,
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            win: h.win,
            key: h.key,
            problem: None,
            suggestion: None,
        }
    }

    fn hotkey(&self) -> Hotkey {
        Hotkey {
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            win: self.win,
            key: self.key,
        }
    }
}

/// The window: the title, warnings, the tab strip and the page.
pub fn show(app: &mut SettingsApp, ui: &mut Ui) {
    let p = theme::of(ui);
    egui::Frame::new()
        .fill(p.background)
        .inner_margin(space::XL)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_title(ui, &tr("settings-title"));
            notices(app, ui);
            let titles: Vec<String> = Page::ALL.iter().map(|p| p.title()).collect();
            let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
            let mut index = app.page.index();
            if tab_strip(ui, &tr("settings-title"), &titles, &mut index) {
                app.page = Page::ALL[index];
            }
            ui.add_space(space::M);
            egui::ScrollArea::vertical()
                .id_salt(("settings-page", app.page.index()))
                .auto_shrink([false, false])
                .show(ui, |ui| match app.page {
                    Page::General => general(app, ui),
                    Page::Drawing => drawing(app, ui),
                    Page::Capture => capture(app, ui),
                    Page::About => about(app, ui),
                });
        });
    confirm_reset(app, ui.ctx());
}

/// Problems worth seeing before anything else: the settings can't be saved, the last save
/// failed, the file had something ignored or changed.
fn notices(app: &mut SettingsApp, ui: &mut Ui) {
    let mut shown = false;
    if !app.can_save {
        let path = app
            .about
            .settings_file
            .as_ref()
            .and_then(|f| f.parent())
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let text = if app.about.mode == Some(Mode::Portable) {
            tr!("error-portable-not-writable", path = path)
        } else {
            tr!("error-app-folder-not-writable", path = path)
        };
        info_bar(
            ui,
            InfoKind::Warning,
            Some(&tr("error-folder-not-writable-title")),
            &text,
        );
        shown = true;
    }
    for text in [&app.save_error, &app.autostart_error]
        .into_iter()
        .flatten()
    {
        info_bar(ui, InfoKind::Error, None, text);
        shown = true;
    }
    if !app.warnings.is_empty() {
        let text = app
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        info_bar(
            ui,
            InfoKind::Warning,
            Some(&tr("settings-warnings-title")),
            &text,
        );
        shown = true;
    }
    if shown {
        ui.add_space(space::M);
    }
}

// --- General ---------------------------------------------------------------------------------

fn general(app: &mut SettingsApp, ui: &mut Ui) {
    plain_card(ui, None, |ui| {
        let mut on = app.host.autostart();
        if setting_row(
            ui,
            &tr("settings-start-with-windows"),
            Some(&tr("settings-start-with-windows-help")),
            |ui| toggle(ui, &mut on),
        )
        .changed()
        {
            app.set_autostart(on);
        }
        let click = &mut app.settings.general.tray_click;
        let name = |c: TrayClick| {
            tr(match c {
                TrayClick::Live => "tray-click-live",
                TrayClick::Frozen => "tray-click-frozen",
                TrayClick::Settings => "tray-click-settings",
            })
        };
        let label = tr("settings-tray-click");
        setting_row(ui, &label, Some(&tr("settings-tray-click-help")), |ui| {
            dropdown(ui, "tray-click", None, &name(*click), 260.0, |ui| {
                for c in [TrayClick::Live, TrayClick::Frozen, TrayClick::Settings] {
                    dropdown_value(ui, click, c, name(c));
                }
            })
            .accessible_name(&label)
        });
    });
    ui.add_space(space::M);
    section_card(
        ui,
        &tr("settings-hotkeys"),
        Some(&tr("settings-hotkeys-help")),
        |ui| {
            for slot in Slot::ALL {
                hotkey_row(app, ui, slot);
            }
            print_screen_row(app, ui);
        },
    );
}

/// The hotkeys Overmark has other than `slot`'s: none may take another's key.
fn other_hotkeys(app: &SettingsApp, slot: Option<Slot>) -> Vec<(Hotkey, String)> {
    let mut out: Vec<(Hotkey, String)> = Slot::ALL
        .iter()
        .filter(|s| Some(**s) != slot)
        .filter_map(|s| {
            Hotkey::parse(s.text(&app.settings))
                .ok()
                .flatten()
                .map(|h| (h, s.label()))
        })
        .collect();
    if slot.is_some() && app.settings.general.print_screen != PrintScreen::Off {
        let prt = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: Key::PrintScreen,
        };
        out.push((prt, tr("settings-print-screen")));
    }
    out
}

fn hotkey_row(app: &mut SettingsApp, ui: &mut Ui, slot: Slot) {
    let text = slot.text(&app.settings).clone();
    let parsed = Hotkey::parse(&text);
    let shown = match &parsed {
        Ok(Some(h)) => h.display(),
        Ok(None) => tr("settings-hotkey-none"),
        Err(_) => text.clone(),
    };
    // What's wrong with it now: the file's text, or what the tray found.
    let problem = match (&parsed, app.status.as_ref().and_then(|s| s.get(slot.key()))) {
        (Err(_), _) | (_, Some(State::Invalid)) => {
            Some(tr!("settings-hotkey-invalid", hotkey = text.as_str()))
        }
        (Ok(Some(h)), Some(State::Taken)) => Some(tr!("hotkey-taken", hotkey = h.display())),
        (Ok(Some(h)), Some(State::Failed)) => {
            Some(tr!("settings-hotkey-failed", hotkey = h.display()))
        }
        _ => None,
    };
    let editing = app.edit.as_ref().is_some_and(|e| e.slot == slot);
    let label = slot.label();
    let mut change = false;
    let mut remove = false;
    // The row's buttons keep their own names ("Change the hotkey for Draw"); the hotkey is
    // read as text beside them.
    info_row(ui, &label, None, |ui| {
        let r = if editing {
            ui.label(RichText::new(&shown).color(theme::of(ui).text_secondary))
        } else {
            let r = secondary_button(ui, &tr("settings-hotkey-change"))
                .accessible_name(tr!("settings-hotkey-change-named", action = label.as_str()));
            change = r.clicked();
            r
        };
        if !text.is_empty() && !editing {
            let named = tr!("settings-hotkey-clear-named", action = label.as_str());
            remove = secondary_button(ui, &tr("settings-hotkey-clear"))
                .accessible_name(&named)
                .on_hover_text(&named)
                .clicked();
        }
        ui.label(RichText::new(&shown).strong());
        r
    });
    if let Some(p) = &problem {
        crate::ui::widgets::error_text(ui, p);
    }
    if change {
        app.edit = Some(HotkeyEdit::new(slot, parsed.ok().flatten()));
    }
    if remove {
        slot.text_mut(&mut app.settings).clear();
    }
    if editing {
        hotkey_editor(app, ui);
    }
}

/// The keys of the hotkey being changed: Win, Ctrl, Alt and Shift as toggles, and the key.
fn hotkey_editor(app: &mut SettingsApp, ui: &mut Ui) {
    let Some(mut edit) = app.edit.clone() else {
        return;
    };
    let mut apply = None;
    let mut cancel = false;
    egui::Frame::new()
        .fill(theme::of(ui).background)
        .corner_radius(theme::CONTROL_RADIUS)
        .inner_margin(space::M)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (on, name) in [
                    (&mut edit.win, "key-win"),
                    (&mut edit.ctrl, "key-ctrl"),
                    (&mut edit.alt, "key-alt"),
                    (&mut edit.shift, "key-shift"),
                ] {
                    ui.toggle_value(on, tr(name));
                }
                ui.label("+");
                let key_label = tr("settings-hotkey-key");
                let current = edit.key.display();
                dropdown(ui, "hotkey-key", None, &current, 160.0, |ui| {
                    for k in Key::all() {
                        dropdown_value(ui, &mut edit.key, k, k.display());
                    }
                })
                .accessible_name(&key_label);
            });
            let h = edit.hotkey();
            let alone = !(h.ctrl || h.alt || h.shift || h.win);
            if alone && h.key != Key::PrintScreen {
                crate::ui::widgets::error_text(ui, &tr("hotkey-needs-modifier"));
            } else if h.is_ctrl_alt() {
                note(ui, &tr("hotkey-ctrl-alt-warning"));
            }
            if let Some(p) = &edit.problem {
                crate::ui::widgets::error_text(ui, p);
            }
            ui.add_space(space::S);
            ui.horizontal(|ui| {
                let ok = !alone || h.key == Key::PrintScreen;
                if ui
                    .add_enabled_ui(ok, |ui| primary_button(ui, &tr("settings-hotkey-use")))
                    .inner
                    .clicked()
                {
                    apply = Some(h);
                }
                if let Some(s) = edit.suggestion
                    && secondary_button(
                        ui,
                        &tr!("settings-hotkey-use-instead", hotkey = s.display()),
                    )
                    .clicked()
                {
                    apply = Some(s);
                }
                cancel = secondary_button(ui, &tr("settings-cancel")).clicked();
            });
        });
    app.edit = Some(edit);
    if cancel {
        app.edit = None;
    } else if let Some(h) = apply {
        try_hotkey(app, h);
    }
}

/// Takes `hotkey` for the slot being changed if no other Overmark hotkey and no other app has
/// it; otherwise says why and suggests the nearest free one.
pub(crate) fn try_hotkey(app: &mut SettingsApp, hotkey: Hotkey) {
    let Some(slot) = app.edit.as_ref().map(|e| e.slot) else {
        return;
    };
    let others = other_hotkeys(app, Some(slot));
    let ours: Vec<Hotkey> = others.iter().map(|(h, _)| *h).collect();
    let current = Hotkey::parse(slot.text(&app.settings)).ok().flatten();
    let problem = if let Some((_, action)) = others.iter().find(|(h, _)| *h == hotkey) {
        Some(tr!(
            "hotkey-duplicate",
            hotkey = hotkey.display(),
            action = action.as_str()
        ))
    } else if Some(hotkey) != current && !app.host.is_free(&hotkey) {
        Some(tr!("hotkey-taken", hotkey = hotkey.display()))
    } else {
        None
    };
    match problem {
        None => {
            *slot.text_mut(&mut app.settings) = hotkey.to_setting();
            app.edit = None;
        }
        Some(p) => {
            let suggestion = hotkey
                .alternatives()
                .into_iter()
                .filter(|h| !ours.contains(h))
                .find(|h| app.host.is_free(h));
            if let Some(edit) = app.edit.as_mut() {
                let h = hotkey;
                (edit.ctrl, edit.alt, edit.shift, edit.win, edit.key) =
                    (h.ctrl, h.alt, h.shift, h.win, h.key);
                edit.problem = Some(p);
                edit.suggestion = suggestion;
            }
        }
    }
}

fn print_screen_row(app: &mut SettingsApp, ui: &mut Ui) {
    let name = |p: PrintScreen| {
        tr(match p {
            PrintScreen::Off => "settings-print-screen-off",
            PrintScreen::Live => "tray-draw",
            PrintScreen::Frozen => "tray-draw-frozen",
        })
    };
    let problem = match app
        .status
        .as_ref()
        .and_then(|s| s.get("general.print_screen"))
    {
        Some(State::Taken | State::Failed) => Some(tr("settings-print-screen-taken")),
        _ => None,
    };
    let label = tr("settings-print-screen");
    let current = app.settings.general.print_screen;
    let mut chosen = current;
    setting_row(
        ui,
        &label,
        Some(&flow(&tr("hotkey-print-screen-help"))),
        |ui| {
            dropdown(ui, "print-screen", None, &name(current), 260.0, |ui| {
                for p in [PrintScreen::Off, PrintScreen::Live, PrintScreen::Frozen] {
                    dropdown_value(ui, &mut chosen, p, name(p));
                }
            })
            .accessible_name(&label)
        },
    );
    if let Some(p) = problem {
        crate::ui::widgets::error_text(ui, &p);
    }
    if chosen != current {
        // Print Screen can't be another Overmark hotkey's key too.
        let prt_taken = Slot::ALL.iter().any(|s| {
            Hotkey::parse(s.text(&app.settings))
                .ok()
                .flatten()
                .is_some_and(|h| {
                    h.key == Key::PrintScreen && !(h.ctrl || h.alt || h.shift || h.win)
                })
        });
        if chosen == PrintScreen::Off || !prt_taken {
            app.settings.general.print_screen = chosen;
        }
    }
}

// --- Drawing ---------------------------------------------------------------------------------

fn colour32(c: Colour) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn colour(c: Color32) -> Colour {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Colour { r, g, b, a }
}

/// A colour button named `name` for a screen reader; true when the colour changed.
fn colour_button(ui: &mut Ui, value: &mut Colour, name: &str, alpha: Alpha) -> egui::Response {
    let mut c = colour32(*value);
    // Outlined, so a white swatch shows on a light card.
    let border = theme::of(ui).control_border;
    let r = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(3.0)
        .show(ui, |ui| {
            egui::color_picker::color_edit_button_srgba(ui, &mut c, alpha)
        })
        .inner
        .accessible_name(name);
    let picked = colour(c);
    if picked != *value {
        *value = picked;
    }
    r
}

/// How a slider row's number is written ("4 px", "35%") and read back from what's typed in its
/// box.
#[derive(Clone, Copy)]
struct Numbers {
    format: fn(f64) -> String,
    parse: fn(&str) -> Option<f64>,
}

/// Pixels: "4 px".
const PX: Numbers = Numbers {
    format: |v| tr!("toolbar-width-value", width = format!("{v:.0}")),
    parse: leading_number,
};
/// The dim: "Off", or a percentage of the 0 to 1 it's kept as.
const DIM: Numbers = Numbers {
    format: |v| {
        if v <= 0.0 {
            tr("settings-dim-off")
        } else {
            tr!("settings-percent", percent = (v * 100.0).round() as i64)
        }
    },
    parse: |s| match leading_number(s) {
        Some(percent) => Some(percent / 100.0),
        None => s
            .trim()
            .eq_ignore_ascii_case(&tr("settings-dim-off"))
            .then_some(0.0),
    },
};
/// Seconds, to a tenth: "3 seconds".
const SECONDS: Numbers = Numbers {
    format: |v| tr!("settings-seconds", seconds = (v * 10.0).round() / 10.0),
    parse: leading_number,
};
/// Milliseconds: "500 ms".
const MS: Numbers = Numbers {
    format: |v| tr!("settings-ms", ms = v.round() as u64),
    parse: leading_number,
};
/// A plain number: JPG's quality.
const PLAIN: Numbers = Numbers {
    format: |v| format!("{v:.0}"),
    parse: leading_number,
};

/// The first number in `text` ("12 px" is 12, "0.5 seconds" 0.5), with its unit or without.
fn leading_number(text: &str) -> Option<f64> {
    let start = text.find(|c: char| c.is_ascii_digit() || c == '.' || c == '-')?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// What each value of `range`, in steps of `step`, looks like written by `numbers`.
fn every_value(range: std::ops::RangeInclusive<f64>, step: f64, numbers: Numbers) -> Vec<String> {
    let (lo, hi) = (*range.start(), *range.end());
    let steps = ((hi - lo) / step).round() as u64;
    (0..=steps)
        .map(|i| (numbers.format)((lo + i as f64 * step).min(hi)))
        .collect()
}

/// The value boxes of a card's sliders, all one width: the widest text any of them can show
/// (`texts`, every value each can take), in the current font and scale, so the sliders beside
/// them line up and no box grows as its number does. Measured once for a scale and kept,
/// under `key`.
fn value_box_width(ui: &Ui, key: &str, texts: impl FnOnce() -> Vec<String>) -> f32 {
    let id = egui::Id::unique(("value-box-width", key));
    let scale = ui.ctx().pixels_per_point();
    if let Some((at, width)) = ui.data(|d| d.get_temp::<(f32, f32)>(id))
        && at == scale
    {
        return width;
    }
    let font = ui.style().drag_value_text_style.resolve(ui.style());
    let painter = ui.painter();
    let text = texts()
        .into_iter()
        .map(|t| {
            painter
                .layout_no_wrap(t, font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let width = (text + 2.0 * ui.spacing().button_padding.x).ceil();
    ui.data_mut(|d| d.insert_temp(id, (scale, width)));
    width
}

/// A slider row: `value` within `range` on a slider, and in a box of `width` beside it (its
/// card's [`value_box_width`]) to type into. The box keeps its width as you type, the text
/// scrolling inside it, and what's typed is kept in `range`.
fn slider_row<N: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    help: Option<&str>,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    numbers: Numbers,
    width: f32,
) {
    let span = range.end().to_f64() - range.start().to_f64();
    setting_row(ui, label, help, |ui| {
        // Right to left: the slider at the row's end, its box before it.
        let slider = ui.add(
            egui::Slider::new(value, range.clone())
                .custom_formatter(move |v, _| (numbers.format)(v))
                .show_value(false)
                .text(""),
        );
        let size = egui::vec2(width, ui.spacing().interact_size.y);
        let speed = span / f64::from(ui.spacing().slider_width.max(1.0));
        ui.add_sized(
            size,
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .custom_formatter(move |v, _| (numbers.format)(v))
                .custom_parser(numbers.parse)
                .clip_text(true)
                .min_size(size),
        )
        .accessible_name(tr!("settings-value-box", setting = label));
        slider
    });
}

fn drawing(app: &mut SettingsApp, ui: &mut Ui) {
    let d = &mut app.settings.drawing;
    plain_card(ui, None, |ui| {
        let tool_label = tr("settings-default-tool");
        let current = d.default_tool;
        setting_row(ui, &tool_label, None, |ui| {
            dropdown(ui, "default-tool", None, &current.name(), 260.0, |ui| {
                for t in ToolKind::ALL {
                    dropdown_value(ui, &mut d.default_tool, t, t.name());
                }
            })
            .accessible_name(&tool_label)
        });
    });
    ui.add_space(space::M);
    section_card(
        ui,
        &tr("settings-palette"),
        Some(&tr("settings-palette-help")),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, c) in d.palette.iter_mut().enumerate().take(PALETTE_SIZE) {
                    ui.vertical(|ui| {
                        let name = tr!("settings-colour-n", n = i + 1);
                        colour_button(ui, c, &name, Alpha::OnlyBlend);
                        ui.label(RichText::new((i + 1).to_string()).small());
                    });
                }
            });
            let label = tr("settings-default-colour");
            let n = d.default_colour;
            setting_row(ui, &label, None, |ui| {
                let shown = tr!("settings-colour-n", n = n + 1);
                dropdown(ui, "default-colour", None, &shown, 200.0, |ui| {
                    for i in 0..PALETTE_SIZE {
                        dropdown_value(
                            ui,
                            &mut d.default_colour,
                            i,
                            tr!("settings-colour-n", n = i + 1),
                        );
                    }
                })
                .accessible_name(&label)
            });
        },
    );
    ui.add_space(space::M);
    section_card(ui, &tr("settings-default-widths"), None, |ui| {
        let (lo, hi) = Widths::RANGE;
        let boxes = value_box_width(ui, "widths", || every_value(lo as f64..=hi as f64, 1.0, PX));
        let w = &mut d.widths;
        slider_row(
            ui,
            &tr("settings-width-pen"),
            None,
            &mut w.pen,
            lo..=hi,
            PX,
            boxes,
        );
        slider_row(
            ui,
            &tr("settings-width-highlighter"),
            None,
            &mut w.highlighter,
            lo..=hi,
            PX,
            boxes,
        );
        // One width for every shape and arrow.
        let mut shapes = w.arrow;
        slider_row(
            ui,
            &tr("settings-width-shapes"),
            None,
            &mut shapes,
            lo..=hi,
            PX,
            boxes,
        );
        if shapes != w.arrow {
            (w.arrow, w.rectangle, w.ellipse, w.line) = (shapes, shapes, shapes, shapes);
        }
        slider_row(
            ui,
            &tr("settings-width-text"),
            None,
            &mut w.text,
            lo..=hi,
            PX,
            boxes,
        );
        slider_row(
            ui,
            &tr("settings-width-stamp"),
            None,
            &mut w.stamp,
            lo..=hi,
            PX,
            boxes,
        );
        slider_row(
            ui,
            &tr("settings-width-blur"),
            None,
            &mut w.blur,
            lo..=hi,
            PX,
            boxes,
        );
    });
    ui.add_space(space::M);
    plain_card(ui, None, |ui| {
        let boxes = value_box_width(ui, "drawing", || {
            let mut all = every_value(f64::from(DIM_RANGE.0)..=f64::from(DIM_RANGE.1), 0.01, DIM);
            all.extend(every_value(
                f64::from(INK_SECONDS_RANGE.0)..=f64::from(INK_SECONDS_RANGE.1),
                0.1,
                SECONDS,
            ));
            all.extend(every_value(
                LASER_TRAIL_RANGE.0 as f64..=LASER_TRAIL_RANGE.1 as f64,
                1.0,
                MS,
            ));
            all
        });
        slider_row(
            ui,
            &tr("settings-dim"),
            Some(&tr("settings-dim-help")),
            &mut d.dim,
            DIM_RANGE.0..=DIM_RANGE.1,
            DIM,
            boxes,
        );
        setting_row(
            ui,
            &tr("settings-disappearing-ink"),
            Some(&tr("settings-disappearing-ink-help")),
            |ui| toggle(ui, &mut d.disappearing_ink),
        );
        ui.add_enabled_ui(d.disappearing_ink, |ui| {
            slider_row(
                ui,
                &tr("settings-disappearing-ink-delay"),
                None,
                &mut d.disappearing_ink_seconds,
                INK_SECONDS_RANGE.0..=INK_SECONDS_RANGE.1,
                SECONDS,
                boxes,
            );
        });
        let mut trail = u64::from(d.laser_trail_ms);
        slider_row(
            ui,
            &tr("settings-laser-trail"),
            None,
            &mut trail,
            LASER_TRAIL_RANGE.0..=LASER_TRAIL_RANGE.1,
            MS,
            boxes,
        );
        d.laser_trail_ms = trail as u32;
        setting_row(
            ui,
            &tr("settings-right-click-undoes"),
            Some(&tr("settings-right-click-undoes-help")),
            |ui| toggle(ui, &mut d.right_click_undoes),
        );
    });
    ui.add_space(space::M);
    let b = &mut app.settings.backdrops;
    section_card(ui, &tr("settings-backdrops"), None, |ui| {
        for (label, c) in [
            (tr("settings-whiteboard-colour"), &mut b.whiteboard_colour),
            (tr("settings-blackboard-colour"), &mut b.blackboard_colour),
        ] {
            setting_row(ui, &label, None, |ui| {
                colour_button(ui, c, &label, Alpha::Opaque)
            });
        }
    });
    ui.add_space(space::M);
    let g = &mut app.settings.guides;
    section_card(ui, &tr("settings-guides"), None, |ui| {
        let (lo, hi) = GuideSettings::SPACING_RANGE;
        let boxes = value_box_width(ui, "guides", || every_value(lo as f64..=hi as f64, 1.0, PX));
        let mut spacing = u64::from(g.spacing);
        slider_row(
            ui,
            &tr("settings-guide-spacing"),
            None,
            &mut spacing,
            lo..=hi,
            PX,
            boxes,
        );
        g.spacing = spacing as u32;
        setting_row(ui, &tr("settings-guides-on-shares"), None, |ui| {
            toggle(ui, &mut g.show_on_shares)
        });
        setting_row(ui, &tr("settings-guides-in-saves"), None, |ui| {
            toggle(ui, &mut g.include_in_saves)
        });
    });
}

// --- Capture ---------------------------------------------------------------------------------

fn capture(app: &mut SettingsApp, ui: &mut Ui) {
    let pictures = app.host.pictures();
    let resolved = app.settings.capture.resolve_save_folder(&pictures);
    let mut browse = false;
    let mut reset_folder = false;
    let is_default = app.settings.capture.save_folder
        == overmark_core::settings::CaptureSettings::default().save_folder;
    section_card(ui, &tr("settings-save-location"), None, |ui| {
        ui.horizontal(|ui| {
            ui.set_max_width(ui.available_width());
            path_field(ui, &resolved.display().to_string());
        });
        if is_default {
            note(ui, &tr("settings-save-location-default"));
        }
        ui.horizontal(|ui| {
            browse = secondary_button(ui, &tr("settings-save-location-browse")).clicked();
            if !is_default {
                reset_folder = secondary_button(ui, &tr("settings-save-location-reset")).clicked();
            }
        });
    });
    if browse && let Some(folder) = app.host.pick_folder(&resolved) {
        app.settings.capture.save_folder = folder_setting(&folder, &pictures);
    }
    if reset_folder {
        app.settings.capture.save_folder =
            overmark_core::settings::CaptureSettings::default().save_folder;
    }
    ui.add_space(space::M);
    let c = &mut app.settings.capture;
    let draft = &mut app.pattern_draft;
    plain_card(ui, None, |ui| {
        let label = tr("settings-file-name");
        // The field shows the setting, or a pattern being typed that can't be used yet (the
        // draft, which goes whenever the settings are replaced: a reset, a change on disk).
        let mut text = draft.clone().unwrap_or_else(|| c.file_name_pattern.clone());
        let check = file_name::check_pattern(&text);
        let description = match &check {
            Ok(()) => {
                let example = file_name::file_name(
                    &text,
                    &CaptureTime::EXAMPLE,
                    c.format.extension(),
                    |_| false,
                )
                .unwrap_or_default();
                tr!("settings-file-name-example", example = example)
            }
            Err(e) => e.to_string(),
        };
        setting_row(ui, &label, Some(&description), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0))
        });
        if check.is_err() {
            crate::ui::widgets::error_text(ui, &description);
        }
        // A pattern that works is taken at once; one that doesn't stays a draft.
        if file_name::check_pattern(&text).is_ok() {
            if text != c.file_name_pattern {
                c.file_name_pattern = text;
            }
            *draft = None;
        } else {
            *draft = Some(text);
        }
        note(ui, &tr("settings-file-name-help"));

        let after = [
            (
                AfterCapture::CopyAndSave,
                tr("settings-after-capture-copy-and-save"),
            ),
            (AfterCapture::Copy, tr("settings-after-capture-copy")),
            (AfterCapture::Save, tr("settings-after-capture-save")),
        ];
        let names: Vec<&str> = after.iter().map(|(_, n)| n.as_str()).collect();
        let i = after
            .iter()
            .position(|(a, _)| *a == c.after_capture)
            .unwrap_or(0);
        let mut chosen = None;
        setting_row(ui, &tr("settings-after-capture"), None, |ui| {
            let (r, clicked) = segmented(ui, &names, i);
            chosen = clicked;
            r
        });
        if let Some(n) = chosen {
            c.after_capture = after[n].0;
        }
        let formats = [ImageFormat::Png, ImageFormat::Jpg];
        let names = [tr("settings-format-png"), tr("settings-format-jpg")];
        let i = formats.iter().position(|f| *f == c.format).unwrap_or(0);
        let mut chosen = None;
        setting_row(ui, &tr("settings-format"), None, |ui| {
            let (r, clicked) = segmented(ui, &[&names[0], &names[1]], i);
            chosen = clicked;
            r
        });
        if let Some(n) = chosen {
            c.format = formats[n];
        }
        let boxes = value_box_width(ui, "capture", || every_value(1.0..=100.0, 1.0, PLAIN));
        ui.add_enabled_ui(c.format == ImageFormat::Jpg, |ui| {
            slider_row(
                ui,
                &tr("settings-jpg-quality"),
                None,
                &mut c.jpg_quality,
                1..=100,
                PLAIN,
                boxes,
            );
        });
        setting_row(ui, &tr("settings-include-drawing"), None, |ui| {
            toggle(ui, &mut c.include_drawing)
        });
        setting_row(ui, &tr("settings-toast"), None, |ui| {
            toggle(ui, &mut c.toast)
        });
    });
    ui.add_space(space::M);
    recording(app, ui);
}

/// Recording, on the Capture page: the video's frame rate, quality, pointer and encoder. Videos
/// go in the save folder above, named by the pattern above.
fn recording(app: &mut SettingsApp, ui: &mut Ui) {
    use overmark_core::record::{Encoder, FrameRate, Quality};
    let r = &mut app.settings.recording;
    section_card(
        ui,
        &tr("settings-recording"),
        Some(&tr("settings-recording-description")),
        |ui| {
            let rates = FrameRate::ALL;
            let names: Vec<String> = rates
                .iter()
                .map(|f| tr!("settings-recording-fps", fps = f.fps()))
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let i = rates.iter().position(|f| *f == r.frame_rate()).unwrap_or(0);
            let mut chosen = None;
            setting_row(ui, &tr("settings-recording-frame-rate"), None, |ui| {
                let (resp, clicked) = segmented(ui, &names, i);
                chosen = clicked;
                resp
            });
            if let Some(n) = chosen {
                r.frame_rate = rates[n].fps();
            }

            let qualities = Quality::ALL;
            let names = [
                tr("settings-recording-quality-small"),
                tr("settings-recording-quality-balanced"),
                tr("settings-recording-quality-sharp"),
            ];
            let i = qualities.iter().position(|q| *q == r.quality).unwrap_or(1);
            let mut chosen = None;
            setting_row(
                ui,
                &tr("settings-recording-quality"),
                Some(&tr("settings-recording-quality-description")),
                |ui| {
                    let (resp, clicked) = segmented(ui, &[&names[0], &names[1], &names[2]], i);
                    chosen = clicked;
                    resp
                },
            );
            if let Some(n) = chosen {
                r.quality = qualities[n];
            }

            setting_row(ui, &tr("settings-recording-pointer"), None, |ui| {
                toggle(ui, &mut r.pointer)
            });

            let encoders = [Encoder::Automatic, Encoder::Software];
            let names = [
                tr("settings-recording-encoder-automatic"),
                tr("settings-recording-encoder-software"),
            ];
            let i = encoders.iter().position(|e| *e == r.encoder).unwrap_or(0);
            let mut chosen = None;
            setting_row(
                ui,
                &tr("settings-recording-encoder"),
                Some(&tr("settings-recording-encoder-description")),
                |ui| {
                    let (resp, clicked) = segmented(ui, &[&names[0], &names[1]], i);
                    chosen = clicked;
                    resp
                },
            );
            if let Some(n) = chosen {
                r.encoder = encoders[n];
            }
        },
    );
}

/// The setting for a folder picked: `{pictures}\…` when it's in Pictures, so it follows a
/// moved Pictures folder; the path as it is otherwise.
pub(crate) fn folder_setting(folder: &std::path::Path, pictures: &std::path::Path) -> String {
    match folder.strip_prefix(pictures) {
        Ok(rest) if rest.as_os_str().is_empty() => PICTURES_TOKEN.to_owned(),
        Ok(rest) => format!(r"{PICTURES_TOKEN}\{}", rest.display()),
        Err(_) => folder.display().to_string(),
    }
}

// --- About and advanced ----------------------------------------------------------------------

fn about(app: &mut SettingsApp, ui: &mut Ui) {
    plain_card(ui, None, |ui| {
        let version = tr!(
            "settings-version-value",
            version = env!("CARGO_PKG_VERSION")
        );
        info(ui, &tr("settings-version"), &version);
        info(ui, &tr("settings-licence"), &tr("settings-licence-value"));
        info(
            ui,
            &tr("settings-author"),
            &tr!("settings-author-value", name = AUTHOR),
        );
        // The address itself, so it reads as the repository; it opens as the Links do.
        let repo = crate::ui::widgets::info_row(ui, &tr("settings-repository"), None, |ui| {
            ui.link(SOURCE_URL)
        });
        if repo.clicked() {
            app.host.open(SOURCE_URL);
        }
        if let Some(mode) = app.about.mode_label() {
            info(ui, &tr("settings-mode"), &mode);
        }
        if let Some(file) = &app.about.settings_file {
            crate::ui::widgets::path_row(
                ui,
                &tr("settings-settings-file"),
                None,
                &file.display().to_string(),
            );
        }
        ui.add_space(space::S);
        ui.horizontal_wrapped(|ui| {
            if let Some(dir) = app.about.settings_file.as_ref().and_then(|f| f.parent())
                && secondary_button(ui, &tr("settings-open-settings-folder")).clicked()
            {
                app.host.open(&dir.display().to_string());
            }
            if let Some(logs) = &app.about.logs_dir
                && secondary_button(ui, &tr("settings-open-log-folder")).clicked()
            {
                let _ = std::fs::create_dir_all(logs);
                app.host.open(&logs.display().to_string());
            }
        });
    });
    ui.add_space(space::M);
    section_card(ui, &tr("settings-links"), None, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (name, url) in [
                ("settings-link-website", WEBSITE_URL),
                ("settings-link-source", SOURCE_URL),
                ("settings-link-releases", RELEASES_URL),
                ("settings-link-issues", ISSUES_URL),
            ] {
                if ui.link(tr(name)).on_hover_text(url).clicked() {
                    app.host.open(url);
                }
            }
        });
    });
    ui.add_space(space::M);
    plain_card(ui, None, |ui| {
        setting_row(
            ui,
            &tr("settings-check-updates"),
            Some(&flow(&tr("settings-check-updates-help"))),
            |ui| toggle(ui, &mut app.settings.advanced.check_for_updates),
        );
        setting_row(
            ui,
            &tr("settings-diagnostic-logging"),
            Some(&flow(&tr("settings-diagnostic-logging-help"))),
            |ui| toggle(ui, &mut app.settings.advanced.diagnostic_logging),
        );
    });
    ui.add_space(space::M);
    section_card(
        ui,
        &tr("settings-third-party"),
        Some(&flow(&tr("settings-third-party-help"))),
        |ui| {
            note(ui, &flow(&tr("settings-third-party-crt")));
            ui.add_space(space::S);
            let lines = app.notices.get_or_insert_with(|| NOTICES.lines().collect());
            let row = ui.text_style_height(&egui::TextStyle::Monospace);
            egui::ScrollArea::vertical()
                .id_salt("third-party-notices")
                .max_height(260.0)
                .auto_shrink([false, true])
                .show_rows(ui, row, lines.len(), |ui, range| {
                    for line in &lines[range] {
                        ui.label(RichText::new(*line).monospace());
                    }
                });
        },
    );
    ui.add_space(space::M);
    section_card(
        ui,
        &tr("settings-reset"),
        Some(&tr("settings-reset-help")),
        |ui| {
            if danger_button(ui, None, &tr("settings-reset")).clicked() {
                app.confirm_reset = true;
            }
        },
    );
}

/// A line that comes second (help, a note), wrapped to the width.
fn note(ui: &mut Ui, text: &str) {
    let colour = theme::of(ui).text_secondary;
    ui.add(egui::Label::new(RichText::new(text).color(colour)).wrap());
}

/// A label and its value, as a row.
fn info(ui: &mut Ui, label: &str, value: &str) {
    crate::ui::widgets::info_row(ui, label, None, |ui| ui.label(value));
}

/// "Reset every setting?" in a small window over the page.
fn confirm_reset(app: &mut SettingsApp, ctx: &egui::Context) {
    if !app.confirm_reset {
        return;
    }
    let mut yes = false;
    let mut no = false;
    egui::Modal::new(egui::Id::unique("confirm-reset"))
        .frame(crate::ui::widgets::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(flow(&tr("settings-reset-confirm")));
            ui.add_space(space::M);
            ui.horizontal(|ui| {
                yes = crate::ui::widgets::danger_button_filled(ui, &tr("settings-reset-yes"))
                    .clicked();
                no = secondary_button(ui, &tr("settings-cancel")).clicked();
            });
        });
    if yes {
        app.reset();
        app.confirm_reset = false;
    } else if no || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.confirm_reset = false;
    }
}
