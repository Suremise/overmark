//! What the toolbar shows and what its controls do,
//! as an egui function of the drawing's state: testable with egui_kittest, drawn by
//! [`super::Toolbar`] into its window.
//!
//! Every control has an accessible name (what Narrator reads) and a tooltip with its key; each
//! does what its key does ([`Action`] goes through the same path as the key).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Response, Role, Sense, Stroke, Ui, WidgetInfo, vec2,
};
use overmark_core::backdrop::{Backdrop, Guides};
use overmark_core::colour::Colour;
use overmark_core::i18n::tr;
use overmark_core::settings::Widths;
use overmark_core::tool::ToolKind;
use overmark_core::tr;
use overmark_win::overlay::Command;

use super::icons::Glyph;
use crate::ui::icons::Icon;
use crate::ui::theme;

/// What the toolbar shows.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub backdrop: Backdrop,
    pub guides: Guides,
    pub tool: ToolKind,
    pub colour: Colour,
    pub palette: Vec<Colour>,
    /// The current tool's width, in pixels (none for the eraser and the laser).
    pub width: Option<f32>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub collapsed: bool,
    /// Settings' disappearing ink, which the toolbar turns on and off.
    pub disappearing_ink: bool,
    /// Settings' "Right-click undoes": the undo button's tooltip says so.
    pub right_click_undoes: bool,
    /// A box to record is up (chosen, recording or paused): Record is greyed out until it
    /// goes.
    pub recording: bool,
}

/// What a control asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Tool(ToolKind),
    /// The palette's colour (0-based), as keys 1 to 9.
    Colour(usize),
    Undo,
    Redo,
    Clear,
    Close,
    Capture(Command),
    /// F.
    ToggleFreeze,
    Backdrop(Backdrop),
    Guides(Guides),
    /// Collapse to a pill (true) or expand.
    Collapse(bool),
    /// Disappearing ink on (true) or off: Settings' own setting.
    DisappearingInk(bool),
    /// The current tool's width from the width slider (Settings' pixels), as holding the
    /// tool's key and scrolling sets it.
    Width(f32),
    /// Record a video of a box on the screen: choose the box, then the bar on its edge.
    Record,
}

/// Where the grip was drawn last frame (points), kept in egui's memory under this id: a press
/// there drags the toolbar (the window follows the pointer, which egui can't do from inside).
pub fn grip_id() -> egui::Id {
    egui::Id::unique("overmark-toolbar-grip")
}

/// A key as the toolbar shows it: modifiers from Fluent, in the order given, then the key.
fn keys(modifiers: &[&str], key: &str) -> String {
    modifiers
        .iter()
        .map(|m| tr(m))
        .chain(std::iter::once(key.to_owned()))
        .collect::<Vec<_>>()
        .join("+")
}

fn tool_name(t: ToolKind) -> String {
    tr(&format!("tool-{}", t.as_str()))
}

pub(crate) const BUTTON: f32 = 30.0;
/// Room round the toolbar inside its window: its frame's margins, border and shadow.
const MARGIN: f32 = 40.0;

/// An icon button: `name` is what a screen reader reads, `tip` the hover text with its key.
pub(crate) fn button(
    ui: &mut Ui,
    paint: impl FnOnce(&egui::Painter, egui::Rect, Color32),
    name: &str,
    tip: &str,
) -> Response {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            Color32::TRANSPARENT
        } else if response.is_pointer_button_down_on() {
            p.subtle_selected
        } else if response.hovered() {
            p.subtle_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), fill);
        let colour = if enabled {
            p.text
        } else {
            p.text_secondary.gamma_multiply(0.6)
        };
        paint(ui.painter(), rect.shrink(7.0), colour);
    }
    response.on_hover_text(tip).on_disabled_hover_text(tip)
}

/// An icon button that's on or off (a screen reader says which), filled in the accent when on.
fn toggle(
    ui: &mut Ui,
    paint: impl FnOnce(&egui::Painter, egui::Rect, Color32),
    name: &str,
    tip: &str,
    on: bool,
) -> Response {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, name));
    if ui.is_rect_visible(rect) {
        let fill = if on {
            p.accent
        } else if response.hovered() {
            p.subtle_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), fill);
        paint(
            ui.painter(),
            rect.shrink(7.0),
            if on { p.on_accent } else { p.text },
        );
    }
    response.on_hover_text(tip.to_owned())
}

/// The freeze button's colours in one state: its icon, its fill (resting, hovered, pressed) and
/// its border.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FreezeLook {
    icon: Color32,
    fill: Color32,
    hover: Color32,
    pressed: Color32,
    border: Color32,
}

/// The freeze button's look on `backdrop`: a filled button, unlike the
/// flat controls round it. An amber pause in Live and a green play while Frozen, the same in
/// both themes; on the boards a grey one with a greyed icon. The amber is a yellow amber (hue
/// about 44°), well clear of the copper accent (about 21° light, 25° dark) that fills an "on"
/// toggle and rings the chosen choice, and it always has its darker border; the green is the
/// toolbar's only green (Live's chip is red, Frozen's blue).
fn freeze_look(backdrop: Backdrop, dark: bool) -> FreezeLook {
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    match backdrop {
        Backdrop::Live => FreezeLook {
            icon: rgb(0x1B1B1B),
            fill: rgb(0xF5B700),
            hover: rgb(0xE3A900),
            pressed: rgb(0xCC9800),
            border: rgb(0x9E7600),
        },
        Backdrop::Frozen => FreezeLook {
            icon: Color32::WHITE,
            fill: rgb(0x1E8E3E),
            hover: rgb(0x1A7F37),
            pressed: rgb(0x166E30),
            border: rgb(0x0F5124),
        },
        Backdrop::Whiteboard | Backdrop::Blackboard => {
            let p = if dark { &theme::DARK } else { &theme::LIGHT };
            FreezeLook {
                icon: p.text_secondary.gamma_multiply(0.6),
                fill: p.neutral_fill,
                hover: p.neutral_fill,
                pressed: p.neutral_fill,
                border: p.border,
            }
        }
    }
}

/// Freeze's button (pause in Live, play while Frozen): the size of an icon button, filled in
/// its colour ([`freeze_look`]), darker hovered and darker again pressed; greyed out on the
/// boards. The icon and the tooltip say what it does, not the colour alone.
fn freeze_button(ui: &mut Ui, g: Glyph, name: &str, tip: &str, backdrop: Backdrop) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));
    if ui.is_rect_visible(rect) {
        let look = freeze_look(backdrop, ui.visuals().dark_mode);
        let fill = if !enabled {
            look.fill
        } else if response.is_pointer_button_down_on() {
            look.pressed
        } else if response.hovered() {
            look.hover
        } else {
            look.fill
        };
        // A little inside the icon buttons' square, so it reads as a button on the bar.
        let body = rect.shrink(1.0);
        let radius = CornerRadius::same(theme::CONTROL_RADIUS + 1);
        let painter = ui.painter();
        painter.rect_filled(body, radius, fill);
        painter.rect_stroke(
            body,
            radius,
            Stroke::new(1.0, look.border),
            egui::StrokeKind::Inside,
        );
        g.paint(painter, rect.shrink(8.0), look.icon);
    }
    response.on_hover_text(tip).on_disabled_hover_text(tip)
}

pub(crate) fn glyph(g: Glyph) -> impl FnOnce(&egui::Painter, egui::Rect, Color32) {
    move |p, r, c| g.paint(p, r, c)
}

pub(crate) fn icon(i: Icon) -> impl FnOnce(&egui::Painter, egui::Rect, Color32) {
    move |p, r, c| i.paint(p, r, c)
}

pub(crate) fn separator(ui: &mut Ui) {
    let p = theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, BUTTON - 8.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border);
}

fn colour32(c: Colour) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// The toolbar's frame (the shared look): the card colour, its border and a soft shadow; a pill
/// when `round`.
pub(crate) fn panel_frame(p: &theme::Palette, round: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(if round { u8::MAX } else { 8 }))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(48),
        })
}

/// The toolbar, at the top centre of `ctx`'s screen (its window), and the strip of the group
/// that's open, just below it. Returns what was asked for.
pub fn show(ctx: &egui::Context, state: &State) -> Vec<Action> {
    let mut actions = Vec::new();
    // Collapsed, or the width gone (the eraser, the laser): no strip.
    if state.collapsed || (open_group(ctx) == Some(Group::Width) && state.width.is_none()) {
        set_open_group(ctx, None);
    }
    let available = ctx.content_rect().width();
    let bar = egui::Area::new(egui::Id::unique("overmark-toolbar"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, 8.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let p = theme::of(ui);
            panel_frame(p, state.collapsed).show(ui, |ui| {
                // As wide as its window allows (a narrow monitor at a high scale has fewer
                // points than the toolbar's row): it wraps onto another row rather than losing
                // its ends.
                ui.set_max_width((available - MARGIN).max(BUTTON * 4.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::space::TOOLBAR_GAP;
                    if state.collapsed {
                        collapsed(ui, state, &mut actions);
                    } else {
                        expanded(ui, state, &mut actions);
                    }
                });
            });
        })
        .response
        .rect;
    if let Some(group) = open_group(ctx) {
        strip(ctx, state, group, bar, &mut actions);
    }
    actions
}

fn grip(ui: &mut Ui) {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, BUTTON), Sense::drag());
    let name = tr("toolbar-move");
    response.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, &name));
    Glyph::Grip.paint(ui.painter(), rect.shrink2(vec2(0.0, 7.0)), p.text_secondary);
    ui.ctx().data_mut(|d| d.insert_temp(grip_id(), rect));
    response.on_hover_text(tr("toolbar-move-tooltip"));
}

/// The mode on the collapsed pill: the backdrop's icon on its chip, as on the backdrop's
/// button, its name what a screen reader reads, and what it means in its tooltip.
fn mode(ui: &mut Ui, state: &State) {
    let p = theme::of(ui);
    let name = state.backdrop.name();
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &name));
    chip(
        ui.painter(),
        rect,
        Glyph::backdrop(state.backdrop),
        backdrop_chip(ui, state.backdrop),
        p.control_border,
    );
    response.on_hover_text(mode_tip(state.backdrop));
}

/// What the backdrop means: Live's and Frozen's warning, with F; the boards' names.
fn mode_tip(b: Backdrop) -> String {
    match b {
        Backdrop::Live => tr!("mode-live-tooltip", key = "F"),
        Backdrop::Frozen => tr!("mode-frozen-tooltip", key = "F"),
        b => b.name(),
    }
}

fn collapsed(ui: &mut Ui, state: &State, actions: &mut Vec<Action>) {
    grip(ui);
    mode(ui, state);
    if button(
        ui,
        glyph(Glyph::ChevronDown),
        &tr("toolbar-expand"),
        &tr("toolbar-expand-tooltip"),
    )
    .clicked()
    {
        actions.push(Action::Collapse(false));
    }
}

fn expanded(ui: &mut Ui, state: &State, actions: &mut Vec<Action>) {
    grip(ui);
    // The backdrop's own icon and colour stand for the mode:
    // Frozen is a blue snowflake, where it was a badge beside the grip.
    let backdrop_tip = tr!(
        "toolbar-backdrop-tooltip",
        next = "W",
        previous = keys(&["key-shift"], "W"),
        freeze = "F"
    );
    let now = mode_tip(state.backdrop);
    group_button(
        ui,
        Group::Backdrop,
        &tr("toolbar-backdrop"),
        &format!("{now}\n{backdrop_tip}"),
        Some(&state.backdrop.name()),
        Face::Icon(
            Glyph::backdrop(state.backdrop),
            Some(backdrop_chip(ui, state.backdrop)),
        ),
    );
    let guides_tip = tr!(
        "toolbar-guides-tooltip",
        next = "G",
        previous = keys(&["key-shift"], "G")
    );
    let guides = state.guides.name();
    group_button(
        ui,
        Group::Guides,
        &tr("toolbar-guides"),
        &format!("{guides}\n{guides_tip}"),
        Some(&guides),
        Face::Icon(Glyph::guides(state.guides), None),
    );
    separator(ui);
    let tool = tool_name(state.tool);
    group_button(
        ui,
        Group::Tool,
        &tr("toolbar-tool"),
        &format!("{tool}\n{}", tr("toolbar-tool-tooltip")),
        Some(&tool),
        Face::Icon(Glyph::tool(state.tool), None),
    );
    let open = open_group(ui.ctx()) == Some(Group::Colour);
    let colour = swatch(
        ui,
        state.colour,
        &tr("toolbar-colour"),
        &tr("toolbar-colour-tooltip"),
        open,
    );
    opens(ui, Group::Colour, &colour);
    // The eraser and the laser have no width: its button stays, greyed out, so the toolbar
    // keeps its size.
    let widest = widest_text(ui, width_texts());
    match state.width {
        Some(w) => {
            let text = width_text(w);
            group_button(
                ui,
                Group::Width,
                &format!("{}: {text}", tr("toolbar-width")),
                &tr("toolbar-width-tooltip"),
                Some(&text),
                Face::Text(widest),
            );
        }
        None => {
            group_button(
                ui,
                Group::Width,
                &tr("toolbar-width"),
                &tr("toolbar-width-none-tooltip"),
                None,
                Face::Text(widest),
            );
        }
    }
    let fade = toggle(
        ui,
        glyph(Glyph::Fade),
        &tr("toolbar-disappearing-ink"),
        &tr("toolbar-disappearing-ink-tooltip"),
        state.disappearing_ink,
    );
    if fade.clicked() {
        actions.push(Action::DisappearingInk(!state.disappearing_ink));
    }
    separator(ui);
    let ctrl = |k: &str| keys(&["key-ctrl"], k);
    ui.add_enabled_ui(state.can_undo, |ui| {
        if button(
            ui,
            glyph(Glyph::Undo),
            &tr("toolbar-undo"),
            &if state.right_click_undoes {
                tr!("toolbar-undo-right-click-tooltip", key = ctrl("Z"))
            } else {
                tr!("toolbar-undo-tooltip", key = ctrl("Z"))
            },
        )
        .clicked()
        {
            actions.push(Action::Undo);
        }
    });
    ui.add_enabled_ui(state.can_redo, |ui| {
        let redo_keys = format!("{}, {}", ctrl("Y"), keys(&["key-ctrl", "key-shift"], "Z"));
        if button(
            ui,
            glyph(Glyph::Redo),
            &tr("toolbar-redo"),
            &tr!("toolbar-redo-tooltip", key = redo_keys),
        )
        .clicked()
        {
            actions.push(Action::Redo);
        }
    });
    if button(
        ui,
        icon(Icon::Delete),
        &tr("toolbar-clear"),
        &tr!("toolbar-clear-tooltip", key = tr("key-delete")),
    )
    .clicked()
    {
        actions.push(Action::Clear);
    }
    separator(ui);
    if button(
        ui,
        glyph(Glyph::Save),
        &tr("toolbar-save"),
        &tr!("toolbar-save-tooltip", key = ctrl("S")),
    )
    .clicked()
    {
        actions.push(Action::Capture(Command::Save));
    }
    if button(
        ui,
        glyph(Glyph::Region),
        &tr("toolbar-save-region"),
        &tr!(
            "toolbar-save-region-tooltip",
            key = keys(&["key-ctrl", "key-shift"], "S")
        ),
    )
    .clicked()
    {
        actions.push(Action::Capture(Command::SaveRegion));
    }
    if button(
        ui,
        icon(Icon::Copy),
        &tr("toolbar-copy"),
        &tr!("toolbar-copy-tooltip", key = ctrl("C")),
    )
    .clicked()
    {
        actions.push(Action::Capture(Command::Copy));
    }
    more_menu(ui, actions);
    ui.add_enabled_ui(!state.recording, |ui| {
        let tip = if state.recording {
            tr("toolbar-record-busy-tooltip")
        } else {
            tr("toolbar-record-tooltip")
        };
        if button(ui, glyph(Glyph::Record), &tr("toolbar-record"), &tip).clicked() {
            actions.push(Action::Record);
        }
    });
    // Freeze as a media control: pause in Live, play while
    // frozen, greyed out on the boards; the snowflake is the Frozen backdrop's alone.
    let (g, name, tip) = match state.backdrop {
        Backdrop::Frozen => (
            Glyph::Play,
            tr("toolbar-unfreeze"),
            tr!("toolbar-unfreeze-tooltip", key = "F"),
        ),
        Backdrop::Live => (
            Glyph::Pause,
            tr("toolbar-freeze"),
            tr!("toolbar-freeze-tooltip", key = "F"),
        ),
        Backdrop::Whiteboard | Backdrop::Blackboard => (
            Glyph::Pause,
            tr("toolbar-freeze"),
            tr!("toolbar-freeze-boards-tooltip", key = "F"),
        ),
    };
    let live = matches!(state.backdrop, Backdrop::Live | Backdrop::Frozen);
    ui.add_enabled_ui(live, |ui| {
        if freeze_button(ui, g, &name, &tip, state.backdrop).clicked() {
            actions.push(Action::ToggleFreeze);
        }
    });
    separator(ui);
    if button(
        ui,
        glyph(Glyph::ChevronUp),
        &tr("toolbar-collapse"),
        &tr("toolbar-collapse-tooltip"),
    )
    .clicked()
    {
        actions.push(Action::Collapse(true));
    }
    if button(
        ui,
        icon(Icon::Cross),
        &tr("toolbar-close"),
        &tr!("toolbar-close-tooltip", key = tr("key-esc")),
    )
    .clicked()
    {
        actions.push(Action::Close);
    }
}

/// "More": Save without the drawing (Ctrl+Shift+Alt+S).
fn more_menu(ui: &mut Ui, actions: &mut Vec<Action>) {
    let name = tr("toolbar-more");
    let response = button(ui, icon(Icon::More), &name, &tr("toolbar-more-tooltip"));
    egui::Popup::menu(&response).show(|ui| {
        let key = keys(&["key-ctrl", "key-shift", "key-alt"], "S");
        let item = ui
            .button(tr("toolbar-save-without-drawing"))
            .on_hover_text(tr!("toolbar-save-without-drawing-tooltip", key = key));
        if item.clicked() {
            actions.push(Action::Capture(Command::SaveWithoutDrawing));
            ui.close();
        }
    });
}

fn swatch(ui: &mut Ui, colour: Colour, name: &str, tip: &str, selected: bool) -> Response {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.circle_filled(rect.center(), 9.0, colour32(colour));
        painter.circle_stroke(rect.center(), 9.0, Stroke::new(1.0, p.control_border));
        if selected || response.hovered() {
            painter.circle_stroke(rect.center(), 11.5, Stroke::new(1.5, p.text));
        }
    }
    response.on_hover_text(tip.to_owned())
}

/// The `i`th palette colour's name and tooltip: its own name while it's the default palette's
/// ("Red", "Custom colour" for the ninth), "Colour 4" once Settings changed it; the tooltip
/// gives its key.
fn colour_name(i: usize, c: Colour) -> (String, String) {
    const NAMES: [&str; 9] = [
        "colour-red",
        "colour-amber",
        "colour-yellow",
        "colour-green",
        "colour-blue",
        "colour-purple",
        "colour-black",
        "colour-white",
        "colour-custom",
    ];
    let key = (i + 1).to_string();
    let default = overmark_core::settings::DEFAULT_PALETTE.get(i) == Some(&c);
    match NAMES.get(i) {
        Some(name) if default || i == 8 => {
            let name = tr(name);
            let tip = tr!("colour-tooltip", colour = name.clone(), key = key);
            (name, tip)
        }
        _ => (
            tr!("toolbar-colour-n", n = key.clone()),
            tr!("toolbar-colour-n-tooltip", n = key),
        ),
    }
}

/// A group of choices on the toolbar: its button opens its strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Backdrop,
    Guides,
    Tool,
    Colour,
    Width,
}

/// Which group's strip is open, in egui's memory (none when the layer opens: the toolbar
/// closes it as it closes).
fn strip_id() -> egui::Id {
    egui::Id::unique("overmark-toolbar-strip")
}

/// Where the open group's button starts (points), for the strip to start under it.
fn anchor_id() -> egui::Id {
    egui::Id::unique("overmark-toolbar-strip-anchor")
}

/// The width's strip was just opened: its slider takes the focus, so the arrows move it.
fn focus_id() -> egui::Id {
    egui::Id::unique("overmark-toolbar-strip-focus")
}

/// The group whose strip is open, if one is.
pub fn open_group(ctx: &egui::Context) -> Option<Group> {
    ctx.data(|d| d.get_temp::<Option<Group>>(strip_id()))
        .flatten()
}

/// Opens `group`'s strip, or closes the strip (`None`).
pub fn set_open_group(ctx: &egui::Context, group: Option<Group>) {
    ctx.data_mut(|d| d.insert_temp(strip_id(), group));
}

/// `group`'s button was drawn as `response`: a click opens its strip, or closes it if it was
/// open (another group's strip gives way); while open, the strip starts under it.
fn opens(ui: &Ui, group: Group, response: &Response) {
    let ctx = ui.ctx();
    if response.clicked() {
        let open = open_group(ctx) != Some(group);
        set_open_group(ctx, open.then_some(group));
        if open && group == Group::Width {
            ctx.data_mut(|d| d.insert_temp(focus_id(), true));
        }
    }
    if open_group(ctx) == Some(group) {
        ctx.data_mut(|d| d.insert_temp(anchor_id(), response.rect.left()));
    }
}

/// What a group's button shows besides its chevron.
enum Face {
    /// The current choice's icon; on a chip of its own colours (`(icon, fill)`) where it has
    /// them (the backdrop's).
    Icon(Glyph, Option<(Color32, Color32)>),
    /// The current choice as text, in a button as wide as the widest it can show (from
    /// [`widest_text`]).
    Text(f32),
}

/// The widest of `texts` in the toolbar's font (its current language and scale), in points.
fn widest_text(ui: &Ui, texts: impl IntoIterator<Item = String>) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    texts
        .into_iter()
        .map(|t| {
            ui.painter()
                .layout_no_wrap(t, font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max)
}

/// The backdrop's colours, on the toolbar's button and in its strip (`(icon, fill)`).
fn backdrop_chip(ui: &Ui, b: Backdrop) -> (Color32, Color32) {
    backdrop_colours(b, ui.visuals().dark_mode)
}

/// Live red (on air), Frozen icy blue, the whiteboard white and the blackboard black, in the light
/// theme or the dark.
fn backdrop_colours(b: Backdrop, dark: bool) -> (Color32, Color32) {
    let p = if dark { &theme::DARK } else { &theme::LIGHT };
    let rgb = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    match b {
        Backdrop::Live => (p.failed, p.failed_fill),
        Backdrop::Frozen if dark => (rgb(0x8EC8F6), rgb(0x1E2E3D)),
        Backdrop::Frozen => (rgb(0x0B5C9E), rgb(0xE1EFFA)),
        Backdrop::Whiteboard => (rgb(0x1F1F1F), Color32::WHITE),
        Backdrop::Blackboard => (rgb(0xF3F3F3), rgb(0x1C1C1C)),
    }
}

/// `g` on a chip of its colours (`(icon, fill)`), in `rect` (a button's icon square).
fn chip(
    painter: &egui::Painter,
    rect: egui::Rect,
    g: Glyph,
    (icon, fill): (Color32, Color32),
    border: Color32,
) {
    let chip = rect.shrink(3.0);
    let radius = CornerRadius::same(4);
    painter.rect_filled(chip, radius, fill);
    painter.rect_stroke(
        chip,
        radius,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    g.paint(painter, rect.shrink(8.0), icon);
}

/// A group's button, named `name` (with `tip`), showing its current choice `value` (as its
/// `face` says) and a small chevron; pressed while its strip is open (a screen reader hears it
/// on or off, and its value). Its size is the same whichever choice it shows, so choosing
/// another never resizes the toolbar. With no `value` (the width, for a tool without
/// one) it is greyed out and opens nothing.
fn group_button(
    ui: &mut Ui,
    group: Group,
    name: &str,
    tip: &str,
    value: Option<&str>,
    face: Face,
) -> Response {
    let p = theme::of(ui);
    let enabled = value.is_some();
    let open = enabled && open_group(ui.ctx()) == Some(group);
    let size = match face {
        Face::Icon(..) => vec2(BUTTON + 12.0 + 4.0, BUTTON),
        Face::Text(widest) => vec2(widest + 8.0 + 4.0 + 12.0 + 6.0, BUTTON),
    };
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, open, name));
    if let Some(value) = value {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_value(value));
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if !enabled {
            Color32::TRANSPARENT
        } else if open || response.is_pointer_button_down_on() {
            p.subtle_selected
        } else if response.hovered() {
            p.subtle_hover
        } else {
            Color32::TRANSPARENT
        };
        let radius = CornerRadius::same(theme::CONTROL_RADIUS);
        painter.rect_filled(rect, radius, fill);
        let border = if enabled { p.control_border } else { p.border };
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        let icon_at = egui::Rect::from_min_size(rect.min, vec2(BUTTON, BUTTON));
        match face {
            Face::Icon(g, Some(colours)) => chip(painter, icon_at, g, colours, p.control_border),
            Face::Icon(g, None) => g.paint(painter, icon_at.shrink(7.0), p.text),
            Face::Text(_) => {
                if let Some(value) = value {
                    let galley = painter.layout_no_wrap(
                        value.to_owned(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        p.text,
                    );
                    let at = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
                    painter.galley(at, galley, p.text);
                }
            }
        }
        let chevron = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 6.0 - 6.0, rect.center().y),
            vec2(12.0, 12.0),
        );
        let g = if open {
            Glyph::ChevronUp
        } else {
            Glyph::ChevronDown
        };
        let colour = if enabled {
            p.text_secondary
        } else {
            p.text_secondary.gamma_multiply(0.4)
        };
        g.paint(painter, chevron, colour);
    }
    if enabled {
        opens(ui, group, &response);
    }
    response.on_hover_text(tip.to_owned())
}

/// `group`'s strip: its choices in a row of the toolbar's look, just below the toolbar (`bar`)
/// and starting under the group's button, kept inside the window and wrapping onto another
/// row where it's narrow; an up arrow at its far right closes it. It stays open while you
/// draw and choose, so several can be tried.
fn strip(
    ctx: &egui::Context,
    state: &State,
    group: Group,
    bar: egui::Rect,
    actions: &mut Vec<Action>,
) {
    let available = ctx.content_rect().width();
    let left = ctx
        .data(|d| d.get_temp::<f32>(anchor_id()))
        .unwrap_or(bar.left());
    let focus = ctx
        .data_mut(|d| d.remove_temp::<bool>(focus_id()))
        .unwrap_or(false);
    egui::Area::new(strip_id().with(group))
        .fixed_pos(egui::pos2(left - 8.0, bar.bottom() + 4.0))
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            let p = theme::of(ui);
            panel_frame(p, false).show(ui, |ui| {
                ui.set_max_width((available - MARGIN).max(BUTTON * 4.0));
                ui.horizontal_wrapped(|ui| {
                    // A group a screen reader reads by its name ("Backdrop choices").
                    let id = ui.unique_id();
                    let name = tr!("toolbar-strip", group = group_name(group));
                    ui.ctx().accesskit_node_builder(id, |node| {
                        node.set_role(Role::Group);
                        node.set_label(name);
                    });
                    ui.spacing_mut().item_spacing.x = theme::space::TOOLBAR_GAP;
                    choices(ui, state, group, focus, actions);
                    separator(ui);
                    if button(
                        ui,
                        glyph(Glyph::ChevronUp),
                        &tr("toolbar-strip-close"),
                        &tr("toolbar-strip-close-tooltip"),
                    )
                    .clicked()
                    {
                        set_open_group(ui.ctx(), None);
                    }
                });
            });
        });
}

fn group_name(group: Group) -> String {
    tr(match group {
        Group::Backdrop => "toolbar-backdrop",
        Group::Guides => "toolbar-guides",
        Group::Tool => "toolbar-tool",
        Group::Colour => "toolbar-colour",
        Group::Width => "toolbar-width",
    })
}

/// A group's choices, each through the same action as its key; the chosen one shown on.
fn choices(ui: &mut Ui, state: &State, group: Group, focus: bool, actions: &mut Vec<Action>) {
    match group {
        Group::Backdrop => {
            for b in Backdrop::ALL {
                let tip = match b {
                    Backdrop::Live | Backdrop::Frozen => format!("{} (F)", b.name()),
                    _ => b.name(),
                };
                let on = b == state.backdrop;
                if backdrop_choice(ui, b, &tip, on).clicked() && !on {
                    actions.push(Action::Backdrop(b));
                }
            }
        }
        Group::Guides => {
            for g in Guides::ALL {
                let on = g == state.guides;
                let name = g.name();
                if toggle(ui, glyph(Glyph::guides(g)), &name, &name, on).clicked() && !on {
                    actions.push(Action::Guides(g));
                }
            }
        }
        Group::Tool => {
            for t in ToolKind::ALL {
                let tip = tr!(
                    &format!("tool-{}-tooltip", t.as_str()),
                    key = t.key().to_string()
                );
                let on = t == state.tool;
                if toggle(ui, glyph(Glyph::tool(t)), &tool_name(t), &tip, on).clicked() && !on {
                    actions.push(Action::Tool(t));
                }
            }
        }
        Group::Colour => {
            for (i, c) in state.palette.iter().enumerate() {
                let (label, tip) = colour_name(i, *c);
                if swatch(ui, *c, &label, &tip, *c == state.colour).clicked() {
                    actions.push(Action::Colour(i));
                }
            }
        }
        Group::Width => {
            if let Some(w) = state.width {
                let changed = width_slider(ui, w, &tr("toolbar-width"), focus);
                if let Some(new) = changed {
                    actions.push(Action::Width(new));
                }
                ui.label(width_text(changed.unwrap_or(w)));
            }
        }
    }
}

/// A backdrop in its strip: its icon on its chip, as on the toolbar's button; the chosen one
/// ringed in the accent (a screen reader hears it on).
fn backdrop_choice(ui: &mut Ui, b: Backdrop, tip: &str, on: bool) -> Response {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    let name = b.name();
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, &name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(theme::CONTROL_RADIUS);
        if response.hovered() && !on {
            painter.rect_filled(rect, radius, p.subtle_hover);
        }
        chip(
            painter,
            rect,
            Glyph::backdrop(b),
            backdrop_chip(ui, b),
            p.control_border,
        );
        if on {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    response.on_hover_text(tip.to_owned())
}

/// A width as the toolbar shows it ("4 px").
fn width_text(w: f32) -> String {
    tr!(
        "toolbar-width-value",
        width = format!("{}", w.round() as i64)
    )
}

/// Every width the readout can show ("1 px" to "200 px", [`Widths::RANGE`]), for its button to
/// fit the widest: a font's digits needn't all be as wide.
fn width_texts() -> impl Iterator<Item = String> {
    let (lo, hi) = Widths::RANGE;
    (lo.round() as i64..=hi.round() as i64).map(|w| width_text(w as f32))
}

/// The slider's length, in points.
const SLIDER: f32 = 160.0;

/// A horizontal slider for `width` over [`Widths::RANGE`], spaced as the wheel steps it (each
/// step the same factor, so the thin widths have room): dragging sets a whole number of
/// pixels; an arrow key, or a screen reader's increment, is one notch of the wheel
/// ([`Widths::stepped`]). It takes the focus as it opens (`focus`), so the arrows move it at
/// once. Returns the new width if it changed.
fn width_slider(ui: &mut Ui, width: f32, name: &str, focus: bool) -> Option<f32> {
    use egui::accesskit::{Action as A, ActionData};
    let p = theme::of(ui);
    let (lo, hi) = Widths::RANGE;
    let (rect, response) = ui.allocate_exact_size(vec2(SLIDER, 22.0), Sense::click_and_drag());
    if focus || ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
    let rail = rect.shrink2(vec2(8.0, 0.0));
    let span = (hi / lo).ln();
    let x_of = |w: f32| rail.left() + rail.width() * ((w / lo).ln() / span).clamp(0.0, 1.0);
    let mut new = None;
    if response.is_pointer_button_down_on()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
        new = Some(Widths::stepped((lo * (span * t).exp()).round(), 0.0));
        response.request_focus();
    }
    let mut steps = 0.0;
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        ui.input(|i| {
            steps +=
                (i.num_presses(egui::Key::ArrowRight) + i.num_presses(egui::Key::ArrowUp)) as f32;
            steps -=
                (i.num_presses(egui::Key::ArrowLeft) + i.num_presses(egui::Key::ArrowDown)) as f32;
        });
    }
    ui.input(|i| {
        steps += i.num_accesskit_action_requests(response.id, A::Increment) as f32;
        steps -= i.num_accesskit_action_requests(response.id, A::Decrement) as f32;
        for r in i.accesskit_action_requests(response.id, A::SetValue) {
            if let Some(ActionData::NumericValue(v)) = r.data {
                new = Some(Widths::stepped(v as f32, 0.0));
            }
        }
    });
    if steps != 0.0 {
        new = Some(Widths::stepped(new.unwrap_or(width), steps));
    }
    let value = new.unwrap_or(width);
    response.widget_info(|| WidgetInfo::slider(true, f64::from(value.round()), name));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_min_numeric_value(f64::from(lo));
        node.set_max_numeric_value(f64::from(hi));
        node.add_action(A::SetValue);
        if value < hi {
            node.add_action(A::Increment);
        }
        if value > lo {
            node.add_action(A::Decrement);
        }
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let y = rect.center().y;
        let x = x_of(value);
        let track = |a: f32, b: f32| {
            egui::Rect::from_min_max(egui::pos2(a, y - 2.0), egui::pos2(b, y + 2.0))
        };
        let round = CornerRadius::same(2);
        painter.rect_filled(track(rail.left(), rail.right()), round, p.border);
        painter.rect_filled(track(rail.left(), x), round, p.accent);
        let handle = egui::pos2(x, y);
        if response.has_focus() {
            painter.circle_stroke(handle, 10.0, Stroke::new(2.0, p.accent));
        }
        let fill = if response.hovered() || response.dragged() {
            p.accent
        } else {
            p.card
        };
        painter.circle_filled(handle, 7.0, fill);
        painter.circle_stroke(handle, 7.0, Stroke::new(2.0, p.accent));
    }
    new.filter(|n| *n != width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use overmark_core::settings::DEFAULT_PALETTE;

    fn state() -> State {
        State {
            backdrop: Backdrop::Live,
            guides: Guides::Off,
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
        }
    }

    fn harness(state: State) -> Harness<'static, (State, Vec<Action>)> {
        overmark_core::i18n::init("en");
        let mut h = Harness::builder()
            .with_size(vec2(1100.0, 400.0))
            .with_context_setup(|ctx| crate::ui::install(ctx).expect("Windows has a text font"))
            .build_ui_state(
                |ui, (state, actions): &mut (State, Vec<Action>)| {
                    let new = show(ui.ctx(), state);
                    // The width follows, as the app takes it up.
                    for a in &new {
                        if let Action::Width(w) = a {
                            state.width = Some(*w);
                        }
                    }
                    actions.extend(new);
                },
                (state, Vec::new()),
            );
        h.run();
        h
    }

    fn widths(h: &Harness<'static, (State, Vec<Action>)>) -> Vec<f32> {
        h.state()
            .1
            .iter()
            .filter_map(|a| match a {
                Action::Width(w) => Some(*w),
                _ => None,
            })
            .collect()
    }

    fn slider_open(h: &Harness<'static, (State, Vec<Action>)>) -> bool {
        h.query_by_role(Role::Slider).is_some()
    }

    #[test]
    fn clicking_the_width_opens_its_slider_just_below_and_again_closes_it() {
        let mut h = harness(state());
        assert!(!slider_open(&h));
        let readout = h.get_by_label("Width: 4 px").rect();
        // A click of the pointer, as the toolbar's window gives it.
        h.get_by_label("Width: 4 px").click();
        h.run();
        let slider = h.get_by_role(Role::Slider);
        assert_eq!(slider.accesskit_node().label().as_deref(), Some("Width"));
        assert_eq!(slider.accesskit_node().numeric_value(), Some(4.0));
        assert!(slider.is_focused(), "the arrows move it at once");
        let r = slider.rect();
        assert!(
            r.top() >= readout.bottom() && r.top() - readout.bottom() < 24.0,
            "just below: the slider at {r:?}, the readout at {readout:?}"
        );
        assert!(
            (r.left() - readout.left()).abs() < 24.0,
            "{r:?} {readout:?}"
        );
        assert!(pressed(&h, "Width: 4 px"), "its button looks pressed");
        h.get_by_label("Width: 4 px").click();
        h.run();
        assert!(!slider_open(&h));
        assert!(widths(&h).is_empty(), "opening and closing changes nothing");
        // A screen reader's press opens it too.
        h.get_by_label("Width: 4 px").click_accesskit();
        h.run();
        assert!(slider_open(&h));
    }

    #[test]
    fn the_slider_steps_as_the_wheel_does_and_drags_over_its_range() {
        let (lo, hi) = Widths::RANGE;
        let mut h = harness(state());
        h.get_by_label("Width: 4 px").click();
        h.run();
        // An arrow is one notch of the wheel.
        h.key_press(egui::Key::ArrowRight);
        h.run();
        assert_eq!(widths(&h), vec![Widths::stepped(4.0, 1.0)]);
        h.key_press(egui::Key::ArrowLeft);
        h.key_press(egui::Key::ArrowLeft);
        h.run();
        let w = h.state().0.width.unwrap();
        assert!((w - Widths::stepped(4.0, -1.0)).abs() < 1e-4, "{w}");
        // A screen reader's increment and set value.
        h.state_mut().1.clear();
        h.get_by_role(Role::Slider).increment_accesskit();
        h.run();
        assert_eq!(widths(&h), vec![Widths::stepped(w, 1.0)]);
        h.get_by_role(Role::Slider).set_value_accesskit(1000.0);
        h.run();
        assert_eq!(h.state().0.width, Some(hi));
        assert_eq!(
            h.get_by_role(Role::Slider).accesskit_node().numeric_value(),
            Some(f64::from(hi))
        );
        // The value shows beside it, and on the readout.
        assert!(h.query_by_label("200 px").is_some());
        assert!(h.query_by_label("Width: 200 px").is_some());
        // Dragged to its left end: the thinnest; to the middle, a whole number of pixels.
        let r = h.get_by_role(Role::Slider).rect();
        h.drag_at(r.left_center());
        h.run();
        h.drop_at(r.left_center());
        h.run();
        assert_eq!(h.state().0.width, Some(lo));
        h.drag_at(r.center());
        h.run();
        h.drop_at(r.center());
        h.run();
        let mid = h.state().0.width.unwrap();
        assert!(mid > lo && mid < hi && mid.fract() == 0.0, "{mid}");
        // Still open: dragging on it doesn't close it.
        assert!(slider_open(&h));
    }

    /// Whether `group`'s strip is open ("Tool" for the Tool choices).
    fn strip_open(h: &Harness<'static, (State, Vec<Action>)>, group: &str) -> bool {
        h.query_by_label(&format!("{group} choices")).is_some()
    }

    /// Whether the button named `name` is on (pressed, its strip open).
    fn pressed(h: &Harness<'static, (State, Vec<Action>)>, name: &str) -> bool {
        h.get_by_label(name).accesskit_node().data().toggled()
            == Some(egui::accesskit::Toggled::True)
    }

    #[test]
    fn each_group_opens_its_strip_just_below_and_another_switches_it() {
        let mut h = harness(state());
        let mut last: Option<&str> = None;
        for (button, group, choice) in [
            ("Backdrop", "Backdrop", "Live"),
            ("Guides", "Guides", "No guides"),
            ("Tool", "Tool", "Pen"),
            ("Colour", "Colour", "Red"),
            ("Width: 4 px", "Width", "Width"),
        ] {
            let below = h.get_by_label("Undo").rect().bottom();
            h.get_by_label(button).click();
            h.run();
            assert!(strip_open(&h, group), "{group}");
            assert!(pressed(&h, button), "{button} looks pressed");
            // One strip at a time: the last gives way.
            if let Some(last) = last {
                assert!(!strip_open(&h, last), "{last} closed when {group} opened");
            }
            let at = h.get_by_label(choice).rect();
            let under = h.get_by_label(button).rect();
            assert!(
                at.top() > below && at.top() - below < 30.0,
                "{choice} just below the toolbar: {at:?}, the toolbar ends at {below}"
            );
            assert!(
                (at.left() - under.left()).abs() < 30.0,
                "{group}'s strip starts under its button: {at:?}, the button {under:?}"
            );
            last = Some(group);
        }
        assert!(h.state().1.is_empty(), "opening strips changes nothing");
    }

    #[test]
    fn the_strips_choices_have_names_and_show_the_chosen_one() {
        let mut s = state();
        s.backdrop = Backdrop::Frozen;
        s.guides = Guides::Grid;
        let mut h = harness(s);
        for (button, choices) in [
            ("Backdrop", ["Live", "Whiteboard", "Blackboard"]),
            ("Guides", ["No guides", "Dots", "Ruled lines"]),
            ("Tool", ["Arrow", "Eraser", "Laser pointer"]),
        ] {
            h.get_by_label(button).click_accesskit();
            h.run();
            for c in choices {
                assert!(!pressed(&h, c), "{c} isn't chosen");
            }
        }
        // The chosen one is on.
        h.get_by_label("Tool").click_accesskit();
        h.run();
        h.get_by_label("Tool").click_accesskit();
        h.run();
        assert!(pressed(&h, "Pen"));
        h.get_by_label("Guides").click_accesskit();
        h.run();
        assert!(pressed(&h, "Grid"));
    }

    #[test]
    fn the_strip_stays_open_while_drawing_and_closes_by_its_arrow_or_its_button() {
        let mut h = harness(state());
        h.get_by_label("Tool").click();
        h.run();
        // A pick, a click on the ink (outside the toolbar's pixels) and Esc (the layer's)
        // leave it open: several tools can be tried.
        h.get_by_label("Arrow").click();
        h.run();
        let away = egui::pos2(500.0, 300.0);
        h.drag_at(away);
        h.run();
        h.drop_at(away);
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
        h.get_by_label("Ellipse").click();
        h.run();
        assert!(strip_open(&h, "Tool"));
        assert_eq!(
            h.state().1,
            vec![
                Action::Tool(ToolKind::Arrow),
                Action::Tool(ToolKind::Ellipse)
            ]
        );
        // Its up arrow closes it.
        h.get_by_label("Close the choices").click();
        h.run();
        assert!(!strip_open(&h, "Tool"));
        assert!(!pressed(&h, "Tool"));
        // So does its button, again; a screen reader's press does both.
        h.get_by_label("Tool").click();
        h.run();
        assert!(strip_open(&h, "Tool"));
        h.get_by_label("Tool").click_accesskit();
        h.run();
        assert!(!strip_open(&h, "Tool"));
        h.get_by_label("Tool").click_accesskit();
        h.run();
        h.get_by_label("Close the choices").click_accesskit();
        h.run();
        assert!(!strip_open(&h, "Tool"));
    }

    #[test]
    fn collapsing_closes_the_strip() {
        let mut h = harness(state());
        h.get_by_label("Colour").click();
        h.run();
        assert!(strip_open(&h, "Colour"));
        h.state_mut().0.collapsed = true;
        h.run();
        h.state_mut().0.collapsed = false;
        h.run();
        assert!(!strip_open(&h, "Colour"));
        // And a tool with no width closes the width's.
        h.get_by_label("Width: 4 px").click();
        h.run();
        h.state_mut().0.width = None;
        h.run();
        h.state_mut().0.width = Some(4.0);
        h.run();
        assert!(!strip_open(&h, "Width"));
    }

    #[test]
    fn at_a_very_narrow_width_the_strip_wraps_inside_the_window() {
        // A 1280-pixel monitor at 300 %: 427 points.
        let width = 1280.0 / 3.0;
        overmark_core::i18n::init("en");
        let mut h = Harness::builder()
            .with_size(vec2(width, 360.0))
            .with_context_setup(|ctx| crate::ui::install(ctx).expect("Windows has a text font"))
            .build_ui_state(
                |ui, (state, actions): &mut (State, Vec<Action>)| {
                    actions.extend(show(ui.ctx(), state));
                },
                (state(), Vec::new()),
            );
        h.run();
        h.get_by_label("Tool").click_accesskit();
        h.run();
        let first = h.get_by_label("Pen").rect();
        let close = h.get_by_label("Close the choices").rect();
        for t in ToolKind::ALL {
            let r = h.get_by_label(&tool_name(t)).rect();
            assert!(
                r.min.x >= 0.0 && r.max.x <= width && r.max.y <= 360.0,
                "{t:?} at {r:?}, the window {width} wide"
            );
        }
        assert!(close.min.x >= 0.0 && close.max.x <= width, "{close:?}");
        assert!(
            close.top() > first.bottom(),
            "it wraps: {close:?} below {first:?}"
        );
    }

    #[test]
    fn the_eraser_and_the_laser_have_no_width() {
        let mut s = state();
        s.tool = ToolKind::Eraser;
        s.width = None;
        let mut h = harness(s);
        assert!(h.query_by_label_contains("px").is_none());
        assert!(h.query_by_role(Role::Slider).is_none());
        // Its button stays, greyed out, and opens nothing.
        let width = h.get_by_label("Width");
        assert!(width.accesskit_node().is_disabled());
        assert_eq!(width.accesskit_node().value(), None);
        h.get_by_label("Width").click();
        h.run();
        assert!(!strip_open(&h, "Width"));
        assert!(h.state().1.is_empty());
    }

    #[test]
    fn the_width_says_both_ways_to_change_it() {
        overmark_core::i18n::init("en");
        let tip = tr("toolbar-width-tooltip");
        assert!(tip.contains("click") && tip.contains("scroll"), "{tip}");
    }

    #[test]
    fn every_button_has_its_name_and_does_what_its_key_does() {
        let mut h = harness(state());
        for (name, want) in [
            ("Undo", Action::Undo),
            ("Clear", Action::Clear),
            ("Save", Action::Capture(Command::Save)),
            ("Save a region", Action::Capture(Command::SaveRegion)),
            ("Copy", Action::Capture(Command::Copy)),
            ("Freeze", Action::ToggleFreeze),
            ("Collapse", Action::Collapse(true)),
            ("Close", Action::Close),
        ] {
            h.state_mut().1.clear();
            h.get_by_label(name).click_accesskit();
            h.run();
            assert_eq!(h.state().1, vec![want], "{name}");
        }
    }

    #[test]
    fn redo_is_greyed_out_with_nothing_to_redo() {
        let mut h = harness(state());
        h.get_by_label("Redo").click_accesskit();
        h.run();
        assert!(h.state().1.is_empty());
        let mut s = state();
        s.can_redo = true;
        s.can_undo = false;
        let mut h = harness(s);
        h.get_by_label("Undo").click_accesskit();
        h.get_by_label("Redo").click_accesskit();
        h.run();
        assert_eq!(h.state().1, vec![Action::Redo]);
    }

    #[test]
    fn save_without_the_drawing_is_in_more() {
        let mut h = harness(state());
        h.get_by_label("More").click_accesskit();
        h.run();
        h.get_by_label("Save without the drawing").click_accesskit();
        h.run();
        assert!(
            h.state()
                .1
                .contains(&Action::Capture(Command::SaveWithoutDrawing))
        );
    }

    #[test]
    fn the_strips_choose_a_backdrop_guides_a_tool_and_a_colour() {
        let mut h = harness(state());
        h.get_by_label("Backdrop").click_accesskit();
        h.run();
        h.get_by_label("Whiteboard").click_accesskit();
        h.run();
        h.get_by_label("Guides").click_accesskit();
        h.run();
        h.get_by_label("Dots").click_accesskit();
        h.run();
        h.get_by_label("Tool").click_accesskit();
        h.run();
        h.get_by_label("Rectangle").click_accesskit();
        h.run();
        h.get_by_label("Colour").click_accesskit();
        h.run();
        h.get_by_label("Blue").click_accesskit();
        h.run();
        assert_eq!(
            h.state().1,
            vec![
                Action::Backdrop(Backdrop::Whiteboard),
                Action::Guides(Guides::Dots),
                Action::Tool(ToolKind::Rectangle),
                Action::Colour(4),
            ]
        );
    }

    #[test]
    fn frozen_shows_on_the_backdrops_button_and_unfreeze() {
        let mut s = state();
        s.backdrop = Backdrop::Frozen;
        let h = harness(s);
        assert!(h.query_by_label("Unfreeze").is_some());
        let backdrop = h.get_by_label("Backdrop");
        assert_eq!(backdrop.accesskit_node().value().as_deref(), Some("Frozen"));
        // No badge beside it any more: the button is the mode.
        assert!(h.query_by_label("Frozen").is_none());
    }

    #[test]
    fn freeze_is_pause_in_live_play_while_frozen_and_greyed_on_the_boards() {
        for (backdrop, name, enabled) in [
            (Backdrop::Live, "Freeze", true),
            (Backdrop::Frozen, "Unfreeze", true),
            (Backdrop::Whiteboard, "Freeze", false),
            (Backdrop::Blackboard, "Freeze", false),
        ] {
            let mut s = state();
            s.backdrop = backdrop;
            let mut h = harness(s);
            assert_eq!(
                h.get_by_label(name).accesskit_node().is_disabled(),
                !enabled,
                "{backdrop:?}"
            );
            h.get_by_label(name).click_accesskit();
            h.run();
            let want = if enabled {
                vec![Action::ToggleFreeze]
            } else {
                vec![]
            };
            assert_eq!(h.state().1, want, "{backdrop:?}");
        }
        overmark_core::i18n::init("en");
        assert_eq!(
            tr!("toolbar-freeze-tooltip", key = "F"),
            "Freeze the screen (F)"
        );
        assert_eq!(
            tr!("toolbar-unfreeze-tooltip", key = "F"),
            "Let the screen move again (F)"
        );
        assert!(overmark_core::i18n::has("toolbar-freeze-boards-tooltip"));
    }

    /// A colour's hue in degrees (0 to 360).
    fn hue(c: Color32) -> f32 {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        if d == 0.0 {
            return 0.0;
        }
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h * 60.0
    }

    #[test]
    fn freeze_is_an_amber_pause_and_a_green_play_on_a_button_of_its_own() {
        for dark in [false, true] {
            let p = if dark { &theme::DARK } else { &theme::LIGHT };
            let live = freeze_look(Backdrop::Live, dark);
            let frozen = freeze_look(Backdrop::Frozen, dark);
            let board = freeze_look(Backdrop::Whiteboard, dark);
            assert_eq!(board, freeze_look(Backdrop::Blackboard, dark));
            // Its fill differs in Live, Frozen and on the boards, and is never the flat
            // buttons' none.
            for (a, b) in [(live, frozen), (live, board), (frozen, board)] {
                assert_ne!(a.fill, b.fill, "dark {dark}");
            }
            for look in [live, frozen, board] {
                assert_ne!(look.fill, Color32::TRANSPARENT, "dark {dark}");
            }
            // Amber and green: the icon readable (3:1) on every state of its fill, which darkens
            // hovered and again pressed.
            for (name, look, want) in [("Live", live, 30.0..60.0), ("Frozen", frozen, 100.0..160.0)]
            {
                assert!(
                    want.contains(&hue(look.fill)),
                    "{name}: hue {}",
                    hue(look.fill)
                );
                for fill in [look.fill, look.hover, look.pressed] {
                    let c = theme::contrast(look.icon, fill);
                    assert!(c >= 3.0, "{name} (dark {dark}): {c:.2}:1");
                }
                assert_ne!(look.hover, look.fill);
                assert_ne!(look.pressed, look.hover);
            }
            // The amber is clear of the copper accent (an "on" toggle, the chosen choice's
            // ring), and neither is any backdrop's chip.
            assert!(
                hue(live.fill) - hue(p.accent) >= 15.0,
                "amber {} copper {} (dark {dark})",
                hue(live.fill),
                hue(p.accent)
            );
            for b in Backdrop::ALL {
                let (_, chip) = backdrop_colours(b, dark);
                assert_ne!(chip, live.fill);
                assert_ne!(chip, frozen.fill);
            }
            // On the boards it's grey: no colour.
            assert_eq!(board.fill, p.neutral_fill);
        }
    }

    #[test]
    fn collapsed_is_a_pill_that_expands() {
        let mut s = state();
        s.collapsed = true;
        let mut h = harness(s);
        assert!(h.query_by_label("Undo").is_none());
        // The mode shows on it, as the backdrop's button shows it.
        assert!(h.query_by_label("Live").is_some());
        h.get_by_label("Expand").click_accesskit();
        h.run();
        assert_eq!(h.state().1, vec![Action::Collapse(false)]);
    }

    #[test]
    fn at_a_narrow_scaled_width_every_control_stays_in_the_window() {
        // A 1280-pixel monitor at 150 %: 853 points; the longest names (Frozen, Ruled lines,
        // Highlighter, a 128 px width).
        let mut s = state();
        s.backdrop = Backdrop::Frozen;
        s.guides = Guides::Lines;
        s.tool = ToolKind::Highlighter;
        s.width = Some(128.0);
        let width = 1280.0 / 1.5;
        overmark_core::i18n::init("en");
        let mut h = Harness::builder()
            .with_size(vec2(width, 360.0))
            .with_context_setup(|ctx| crate::ui::install(ctx).expect("Windows has a text font"))
            .build_ui_state(
                |ui, (state, actions): &mut (State, Vec<Action>)| {
                    actions.extend(show(ui.ctx(), state));
                },
                (s, Vec::new()),
            );
        h.run();
        for name in [
            "Move the toolbar",
            "Backdrop",
            "Guides",
            "Tool",
            "Undo",
            "Save",
            "More",
            "Unfreeze",
            "Collapse",
            "Close",
        ] {
            let r = h.get_by_label(name).rect();
            assert!(
                r.min.x >= 0.0 && r.max.x <= width && r.max.y <= 360.0,
                "{name} at {r:?}, the window {width} wide"
            );
        }
        // Each group's strip opens inside the window there too (the width's, from its far
        // right, kept inside).
        for (button, inside) in [
            ("Backdrop", "Blackboard"),
            ("Guides", "Ruled lines"),
            ("Tool", "Laser pointer"),
            ("Colour", "Custom colour"),
            ("Width: 128 px", "128 px"),
        ] {
            h.get_by_label(button).click_accesskit();
            h.run();
            for name in [inside, "Close the choices"] {
                let r = h.get_by_label(name).rect();
                assert!(
                    r.min.x >= 0.0 && r.max.x <= width && r.max.y <= 360.0,
                    "{button}'s {name} at {r:?}, the window {width} wide"
                );
            }
        }
        let r = h.get_by_role(Role::Slider).rect();
        assert!(r.min.x >= 0.0 && r.max.x <= width, "the slider at {r:?}");
        h.get_by_label("Close the choices").click_accesskit();
        h.run();
        // And it still works there.
        h.get_by_label("Close").click_accesskit();
        h.run();
        assert_eq!(h.state().1, vec![Action::Close]);
    }

    /// A harness whose state follows the backdrop, guides, tool and width chosen, as the app's
    /// does.
    fn following(state: State) -> Harness<'static, (State, Vec<Action>)> {
        overmark_core::i18n::init("en");
        let mut h = Harness::builder()
            .with_size(vec2(1100.0, 400.0))
            .with_context_setup(|ctx| crate::ui::install(ctx).expect("Windows has a text font"))
            .build_ui_state(
                |ui, (state, actions): &mut (State, Vec<Action>)| {
                    let new = show(ui.ctx(), state);
                    for a in &new {
                        match *a {
                            Action::Backdrop(b) => state.backdrop = b,
                            Action::Guides(g) => state.guides = g,
                            Action::Tool(t) => {
                                state.tool = t;
                                let none = matches!(t, ToolKind::Eraser | ToolKind::Laser);
                                state.width = if none { None } else { Some(4.0) };
                            }
                            Action::Width(w) => state.width = Some(w),
                            _ => {}
                        }
                    }
                    actions.extend(new);
                },
                (state, Vec::new()),
            );
        h.run();
        h
    }

    /// Where each of the toolbar's controls is, by name (the width's whatever its value).
    fn layout(h: &Harness<'static, (State, Vec<Action>)>) -> Vec<(&'static str, egui::Rect)> {
        [
            "Move the toolbar",
            "Backdrop",
            "Guides",
            "Tool",
            "Colour",
            "Width",
            "Disappearing ink",
            "Undo",
            "Redo",
            "Clear",
            "Save",
            "Save a region",
            "Copy",
            "More",
            "Freeze",
            "Collapse",
            "Close",
        ]
        .into_iter()
        .map(|name| {
            // The width's name gives its value; Freeze is Unfreeze while frozen.
            let node = match name {
                "Width" => h.get_by_label_contains(name),
                "Freeze" => h
                    .query_by_label(name)
                    .unwrap_or_else(|| h.get_by_label("Unfreeze")),
                _ => h.get_by_label(name),
            };
            (name, node.rect())
        })
        .collect()
    }

    #[test]
    fn choosing_another_option_never_resizes_the_toolbar() {
        let mut h = following(state());
        let before = layout(&h);
        // Every option, Frozen (the badge it was) and the eraser and the laser (no width)
        // among them, then back to the first.
        let groups: [(&str, Vec<String>); 3] = [
            ("Backdrop", Backdrop::ALL.map(Backdrop::name).to_vec()),
            ("Guides", Guides::ALL.map(Guides::name).to_vec()),
            ("Tool", ToolKind::ALL.map(tool_name).to_vec()),
        ];
        for (group, options) in groups {
            h.get_by_label(group).click_accesskit();
            h.run();
            for option in options.iter().rev().chain(options.first()) {
                h.get_by_label(option).click_accesskit();
                h.run();
                assert_eq!(
                    layout(&h),
                    before,
                    "{group}: {option} moved or resized the toolbar's controls"
                );
            }
        }
        let s = &h.state().0;
        assert_eq!(
            (s.backdrop, s.guides, s.tool),
            (Backdrop::Live, Guides::Off, ToolKind::Pen),
            "each pick was taken"
        );
        // F's freeze too: the backdrop's button changes its icon, not its size.
        h.state_mut().0.backdrop = Backdrop::Frozen;
        h.run();
        assert_eq!(layout(&h), before, "frozen");
    }

    #[test]
    fn the_group_buttons_say_their_choice_and_the_backdrop_its_colours() {
        let mut s = state();
        s.backdrop = Backdrop::Whiteboard;
        s.guides = Guides::Dots;
        s.tool = ToolKind::Ellipse;
        let h = harness(s);
        for (button, value) in [
            ("Backdrop", "Whiteboard"),
            ("Guides", "Dots"),
            ("Tool", "Ellipse"),
            ("Width: 4 px", "4 px"),
        ] {
            let node = h.get_by_label(button);
            assert_eq!(node.accesskit_node().value().as_deref(), Some(value));
        }
        // Each backdrop its own colours, each icon readable on its chip (3:1, as any icon),
        // in both themes.
        for dark in [false, true] {
            let chips = Backdrop::ALL.map(|b| backdrop_colours(b, dark));
            for (i, (b, (icon, fill))) in Backdrop::ALL.iter().zip(chips).enumerate() {
                let c = theme::contrast(icon, fill);
                assert!(
                    c >= 3.0,
                    "{b:?} (dark {dark}): its icon {c:.2}:1 on its chip"
                );
                for (other, _) in Backdrop::ALL.iter().zip(chips).skip(i + 1) {
                    assert_ne!(chips[i], backdrop_colours(*other, dark), "{b:?} {other:?}");
                }
            }
        }
    }

    #[test]
    fn the_width_readout_is_as_wide_at_1_px_as_at_200_px() {
        let (lo, hi) = Widths::RANGE;
        let mut h = following(state());
        let before = layout(&h);
        for w in [lo, 9.0, 12.0, 128.0, hi] {
            h.state_mut().0.width = Some(w);
            h.run();
            assert!(
                h.query_by_label(&format!("Width: {}", width_text(w)))
                    .is_some()
            );
            assert_eq!(layout(&h), before, "at {w} px");
        }
        // It fits the widest value, measured in the toolbar's font.
        let font = h.ctx.global_style().text_styles[&egui::TextStyle::Body].clone();
        let widest = width_texts()
            .map(|t| {
                h.ctx
                    .fonts_mut(|f| f.layout_no_wrap(t, font.clone(), Color32::BLACK).size().x)
            })
            .fold(0.0, f32::max);
        let readout = h.get_by_label_contains("Width: ").rect();
        assert!(readout.width() > widest + 12.0, "{readout:?} fits {widest}");
    }

    #[test]
    fn the_palette_says_its_colours_names() {
        overmark_core::i18n::init("en");
        let red = overmark_core::settings::DEFAULT_PALETTE[0];
        assert_eq!(colour_name(0, red), ("Red".into(), "Red (1)".into()));
        // Changed in Settings: no longer red, so a number.
        let other = Colour::rgb(1, 2, 3);
        assert_eq!(colour_name(0, other).0, "Colour 1");
        // The ninth is the custom one, whatever it is.
        assert_eq!(colour_name(8, other).0, "Custom colour");
    }

    #[test]
    fn disappearing_ink_turns_on_and_off_and_the_mode_says_what_it_is() {
        let mut h = harness(state());
        h.get_by_label("Disappearing ink").click_accesskit();
        h.run();
        assert_eq!(h.state().1, vec![Action::DisappearingInk(true)]);
        let mut s = state();
        s.disappearing_ink = true;
        let mut h = harness(s);
        h.get_by_label("Disappearing ink").click_accesskit();
        h.run();
        assert_eq!(h.state().1, vec![Action::DisappearingInk(false)]);
        overmark_core::i18n::init("en");
        assert!(tr!("mode-frozen-tooltip", key = "F").contains("F"));
        assert!(tr!("mode-live-tooltip", key = "F").contains("F"));
    }

    #[test]
    fn the_tooltips_give_the_keys() {
        overmark_core::i18n::init("en");
        assert_eq!(
            keys(&["key-ctrl", "key-shift", "key-alt"], "S"),
            "Ctrl+Shift+Alt+S"
        );
        assert!(tr!("toolbar-undo-tooltip", key = keys(&["key-ctrl"], "Z")).contains("Ctrl+Z"));
        let right = tr!(
            "toolbar-undo-right-click-tooltip",
            key = keys(&["key-ctrl"], "Z")
        );
        assert!(
            right.contains("Ctrl+Z") && right.contains("right-click"),
            "{right}"
        );
        for key in [
            "toolbar-backdrop",
            "toolbar-backdrop-tooltip",
            "toolbar-guides",
            "toolbar-guides-tooltip",
            "toolbar-tool",
            "toolbar-tool-tooltip",
            "toolbar-move",
            "toolbar-move-tooltip",
            "toolbar-colour-n",
            "toolbar-colour-n-tooltip",
            "toolbar-name",
        ] {
            assert!(overmark_core::i18n::has(key), "{key}");
        }
    }
}
