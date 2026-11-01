//! The windows' component library: cards, setting rows, buttons, toggles, segmented controls,
//! status chips, path fields, empty states, info bars, tabs and toolbars.
//!
//! Copied from Replica's `ui/widgets.rs` (generic: nothing in it is either app's), less what
//! only Replica uses (its table, folder tree, and time and size wording). Each component draws
//! with [`theme`]'s colours and sizes and has a name in the accessibility tree. The few words
//! they show themselves are the `widget-*` strings in `locales/en/settings.ftl`.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Id, Key, Layout, Margin, Response, RichText, Role, Sense,
    Stroke, Ui, UiBuilder, WidgetInfo, vec2,
};
use overmark_core::i18n::tr;

use super::icons::Icon;
use super::theme::{self, CARD_RADIUS, CONTROL_HEIGHT, CONTROL_RADIUS, Palette, space};

/// An error line, in the theme's error colour.
pub fn error_text(ui: &mut Ui, text: &str) {
    let colour = ui.visuals().error_fg_color;
    ui.label(RichText::new(text).color(colour));
}

/// A banner across the top of a tab: an error or a plain note.
pub fn banner(ui: &mut Ui, error: bool, text: &str) {
    let fill = if error {
        ui.visuals().error_fg_color.gamma_multiply(0.15)
    } else {
        ui.visuals().faint_bg_color
    };
    egui::Frame::new()
        .fill(fill)
        .inner_margin(8.0)
        .corner_radius(4.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if error {
                error_text(ui, text);
            } else {
                ui.label(text);
            }
        });
}

/// Shows `header` with `add` as its body, and also opens or closes it when a screen reader asks
/// (UI Automation's Expand and Collapse: egui answers only a click).
pub fn collapsing<R>(
    ui: &mut Ui,
    header: egui::CollapsingHeader,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::CollapsingResponse<R> {
    use egui::accesskit::Action;
    let response = header.show(ui, add);
    let id = response.header_response.id;
    let (expand, collapse) = ui.input(|i| {
        (
            i.has_accesskit_action_request(id, Action::Expand),
            i.has_accesskit_action_request(id, Action::Collapse),
        )
    });
    if (expand || collapse)
        && let Some(mut state) = egui::collapsing_header::CollapsingState::load(ui.ctx(), id)
    {
        state.set_open(expand);
        state.store(ui.ctx());
        ui.ctx().request_repaint();
    }
    response
}

/// A message written with line breaks for the terminal (Fluent's multi-line values), as one
/// paragraph the window wraps itself. A blank line still starts a new paragraph.
pub fn flow(text: &str) -> String {
    text.split("\n\n")
        .map(|p| p.split('\n').map(str::trim).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n\n")
}

// --- The component library -----------------------------------------------------------------

/// Names `response` as an item of a list or tab list, `selected` or not. egui's own
/// [`WidgetInfo::selected`] reads as toggled (a check box), not selected.
pub fn option_info(response: &Response, role: Role, selected: bool, label: &str) {
    response.widget_info(|| WidgetInfo::labeled(role, true, label));
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_role(role);
        node.set_label(label);
        node.clear_toggled();
        node.set_selected(selected);
    });
}

/// A card: a raised group with a title, an optional line saying what it's for, and its body
/// (often [`setting_row`]s). Its title is read as a heading, so a screen reader can jump
/// from card to card.
pub fn section_card<R>(
    ui: &mut Ui,
    title: &str,
    description: Option<&str>,
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    section_card_with_actions(ui, title, description, |_| {}, body)
}

/// A card without a title, for one whose title would only repeat what's above it (its tab's
/// name): its `description`, if any, as its first line, then its body.
pub fn plain_card<R>(ui: &mut Ui, description: Option<&str>, body: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::of(ui);
    let inner = card_frame(p)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(d) = description {
                ui.label(RichText::new(d).color(p.text_secondary));
                ui.add_space(space::XS);
            }
            ui.vertical(body).inner
        })
        .inner;
    ui.add_space(space::CARD_GAP);
    inner
}

/// A [`section_card`] with buttons (or other controls) that `actions` adds at the right of its
/// title, for what acts on the whole card.
pub fn section_card_with_actions<R>(
    ui: &mut Ui,
    title: &str,
    description: Option<&str>,
    actions: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = theme::of(ui);
    let inner = card_frame(p)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    actions(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        heading(ui, title);
                    });
                });
            });
            if let Some(d) = description {
                ui.label(RichText::new(d).color(p.text_secondary));
            }
            ui.add_space(space::XS);
            // A column of its own, so its first row knows it's first.
            ui.vertical(body).inner
        })
        .inner;
    ui.add_space(space::CARD_GAP);
    inner
}

/// A section's title, read as a heading. On one line, cut with "…" to the width it has (a long
/// job's name in it), whole on hover and for a screen reader.
pub fn heading(ui: &mut Ui, title: &str) -> Response {
    let response = ui
        .add(egui::Label::new(RichText::new(title).text_style(theme::section_style())).truncate());
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Heading);
        node.set_label(title);
    });
    response
}

/// Makes a node for `id` in the accessibility tree with `role` and `label`, for a group the
/// widgets added with [`UiBuilder::accessibility_parent`] then sit in.
fn group_node(ui: &Ui, id: Id, role: Role, label: &str) {
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(role);
        node.set_label(label);
    });
}

/// A dialog's frame (for an `egui::Modal`): a card with room round its contents and a
/// shadow. egui's own is a popup's, with little room.
pub fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    let p = theme::palette(ctx.theme());
    let style = ctx.global_style();
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(space::XL as i8))
        .shadow(style.visuals.window_shadow)
}

/// A card's frame alone: its fill, hairline border, corners and padding.
pub fn card_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(space::CARD_PADDING as i8))
}

/// A row in a card: `label`, with `description` under it, on the left, and the control
/// `control` adds on the right, both centred in the row's height. A row below another in the
/// same column has a hairline above it.
///
/// For a screen reader the row is its control, named by the label and described by the
/// description, as Windows' own settings read: the text beside it isn't read twice.
pub fn setting_row(
    ui: &mut Ui,
    label: &str,
    description: Option<&str>,
    control: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    let (response, _) = row(ui, label, description, true, control);
    let response = response.accessible_name(label);
    if let Some(d) = description {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(d));
    }
    response
}

/// A row laid out as a [`setting_row`] whose control keeps its own name (a status chip, a
/// value): `label` and `description` are read as text beside it.
pub fn info_row(
    ui: &mut Ui,
    label: &str,
    description: Option<&str>,
    control: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    row(ui, label, description, false, control).0
}

/// An [`info_row`] showing a path in a [`path_field`] taking most of the row (named by the
/// path).
pub fn path_row(ui: &mut Ui, label: &str, description: Option<&str>, path: &str) -> Response {
    info_row(ui, label, description, |ui| {
        ui.set_max_width((ui.available_width() * 0.72).max(200.0));
        path_field(ui, path)
    })
}

/// Lays out a row: the control on the right, then the label and description in the width it
/// leaves, painted (when `painted`) or as labels. Returns the control's response and the
/// row's rect.
fn row(
    ui: &mut Ui,
    label: &str,
    description: Option<&str>,
    painted: bool,
    control: impl FnOnce(&mut Ui) -> Response,
) -> (Response, egui::Rect) {
    let p = theme::of(ui);
    // Below something else in the same column: a hairline between them.
    if ui.cursor().top() > ui.max_rect().top() + 0.5 {
        divider(ui, p);
    }
    // The text's height is known once it's laid out at the width the control leaves, so the
    // row is sized by last frame's, and the frame is drawn again when that changes.
    let id = ui.scope_id().with(("setting-row", label));
    let text_height: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(CONTROL_HEIGHT);
    let pad = space::ROW_PADDING;
    let least = if description.is_some() {
        space::ROW_MIN_HEIGHT
    } else {
        0.0
    };
    let rect = egui::Rect::from_min_size(
        ui.cursor().min,
        vec2(
            ui.available_width(),
            (text_height.max(CONTROL_HEIGHT) + 2.0 * pad).max(least),
        ),
    );
    let control = ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(0.0, pad)))
            .layout(Layout::right_to_left(Align::Center)),
        control,
    );
    let response = control.inner;
    let text_width = (control.response.rect.left() - rect.left() - space::L).max(80.0);
    let top = rect.center().y - text_height / 2.0;
    let measured = if painted {
        let painter = ui.painter();
        let body = egui::TextStyle::Body.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let label = painter.layout(label.to_owned(), body, p.text, text_width);
        let mut height = label.size().y;
        painter.galley(egui::pos2(rect.left(), top), label, p.text);
        if let Some(d) = description {
            let d = painter.layout(d.to_owned(), small, p.text_secondary, text_width);
            painter.galley(
                egui::pos2(rect.left(), top + height + 2.0),
                d.clone(),
                p.text,
            );
            height += 2.0 + d.size().y;
        }
        height
    } else {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    egui::pos2(rect.left(), top),
                    vec2(text_width, text_height),
                ))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // A name on one line (a long path in it cut with "…"), what's said of it wrapped.
                ui.add(egui::Label::new(label).truncate());
                if let Some(d) = description {
                    ui.label(RichText::new(d).small().color(p.text_secondary));
                }
            },
        )
        .response
        .rect
        .height()
    };
    if (measured - text_height).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, measured));
        ui.ctx().request_discard("a setting row's height changed");
    }
    ui.advance_cursor_after_rect(rect);
    (response, rect)
}

/// A hairline across the width of `ui`.
fn divider(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
}

/// A button at the control height in `fills` (at rest, hovered, pressed), its text (and
/// `icon`, if any, before it) in `colour`.
fn styled_button(
    ui: &mut Ui,
    icon: Option<Icon>,
    text: &str,
    fills: [Color32; 3],
    colour: Color32,
    border: Stroke,
) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, colour);
    let icon_width = if icon.is_some() { 16.0 + space::S } else { 0.0 };
    let width = (2.0 * space::M + icon_width + galley.size().x).max(88.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            fills[0]
        } else if response.is_pointer_button_down_on() {
            fills[2]
        } else if response.hovered() {
            fills[1]
        } else {
            fills[0]
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(CONTROL_RADIUS),
            fill,
            border,
            egui::StrokeKind::Inside,
        );
        let mut x = rect.center().x - (icon_width + galley.size().x) / 2.0;
        if let Some(icon) = icon {
            let r =
                egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0));
            icon.paint(painter, r.shrink(1.0), colour);
            x += icon_width;
        }
        let y = rect.center().y - galley.size().y / 2.0;
        painter.galley(egui::pos2(x, y), galley, colour);
    }
    response
}

/// The page's main action, in the accent.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = theme::of(ui);
    styled_button(
        ui,
        None,
        text,
        [p.accent, p.accent_hover, p.accent_pressed],
        p.on_accent,
        Stroke::NONE,
    )
}

/// Any other action.
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let p = theme::of(ui);
    styled_button(
        ui,
        None,
        text,
        [p.control, p.control_hover, p.control_pressed],
        p.text,
        secondary_border(p),
    )
}

fn secondary_border(p: &Palette) -> Stroke {
    Stroke::new(1.0, p.border.lerp_to_gamma(p.control_border, 0.35))
}

/// An action that deletes or removes: a secondary button with red text, and `icon` before it
/// if given.
pub fn danger_button(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let p = theme::of(ui);
    styled_button(
        ui,
        icon,
        text,
        [p.control, p.control_hover, p.control_pressed],
        p.failed,
        secondary_border(p),
    )
}

/// Filled red, only for the last step of something that can't be undone: the "Delete" or
/// "Remove all" in its confirmation, "Use a mirror anyway", "Replace it anyway".
pub fn danger_button_filled(ui: &mut Ui, text: &str) -> Response {
    let p = theme::of(ui);
    styled_button(
        ui,
        None,
        text,
        [p.danger, p.danger_hover, p.danger_pressed],
        p.on_danger,
        Stroke::NONE,
    )
}

/// A button with only an icon: `name` is what a screen reader says and the hover text.
pub fn icon_button(ui: &mut Ui, icon: Icon, name: &str) -> Response {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            p.subtle_selected
        } else if response.hovered() {
            p.subtle_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), fill);
        icon.paint(ui.painter(), rect.shrink(7.0), p.text);
    }
    response.on_hover_text(name)
}

/// How a status reads: each has its colour and its own glyph, so it isn't told by colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Failed,
    /// Skipped, overdue or partly done.
    Warning,
    /// Not run, or nothing to say.
    Neutral,
    /// A plain fact, nothing good or bad (installed or portable).
    Info,
}

impl Tone {
    fn colours(self, p: &Palette) -> (Color32, Color32, Icon) {
        match self {
            Tone::Ok => (p.ok, p.ok_fill, Icon::Check),
            Tone::Failed => (p.failed, p.failed_fill, Icon::Cross),
            Tone::Warning => (p.warning, p.warning_fill, Icon::Warning),
            Tone::Neutral => (p.neutral, p.neutral_fill, Icon::Dash),
            Tone::Info => (p.accent, p.info_fill, Icon::Info),
        }
    }
}

/// A choice of a few `options` side by side as one control, `selected` filled with the accent.
/// Read as a radio group, each option a radio button. Returns the group's response (for
/// [`setting_row`] to name) and the option clicked, if it isn't the one already chosen.
pub fn segmented(ui: &mut Ui, options: &[&str], selected: usize) -> (Response, Option<usize>) {
    let p = theme::of(ui);
    let id = ui.next_auto_id().with("segmented");
    ui.ctx()
        .accesskit_node_builder(id, |node| node.set_role(Role::RadioGroup));
    let enabled = ui.is_enabled();
    let mut chosen = None;
    let frame = ui.scope_builder(UiBuilder::new().accessibility_parent(id), |ui| {
        egui::Frame::new()
            .fill(p.control)
            .stroke(secondary_border(p))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
            .inner_margin(Margin::same(2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // A row's control is laid out right to left: add the options last first there,
                // so they read in their order.
                ui.horizontal(|ui| {
                    let mut order: Vec<(usize, &&str)> = options.iter().enumerate().collect();
                    if ui.layout().prefer_right_to_left() {
                        order.reverse();
                    }
                    for (i, option) in order {
                        let on = i == selected;
                        let colour = if on { p.on_accent } else { p.text };
                        let galley = ui.painter().layout_no_wrap(
                            (*option).to_owned(),
                            egui::TextStyle::Button.resolve(ui.style()),
                            colour,
                        );
                        let size = vec2(galley.size().x + 2.0 * space::M, CONTROL_HEIGHT - 4.0);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        response.widget_info(|| {
                            WidgetInfo::selected(Role::RadioButton, enabled, on, *option)
                        });
                        if response.clicked() && !on {
                            chosen = Some(i);
                        }
                        let fill = if on {
                            p.accent
                        } else if response.hovered() && enabled {
                            p.control_hover
                        } else {
                            Color32::TRANSPARENT
                        };
                        let painter = ui.painter();
                        painter.rect_filled(rect, CornerRadius::same(CONTROL_RADIUS - 1), fill);
                        painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
                    }
                });
            })
            .response
    });
    let group = ui.interact(frame.inner.rect, id, Sense::hover());
    (group, chosen)
}

/// A small pill with a status's glyph and `text` (which a screen reader reads).
pub fn status_chip(ui: &mut Ui, tone: Tone, text: &str) -> Response {
    let p = theme::of(ui);
    let (fg, fill, icon) = tone.colours(p);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL_SIZE),
        fg,
    );
    let glyph = CHIP_GLYPH;
    let pad = vec2(space::S, space::XS);
    let size = vec2(
        status_chip_width(ui, text),
        (galley.size().y + pad.y * 2.0).max(22.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(u8::MAX), fill);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + pad.x + glyph / 2.0, rect.center().y),
            vec2(glyph, glyph),
        );
        icon.paint(painter, icon_rect, fg);
        painter.galley(
            egui::pos2(
                icon_rect.right() + space::XS,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            fg,
        );
    }
    response
}

/// `text` cut in the middle with "…" to fit `width` in `font`, keeping more of its end (a
/// path's file name). Whole if it fits.
pub fn middle_ellipsis(ui: &Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let fits = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
            <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| {
        let head = keep * 2 / 5;
        let tail = keep - head;
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - tail..]);
        s
    };
    // The most characters that still fit.
    let (mut lo, mut hi) = (0, chars.len() - 1);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

/// A path in a field-like box: in monospace, cut in the middle when it's too long, with the
/// full path in its hover text and for a screen reader, and a button that copies it.
pub fn path_field(ui: &mut Ui, path: &str) -> Response {
    let p = theme::of(ui);
    let copy_name = tr("widget-copy-path");
    egui::Frame::new()
        .fill(p.control)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(CONTROL_RADIUS))
        .inner_margin(Margin {
            left: space::S as i8,
            right: 2,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| {
            let size = vec2(ui.available_width(), CONTROL_HEIGHT - 6.0);
            ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Copy, &copy_name).clicked() {
                    ui.ctx().copy_text(path.to_owned());
                }
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let shown = middle_ellipsis(ui, path, &font, ui.available_width() - space::S);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let response = ui.add(
                        egui::Label::new(RichText::new(shown).font(font))
                            .truncate()
                            .sense(Sense::hover()),
                    );
                    // The whole path, not what fits.
                    response
                        .ctx
                        .accesskit_node_builder(response.id, |node| node.set_label(path));
                    response.on_hover_text(path)
                })
                .inner
            })
            .inner
        })
        .inner
}

/// An on/off switch, as Windows 11's settings use: the accent when on. Read as a check box;
/// name it with [`setting_row`] or [`Response::labelled_by`].
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = theme::of(ui);
    let size = vec2(40.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let hovered = response.hovered();
        let radius = CornerRadius::same(u8::MAX);
        let painter = ui.painter();
        if *on {
            let fill = if hovered { p.accent_hover } else { p.accent };
            painter.rect_filled(rect, radius, fill);
        } else {
            let fill = if hovered { p.control_hover } else { p.control };
            painter.rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, p.control_border),
                egui::StrokeKind::Inside,
            );
        }
        let knob = if *on { p.on_accent } else { p.text_secondary };
        let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
        let r = if hovered { 7.0 } else { 6.0 };
        painter.circle_filled(egui::pos2(x, rect.center().y), r, knob);
    }
    response
}

/// What an empty list shows: an icon, a title, a line of text and, if given, a button.
/// `true` when the button is clicked.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, text: &str, action: Option<&str>) -> bool {
    let p = theme::of(ui);
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(space::XL);
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        icon.paint(ui.painter(), rect, p.text_secondary);
        ui.add_space(space::M);
        ui.label(RichText::new(title).text_style(theme::section_style()));
        ui.label(RichText::new(text).color(p.text_secondary));
        if let Some(action) = action {
            ui.add_space(space::M);
            clicked = primary_button(ui, action).clicked();
        }
        ui.add_space(space::XL);
    });
    clicked
}

/// What an info bar says: news, a warning or an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoKind {
    Info,
    Warning,
    Error,
}

/// A bar across the content with an icon and `text` (and a bold `title` before it, if given),
/// read as a status, or an alert for an error.
pub fn info_bar(ui: &mut Ui, kind: InfoKind, title: Option<&str>, text: &str) -> Response {
    let p = theme::of(ui);
    let (fill, glyph, icon, role) = match kind {
        InfoKind::Info => (p.info_fill, p.accent, Icon::Info, Role::Status),
        InfoKind::Warning => (p.warning_fill, p.warning, Icon::Warning, Role::Status),
        InfoKind::Error => (p.failed_fill, p.failed, Icon::Cross, Role::Alert),
    };
    // The bar is a status (or alert) holding its text: the text is read from what's in it,
    // not repeated as the bar's own name.
    let id = ui.scope_id().with(("info-bar", text));
    ui.ctx()
        .accesskit_node_builder(id, |node| node.set_role(role));
    ui.scope_builder(UiBuilder::new().accessibility_parent(id), |ui| {
        egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(space::M as i8, space::S as i8 + 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    icon.paint(ui.painter(), rect, glyph);
                    ui.add_space(space::XS);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(t) = title {
                            ui.label(RichText::new(t).font(theme::semibold(theme::BODY_SIZE)));
                        }
                        ui.label(text);
                    });
                });
            })
            .response
    })
    .inner
}

/// Text drawn without a node of its own in the accessibility tree, cut with "…" to the width
/// it has and shown whole on hover: for a table's cells, whose row is named with all of them.
pub fn quiet_text(ui: &mut Ui, text: &str, colour: Option<Color32>) -> Response {
    let colour = colour.unwrap_or(theme::of(ui).text);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let width = ui.available_width().max(0.0);
    let full = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), colour);
    let cut = full.size().x > width;
    let shown = if cut {
        let mut job = egui::text::LayoutJob::single_section(
            text.to_owned(),
            egui::TextFormat::simple(font, colour),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        ui.painter().layout_job(job)
    } else {
        full
    };
    let (rect, response) = ui.allocate_exact_size(shown.size(), Sense::hover());
    ui.painter().galley(rect.min, shown, colour);
    if cut {
        response.on_hover_text(text)
    } else {
        response
    }
}

// --- Added for the Jobs page and the job editor --------------------------------------------

/// A secondary button with `icon` before its text.
pub fn secondary_button_with_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    let p = theme::of(ui);
    styled_button(
        ui,
        Some(icon),
        text,
        [p.control, p.control_hover, p.control_pressed],
        p.text,
        secondary_border(p),
    )
}

/// A small grey pill with `text`, for a kind or a mode beside a name ("Mirror"). Read as text.
pub fn tag(ui: &mut Ui, text: &str) -> Response {
    let p = theme::of(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL_SIZE),
        p.text_secondary,
    );
    let size = vec2(tag_width(ui, text), 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(CONTROL_RADIUS),
            p.neutral_fill,
            Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        painter.galley(
            rect.center() - galley.size() / 2.0,
            galley,
            p.text_secondary,
        );
    }
    response
}

/// A pill with `text` and a small cross that removes it: `true` when the cross is clicked.
/// The cross is a button named `remove_name` ("Remove *.tmp").
pub fn removable_chip(ui: &mut Ui, text: &str, remove_name: &str) -> bool {
    let p = theme::of(ui);
    let mut removed = false;
    egui::Frame::new()
        .fill(p.neutral_fill)
        .corner_radius(CornerRadius::same(u8::MAX))
        .inner_margin(Margin {
            left: space::S as i8 + 2,
            right: 2,
            top: 1,
            bottom: 1,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).small().color(p.text));
                let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                response.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, ui.is_enabled(), remove_name)
                });
                if ui.is_rect_visible(rect) {
                    if response.hovered() {
                        ui.painter()
                            .circle_filled(rect.center(), 9.0, p.subtle_selected);
                    }
                    Icon::Cross.paint(ui.painter(), rect.shrink(5.0), p.text_secondary);
                }
                removed = response.on_hover_text(remove_name).clicked();
            });
        });
    removed
}

/// Text to copy out exactly (a password, a key), large in monospace in a field-like box that
/// can be selected, with a button named `copy_name` beside it that copies it. `true` when
/// copied.
pub fn copy_field(ui: &mut Ui, text: &str, copy_name: &str) -> bool {
    let p = theme::of(ui);
    let mut copied = false;
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(p.control)
            .stroke(Stroke::new(1.0, p.control_border))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(space::M as i8, space::S as i8))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .font(egui::FontId::monospace(18.0))
                            .color(p.text),
                    )
                    .selectable(true),
                );
            });
        if secondary_button_with_icon(ui, Icon::Copy, copy_name).clicked() {
            ui.ctx().copy_text(text.to_owned());
            copied = true;
        }
    });
    copied
}

/// An [`info_bar`] without a title whose group is named by its text's own node, so a screen
/// reader reads the message once (and a test finds it once): for a page's passing messages,
/// "Saved docs." or a validation problem.
pub fn message_bar(ui: &mut Ui, kind: InfoKind, text: &str) -> Response {
    let p = theme::of(ui);
    let (fill, glyph, icon, role) = match kind {
        InfoKind::Info => (p.info_fill, p.accent, Icon::Info, Role::Status),
        InfoKind::Warning => (p.warning_fill, p.warning, Icon::Warning, Role::Status),
        InfoKind::Error => (p.failed_fill, p.failed, Icon::Cross, Role::Alert),
    };
    let id = ui.scope_id().with(("message-bar", text));
    ui.ctx()
        .accesskit_node_builder(id, |node| node.set_role(role));
    ui.scope_builder(UiBuilder::new().accessibility_parent(id), |ui| {
        egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(space::M as i8, space::S as i8 + 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    icon.paint(ui.painter(), rect, glyph);
                    ui.add_space(space::XS);
                    let label = ui.add(egui::Label::new(text).wrap());
                    // Laid out unseen (a dialog's first frame), the text gets no node, and a
                    // name pointing at it would panic AccessKit's Windows adapter.
                    if ui.is_visible() {
                        ui.ctx().accesskit_node_builder(id, |node| {
                            node.push_labelled_by(label.id.accesskit_id())
                        });
                    }
                });
            })
            .response
    })
    .inner
}

/// A chip the size of the column's width at most, that fills with the accent as `fraction`
/// (0 to 1) of something is done: a track across the whole chip in the neutral fill, and
/// `text` centred over both. Read as a progress indicator named `text`.
pub fn progress_chip(ui: &mut Ui, fraction: f32, text: &str) -> Response {
    let p = theme::of(ui);
    let fraction = fraction.clamp(0.0, 1.0);
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), theme::semibold(theme::SMALL_SIZE), p.text);
    let width = (galley.size().x + 2.0 * space::M).max(ui.available_width().min(140.0));
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::ProgressIndicator, true, text));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_numeric_value(f64::from(fraction) * 100.0);
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(100.0);
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(u8::MAX);
        painter.rect_filled(rect, radius, p.neutral_fill);
        let done =
            egui::Rect::from_min_size(rect.min, vec2(rect.width() * fraction, rect.height()));
        // The selection's tint in light mode; in dark mode it's too near the track, so the
        // accent half-blended over it (white text on it is still over 6:1).
        let fill = if ui.visuals().dark_mode {
            p.accent.gamma_multiply(0.45)
        } else {
            p.selection
        };
        painter.with_clip_rect(done).rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, p.text);
    }
    response
}

/// A status chip for a live state that isn't a result ("Waiting its turn"): `icon` in the
/// accent and `text`, on the info fill.
pub fn info_chip(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    let p = theme::of(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL_SIZE),
        p.text,
    );
    let glyph = 12.0;
    let pad = vec2(space::S, space::XS);
    let size = vec2(
        pad.x * 2.0 + glyph + space::XS + galley.size().x,
        (galley.size().y + pad.y * 2.0).max(22.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(u8::MAX), p.info_fill);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + pad.x + glyph / 2.0, rect.center().y),
            vec2(glyph, glyph),
        );
        icon.paint(painter, icon_rect, p.accent);
        painter.galley(
            egui::pos2(
                icon_rect.right() + space::XS,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            p.text,
        );
    }
    response
}

/// Numbered steps in a row, "1  Save your password  2  Create the repository": the steps
/// before `current` ticked, `current` in the accent. Read as a list of their names.
pub fn steps(ui: &mut Ui, names: &[String], current: usize) {
    let p = theme::of(ui);
    let id = ui.scope_id().with("steps");
    group_node(ui, id, Role::List, &names.join(", "));
    ui.scope_builder(UiBuilder::new().accessibility_parent(id), |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, name) in names.iter().enumerate() {
                let (fill, fg) = if i < current {
                    (p.ok_fill, p.ok)
                } else if i == current {
                    (p.accent, p.on_accent)
                } else {
                    (p.neutral_fill, p.text_secondary)
                };
                let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 11.0, fill);
                if i < current {
                    Icon::Check.paint(ui.painter(), rect.shrink(6.0), fg);
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
                        theme::semibold(theme::SMALL_SIZE),
                        fg,
                    );
                }
                let text = RichText::new(name);
                let text = if i == current {
                    text.font(theme::semibold(theme::BODY_SIZE)).color(p.text)
                } else {
                    text.color(p.text_secondary)
                };
                let label = ui.label(text);
                ui.ctx().accesskit_node_builder(label.id, |node| {
                    node.set_role(Role::ListItem);
                    node.set_label(name.as_str());
                });
                if i + 1 < names.len() {
                    ui.add_space(space::S);
                    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::hover());
                    ui.painter()
                        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
                    ui.add_space(space::S);
                }
            }
        });
    });
}

// --- Added for Settings' tabs and the pages' toolbars --------------------------------------

/// A row of tabs under a page's header, the `selected` one in semibold with the accent's
/// underline, over a hairline across the width. A click or the arrow keys (with Home and End)
/// choose a tab, as in Windows' own tab lists. Read as a tab list named `name`. `true` when the
/// choice changed.
pub fn tab_strip(ui: &mut Ui, name: &str, tabs: &[&str], selected: &mut usize) -> bool {
    let p = theme::of(ui);
    let id = ui.scope_id().with(("tab-strip", name));
    let tab_id = |i: usize| id.with(i);
    let before = *selected;

    // The keys, before the tabs are drawn, so the tab they choose is shown this frame.
    if let Some(focused) = (0..tabs.len()).find(|&i| ui.memory(|m| m.has_focus(tab_id(i)))) {
        let (left, right, home, end) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowRight),
                i.consume_key(egui::Modifiers::NONE, Key::Home),
                i.consume_key(egui::Modifiers::NONE, Key::End),
            )
        });
        let last = tabs.len() - 1;
        let to = match (left, right, home, end) {
            (true, ..) => Some(focused.checked_sub(1).unwrap_or(last)),
            (_, true, ..) => Some(if focused == last { 0 } else { focused + 1 }),
            (_, _, true, _) => Some(0),
            (.., true) => Some(last),
            _ => None,
        };
        if let Some(to) = to {
            *selected = to;
            ui.memory_mut(|m| m.request_focus(tab_id(to)));
        }
        // Left and Right stay in the strip, not moving the focus out of it.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                tab_id(focused),
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
    }

    group_node(ui, id, Role::TabList, name);
    let height = CONTROL_HEIGHT + space::S;
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().hline(
        strip.x_range(),
        strip.bottom() - 0.5,
        Stroke::new(1.0, p.border),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(strip)
            .layout(Layout::left_to_right(Align::Center))
            .accessibility_parent(id),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            for (i, title) in tabs.iter().enumerate() {
                let on = *selected == i;
                let font = if on {
                    theme::semibold(theme::BODY_SIZE)
                } else {
                    egui::FontId::proportional(theme::BODY_SIZE)
                };
                let colour = if on { p.text } else { p.text_secondary };
                let galley = ui
                    .painter()
                    .layout_no_wrap((*title).to_owned(), font, colour);
                // Room for the semibold title either way, so the tabs don't shift as one is
                // chosen.
                let bold = ui.painter().layout_no_wrap(
                    (*title).to_owned(),
                    theme::semibold(theme::BODY_SIZE),
                    colour,
                );
                let size = vec2(bold.size().x + 2.0 * space::M, height);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let response = ui.interact(rect, tab_id(i), Sense::click());
                option_info(&response, Role::Tab, on, title);
                if response.clicked() {
                    *selected = i;
                }
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter();
                if response.hovered() || response.is_pointer_button_down_on() {
                    let fill = if response.is_pointer_button_down_on() {
                        p.subtle_selected
                    } else {
                        p.subtle_hover
                    };
                    painter.rect_filled(
                        rect.shrink2(vec2(0.0, space::XS)),
                        CornerRadius::same(CONTROL_RADIUS),
                        fill,
                    );
                }
                painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
                if on {
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + space::M, rect.bottom() - 3.0),
                        egui::pos2(rect.right() - space::M, rect.bottom()),
                    );
                    painter.rect_filled(bar, CornerRadius::same(2), p.accent);
                }
            }
        },
    );
    *selected != before
}

/// A toolbar: `add`'s controls in a row, [`space::TOOLBAR_GAP`] apart, with
/// [`toolbar_group_gap`] between groups of them.
pub fn toolbar<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::TOOLBAR_GAP;
        add(ui)
    })
    .inner
}

/// The room between two groups of a [`toolbar`]'s controls.
pub fn toolbar_group_gap(ui: &mut Ui) {
    ui.add_space(space::TOOLBAR_GROUP_GAP - ui.spacing().item_spacing.x);
}

// --- Added for History's toolbar and long names --------------------------------------------

/// A [`status_chip`]'s glyph, in points.
const CHIP_GLYPH: f32 = 12.0;

/// How wide a [`status_chip`] with `text` is, to keep room for it after something cut to fit.
pub fn status_chip_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL_SIZE),
        Color32::WHITE,
    );
    space::S * 2.0 + CHIP_GLYPH + space::XS + galley.size().x
}

/// How wide a [`tag`] with `text` is.
pub fn tag_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL_SIZE),
        Color32::WHITE,
    );
    galley.size().x + 2.0 * space::S - 2.0
}

/// Runs `add` in what's left of the row less `room` points, kept for what follows it: a name
/// cut with "…" that still leaves its tag or chip in view.
pub fn leaving_room<R>(ui: &mut Ui, room: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = (ui.available_width() - room - ui.spacing().item_spacing.x).max(0.0);
    ui.scope(|ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// A page's title (the editor's "Edit …"), read as a heading: on one line, cut with "…" to
/// the page's width, whole on hover and for a screen reader.
pub fn page_title(ui: &mut Ui, title: &str) -> Response {
    let response = ui.add(egui::Label::new(RichText::new(title).heading()).truncate());
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Heading);
        node.set_label(title);
    });
    response
}

/// What a [`dropdown`] shows closed, and how wide it is with that whole.
fn dropdown_text(ui: &Ui, label: Option<&str>, value: &str) -> (String, f32) {
    let text = match label {
        Some(label) => overmark_core::tr!("widget-dropdown-labelled", label = label, value = value),
        None => value.to_owned(),
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let spacing = ui.spacing();
    // The button's padding, the gap and the arrow round its text.
    let chrome = 2.0 * spacing.button_padding.x + spacing.item_spacing.x + spacing.icon_width;
    let whole = ui
        .painter()
        .layout_no_wrap(text.clone(), font, Color32::WHITE)
        .size()
        .x
        + chrome;
    (text, whole)
}

/// How wide a [`dropdown`] is closed, given the room: to see whether it fits beside something.
pub fn dropdown_width(ui: &Ui, label: Option<&str>, value: &str, max_width: f32) -> f32 {
    let (_, whole) = dropdown_text(ui, label, value);
    whole.min(max_width).max(ui.spacing().combo_width)
}

/// One choice in a dropdown's list, shown chosen when `selected`. Clicked, it closes the list:
/// egui closes it on a pointer click only, and this closes it on a screen reader's (AccessKit's)
/// or the keyboard's choice too, so the next press opens the dropdown rather than closing it.
pub fn dropdown_choice<'a>(
    ui: &mut Ui,
    selected: bool,
    text: impl egui::IntoAtoms<'a>,
) -> Response {
    let response = ui.selectable_label(selected, text);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A [`dropdown_choice`] that sets `current` to `value` when clicked, as
/// [`Ui::selectable_value`] does.
pub fn dropdown_value<'a, V: PartialEq>(
    ui: &mut Ui,
    current: &mut V,
    value: V,
    text: impl egui::IntoAtoms<'a>,
) -> Response {
    let mut response = dropdown_choice(ui, *current == value, text);
    if response.clicked() && *current != value {
        *current = value;
        response.mark_changed();
    }
    response
}

/// A dropdown showing `value` closed, after `label` when it carries its own ("Job: All jobs"),
/// at most `max_width` wide (and never wider than what's left of its row): a longer value is
/// cut with "…" and shown whole on hover. Open, its list is as wide as its widest choice; make
/// its choices with [`dropdown_choice`] or [`dropdown_value`], so they close it. A screen
/// reader reads `value` as its value; name it with [`Response::labelled_by`], or
/// [`Response::accessible_name`] with `label`.
pub fn dropdown<R>(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    label: Option<&str>,
    value: &str,
    max_width: f32,
    choices: impl FnOnce(&mut Ui) -> R,
) -> Response {
    let (text, whole) = dropdown_text(ui, label, value);
    let width = dropdown_width(ui, label, value, max_width).min(ui.available_width());
    let response = ui
        .scope(|ui| {
            ui.set_max_width(width);
            egui::ComboBox::from_id_salt(id_salt)
                .selected_text(text)
                .width(width)
                .truncate()
                .show_ui(ui, choices)
                .response
        })
        .inner;
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_value(value));
    if whole > width {
        response.on_hover_text(value)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::flow;

    #[test]
    fn line_breaks_flow_but_paragraphs_stay() {
        assert_eq!(flow("one\ntwo\n  three"), "one two three");
        assert_eq!(flow("a\nb\n\nc"), "a b\n\nc");
    }
}
