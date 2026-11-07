use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn view(phase: Phase, secs: u64, confirm_discard: bool) -> View {
    View {
        phase,
        elapsed: Duration::from_secs(secs),
        confirm_discard,
    }
}

fn harness(v: View) -> Harness<'static, (View, Vec<BarAction>)> {
    overmark_core::i18n::init("en");
    let mut h = Harness::builder()
        .with_size(vec2(WINDOW.0, WINDOW.1))
        .with_context_setup(|ctx| crate::ui::install(ctx).expect("Windows has a text font"))
        .build_ui_state(
            |ui, (v, actions): &mut (View, Vec<BarAction>)| {
                actions.extend(show(ui.ctx(), v));
            },
            (v, Vec::new()),
        );
    h.run();
    h
}

fn press(h: &mut Harness<'static, (View, Vec<BarAction>)>, name: &str) -> Vec<BarAction> {
    h.state_mut().1.clear();
    h.get_by_label(name).click_accesskit();
    h.run();
    h.state().1.clone()
}

#[test]
fn before_recording_it_offers_record_and_cancel_and_stop_is_greyed_out() {
    let mut h = harness(view(Phase::Ready, 0, false));
    let _ = h.get_by_label("Recorded 0:00");
    assert_eq!(press(&mut h, "Record"), [BarAction::Record]);
    assert_eq!(press(&mut h, "Cancel"), [BarAction::Discard]);
    // Stop does nothing until there's something to save.
    assert!(h.get_by_label("Stop").accesskit_node().is_disabled());
    assert!(h.query_by_label("Pause").is_none());
}

#[test]
fn while_recording_it_offers_pause_stop_and_discard() {
    let mut h = harness(view(Phase::Recording, 75, false));
    let _ = h.get_by_label("Recorded 1:15");
    assert_eq!(press(&mut h, "Pause"), [BarAction::Pause]);
    assert_eq!(press(&mut h, "Stop"), [BarAction::Stop]);
    assert_eq!(press(&mut h, "Discard"), [BarAction::Discard]);
    assert!(h.query_by_label("Record").is_none());
}

#[test]
fn paused_resume_takes_pauses_place() {
    let mut h = harness(view(Phase::Paused, 9, false));
    assert_eq!(press(&mut h, "Resume"), [BarAction::Record]);
    assert!(h.query_by_label("Pause").is_none());
    assert_eq!(press(&mut h, "Stop"), [BarAction::Stop]);
}

#[test]
fn discarding_a_recording_asks_first_in_the_bar() {
    let mut h = harness(view(Phase::Recording, 3, true));
    let _ = h.get_by_label("Discard this recording?");
    // While it asks, nothing else is offered.
    assert!(h.query_by_label("Stop").is_none());
    assert!(h.query_by_label("Pause").is_none());
    assert_eq!(press(&mut h, "Keep"), [BarAction::Keep]);
    assert_eq!(press(&mut h, "Discard"), [BarAction::ConfirmDiscard]);
}

/// Every state fits the window, the question included, so nothing is cut off.
#[test]
fn every_state_fits_its_window() {
    for v in [
        view(Phase::Ready, 0, false),
        view(Phase::Recording, 3725, false),
        view(Phase::Paused, 3725, false),
        view(Phase::Recording, 3725, true),
    ] {
        let h = harness(v.clone());
        let frame = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("overmark-record-bar")))
            .expect("the bar's area");
        assert!(
            frame.max.x <= WINDOW.0 - 2.0 && frame.max.y <= WINDOW.1 - 2.0,
            "{v:?}: {frame:?} in a {WINDOW:?} window"
        );
    }
}

#[test]
fn the_bar_draws_its_frame_and_leaves_the_rest_clear() {
    overmark_core::i18n::init("en");
    let (w, h) = (WINDOW.0 as usize, WINDOW.1 as usize);
    let rgba = render_offscreen(&view(Phase::Recording, 12, false), w, h, 1.0);
    let alpha = |x: usize, y: usize| rgba[(y * w + x) * 4 + 3];
    // Inside the frame, solid; at the window's far right, clear (the pointer goes through).
    assert_eq!(alpha(20, 25), 255);
    assert_eq!(alpha(w - 3, h / 2), 0);
}

/// The visible frame of the bar showing `v` at `scale`, as laid out (points).
fn frame_of(v: &View, scale: f32) -> egui::Rect {
    overmark_core::i18n::init("en");
    let (w, h) = ((WINDOW.0 * scale) as usize, (WINDOW.1 * scale) as usize);
    render_offscreen_with_frame(v, w, h, scale)
        .1
        .expect("the bar's frame")
}

fn states() -> [View; 4] {
    [
        view(Phase::Ready, 0, false),
        view(Phase::Recording, 3725, false),
        view(Phase::Paused, 3725, false),
        view(Phase::Recording, 3725, true),
    ]
}

const MONITOR: ScreenRect = ScreenRect::new(0, 0, 1920, 1080);

/// Where the visible bar lands for a box, in every state, at 100 and 150 %.
fn placed(area: ScreenRect, scale: f32) -> Vec<(View, ScreenRect)> {
    states()
        .into_iter()
        .map(|v| {
            let frame = frame_of(&v, scale);
            let window = bar_rect(area, MONITOR, scale, frame);
            (v, frame_on_screen(window, scale, frame))
        })
        .collect()
}

#[test]
fn the_visible_bar_sits_just_above_a_box_with_room_at_its_left_edge() {
    let area = ScreenRect::new(300, 400, 900, 800);
    for scale in [1.0, 1.5] {
        for (v, bar) in placed(area, scale) {
            assert_eq!(bar.left, area.left, "{scale} {v:?}: {bar:?}");
            assert!(
                bar.bottom <= area.top && bar.bottom >= area.top - (12.0 * scale) as i32,
                "{scale} {v:?}: {bar:?}"
            );
        }
    }
}

#[test]
fn a_full_screen_box_has_the_visible_bar_just_inside_its_top_edge() {
    for scale in [1.0, 1.5] {
        for (v, bar) in placed(MONITOR, scale) {
            assert_eq!(bar.left, 0, "{scale} {v:?}: {bar:?}");
            assert!(
                bar.top > 0 && bar.top <= (12.0 * scale) as i32,
                "{scale} {v:?}: {bar:?}"
            );
        }
    }
}

/// A narrow box at the monitor's right edge: the visible bar moves left only as far as keeps it
/// on the monitor, so it's over the box's top edge, never beside it.
#[test]
fn a_narrow_box_at_the_right_edge_keeps_the_visible_bar_over_it() {
    let area = ScreenRect::new(1850, 500, 1900, 560);
    for scale in [1.0, 1.5] {
        for (v, bar) in placed(area, scale) {
            assert_eq!(bar.right, MONITOR.right, "{scale} {v:?}: {bar:?}");
            assert!(
                bar.left <= area.left && bar.right >= area.right,
                "{scale} {v:?}: the bar {bar:?} is over the box {area:?}"
            );
            assert!(bar.bottom <= area.top, "{scale} {v:?}: {bar:?}");
        }
    }
    // A narrow box with room to its right: the bar starts at its left edge and overhangs.
    let area = ScreenRect::new(500, 500, 560, 560);
    for (v, bar) in placed(area, 1.0) {
        assert_eq!(bar.left, area.left, "{v:?}: {bar:?}");
        assert!(bar.right > area.right, "{v:?}: it overhangs");
    }
}
