use super::*;
use crate::geometry::{ARROW_HEAD_LENGTH, arrow};
use crate::render::{self, FADE_MS, RenderOptions, opacity};

fn at(x: f64, y: f64, t: u64) -> InkPoint {
    InkPoint::mouse(x, y, t)
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64), shift: bool) -> Option<ItemId> {
    s.pointer_down(at(from.0, from.1, 0), shift).unwrap();
    s.pointer_move(at((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, 8), shift);
    s.pointer_up(at(to.0, to.1, 16), shift);
    s.drawing().items().last().map(|i| i.id)
}

fn last_points(s: &Session) -> Vec<Point> {
    s.drawing()
        .items()
        .last()
        .unwrap()
        .points
        .iter()
        .map(InkPoint::position)
        .collect()
}

#[test]
fn shapes_are_one_step_each_and_shift_constrains_them() {
    let mut s = Session::new();
    s.tool = ToolKind::Rectangle;
    drag(&mut s, (10.0, 10.0), (50.0, 30.0), true);
    assert_eq!(
        last_points(&s),
        [Point::new(10.0, 10.0), Point::new(50.0, 50.0)]
    );
    s.tool = ToolKind::Ellipse;
    drag(&mut s, (10.0, 10.0), (50.0, 30.0), false);
    assert_eq!(last_points(&s)[1], Point::new(50.0, 30.0));
    s.tool = ToolKind::Line;
    drag(&mut s, (0.0, 0.0), (100.0, 8.0), true);
    assert_eq!(last_points(&s)[1].y, 0.0, "snapped flat");
    s.tool = ToolKind::Arrow;
    drag(&mut s, (0.0, 100.0), (60.0, 160.0), false);
    assert_eq!(s.drawing().items().len(), 4);
    assert_eq!(s.undo_steps(), 4);
    for item in s.drawing().items() {
        assert!(item.finished);
        assert!(
            s.drawing().bounds(item.id).is_some(),
            "{:?} draws",
            item.tool
        );
    }
    s.undo();
    assert_eq!(s.drawing().items().len(), 3);
}

#[test]
fn an_arrow_item_has_the_arrow_geometry() {
    let mut s = Session::new();
    s.tool = ToolKind::Arrow;
    s.width = 6.0;
    let id = drag(&mut s, (0.0, 0.0), (200.0, 0.0), false).unwrap();
    let bounds = s.drawing().bounds(id).unwrap();
    let polygon = arrow(Point::new(0.0, 0.0), Point::new(200.0, 0.0), 6.0);
    // The head is ARROW_HEAD_LENGTH widths long, and the bounds hold it.
    assert_eq!(200.0 - polygon[2].x, 6.0 * ARROW_HEAD_LENGTH);
    assert!(bounds.contains(polygon[2]) && bounds.contains(polygon[4]));
}

#[test]
fn blur_needs_a_frozen_screen() {
    let mut s = Session::new();
    s.tool = ToolKind::Blur;
    let refused = s.pointer_down(at(10.0, 10.0, 0), false);
    assert_eq!(refused, Err(ToolError::BlurNeedsFrozen));
    assert!(refused.unwrap_err().to_string().contains("frozen"));
    assert!(s.drawing().is_empty());
    s.backdrop = crate::backdrop::Backdrop::Frozen;
    drag(&mut s, (10.0, 10.0), (90.0, 60.0), false);
    assert_eq!(s.drawing().items().len(), 1);
    // A click (no area) leaves nothing and is no step.
    s.pointer_down(at(5.0, 5.0, 0), false).unwrap();
    s.pointer_up(at(5.0, 5.0, 0), false);
    assert_eq!(s.drawing().items().len(), 1);
    assert_eq!(s.undo_steps(), 1);
}

#[test]
fn typing_text_edits_at_the_caret_and_finishes_as_one_step() {
    crate::text::use_test_font();
    let mut s = Session::new();
    s.tool = ToolKind::Text;
    s.width = 20.0;
    s.pointer_down(at(30.0, 40.0, 0), false).unwrap();
    s.pointer_up(at(30.0, 40.0, 0), false);
    let id = s.editing_text().unwrap();
    s.text_insert("Helo");
    s.text_left();
    s.text_insert("l");
    let caret_mid = s.text_caret().unwrap();
    s.text_end_of_line();
    let caret_end = s.text_caret().unwrap();
    assert!(caret_end.x0 > caret_mid.x0);
    s.text_newline();
    s.text_insert("world!");
    s.text_backspace();
    s.text_home();
    s.text_delete();
    let text = s.drawing().item(id).unwrap().text.clone().unwrap();
    assert_eq!(text.text, "Hello\norld");
    assert_eq!(text.caret, 6);
    let caret_line_two = s.text_caret().unwrap();
    assert!(caret_line_two.y0 > caret_mid.y0, "the second line is lower");
    assert!(caret_line_two.x0 >= 30.0 - 1.0 && caret_line_two.x0 < caret_mid.x0);
    assert_eq!(s.undo_steps(), 0, "nothing is a step until it's finished");
    // A click elsewhere finishes it and starts the next.
    s.pointer_down(at(200.0, 40.0, 50), false).unwrap();
    assert_eq!(s.undo_steps(), 1);
    assert!(s.drawing().bounds(id).is_some());
    assert_ne!(s.editing_text(), Some(id));
    // The new one, left empty, is dropped.
    s.text_end(60);
    assert_eq!(s.drawing().items().len(), 1);
    assert_eq!(s.undo_steps(), 1);
    s.undo();
    assert!(s.drawing().is_empty());
}

#[test]
fn stamps_through_the_pointer_count_up() {
    let mut s = Session::new();
    s.tool = ToolKind::Stamp;
    for x in [10.0, 50.0, 90.0] {
        s.pointer_down(at(x, 10.0, 0), false).unwrap();
        s.pointer_up(at(x, 10.0, 0), false);
    }
    let numbers: Vec<_> = s.drawing().items().filter_map(|i| i.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
    assert_eq!(s.undo_steps(), 3);
}

#[test]
fn the_laser_leaves_nothing_behind() {
    let mut s = Session::new();
    s.tool = ToolKind::Laser;
    s.pointer_down(at(10.0, 10.0, 0), false).unwrap();
    for i in 1..20u32 {
        s.pointer_move(
            at(10.0 + f64::from(i) * 5.0, 10.0, u64::from(i) * 10),
            false,
        );
    }
    s.pointer_up(at(110.0, 10.0, 200), false);
    assert!(s.drawing().is_empty());
    assert_eq!(s.undo_steps(), 0);
    assert_eq!(s.laser().dot(), Some(Point::new(105.0, 10.0)));
    assert!(!s.laser().visible_trail(200, 300).is_empty());
    s.laser_hide();
    assert!(!s.laser().is_visible(5000, 300));
}

#[test]
fn fading_starts_after_the_delay_and_takes_fade_ms() {
    let mut s = Session::new();
    s.stroke_begin(at(0.0, 0.0, 1000));
    s.stroke_extend(at(50.0, 0.0, 1100));
    // Not finished: never fades.
    let item = s.drawing().items().next().unwrap().clone();
    assert_eq!(opacity(&item, 1_000_000, Some(3000)), 1.0);
    s.stroke_end();
    let item = s.drawing().items().next().unwrap().clone();
    assert_eq!(opacity(&item, 4000, Some(3000)), 1.0);
    assert_eq!(
        opacity(&item, 4100, Some(3000)),
        1.0,
        "drawn at its last point's time"
    );
    let half = opacity(&item, 4100 + FADE_MS / 2, Some(3000));
    assert!((half - 0.5).abs() < 0.01, "{half}");
    assert_eq!(opacity(&item, 4100 + FADE_MS, Some(3000)), 0.0);
    assert_eq!(opacity(&item, 99_999, None), 1.0, "off: never fades");
}

#[test]
fn faded_items_go_and_undo_restores_the_last_visible_change() {
    let mut s = Session::new();
    let delay = 1000;
    // a, drawn at 0; b, drawn at 2000.
    s.stroke_begin(at(0.0, 0.0, 0));
    s.stroke_extend(at(50.0, 0.0, 0));
    s.stroke_end();
    s.stroke_begin(at(0.0, 50.0, 2000));
    s.stroke_extend(at(50.0, 50.0, 2000));
    s.stroke_end();
    let b = s.drawing().items().last().unwrap().id;
    // At 1700, a has faded (1000 + 600), b hasn't.
    let options = RenderOptions {
        now_ms: 1500,
        disappearing_ink_ms: Some(delay),
        ..RenderOptions::default()
    };
    assert!(
        render::animated_area(s.drawing(), &options).is_some(),
        "a is fading"
    );
    assert!(s.prune_faded(1700, delay).is_some());
    assert_eq!(s.drawing().items().map(|i| i.id).collect::<Vec<_>>(), [b]);
    assert_eq!(s.undo_steps(), 1, "a's step went with it");
    s.undo();
    assert!(
        s.drawing().is_empty(),
        "undo took b, the last visible change"
    );
    assert!(!s.can_undo());
    // b, undone, then faded too: redo doesn't bring it back.
    s.prune_faded(10_000, delay);
    assert!(!s.can_redo());
}

#[test]
fn an_erase_of_faded_items_leaves_the_history() {
    let mut s = Session::new();
    s.stroke_begin(at(0.0, 0.0, 0));
    s.stroke_extend(at(50.0, 0.0, 0));
    s.stroke_end();
    s.erase_begin(Point::new(25.0, 0.0));
    s.erase_end();
    assert_eq!(s.undo_steps(), 2);
    s.prune_faded(5000, 1000);
    // The erased stroke would have faded: undoing the erase would show nothing.
    assert_eq!(s.undo_steps(), 0);
}

#[test]
fn clear_session_resets_frozen_and_the_laser() {
    let mut s = Session::new();
    s.backdrop = crate::backdrop::Backdrop::Frozen;
    s.tool = ToolKind::Laser;
    s.pointer_down(at(1.0, 1.0, 0), false).unwrap();
    s.clear_session();
    assert_eq!(s.backdrop, crate::backdrop::Backdrop::Live);
    assert!(s.laser().dot().is_none());
    assert_eq!(s.tool, ToolKind::Laser);
}

/// A line from `from` to `to` drawn at `t`.
fn line_at(s: &mut Session, from: (f64, f64), to: (f64, f64), t: u64) -> ItemId {
    s.tool = ToolKind::Line;
    s.pointer_down(at(from.0, from.1, t), false).unwrap();
    s.pointer_up(at(to.0, to.1, t), false);
    s.drawing().items().last().unwrap().id
}

/// Undoes until nothing's left, checking each press does something.
fn undo_all(s: &mut Session) -> usize {
    let mut presses = 0;
    while s.can_undo() {
        let before = (s.undo_steps(), s.redo_steps());
        s.undo();
        assert_ne!((s.undo_steps(), s.redo_steps()), before);
        presses += 1;
        assert!(presses < 1000);
    }
    presses
}

/// A recoloured, then erased; B drawn later. Once A has faded, its recolour
/// goes with it, and undo takes B and then stops.
#[test]
fn a_faded_items_recolour_goes_even_when_the_history_holds_it() {
    let mut s = Session::new();
    let a = line_at(&mut s, (10.0, 20.0), (60.0, 20.0), 0);
    s.recolour(a, DEFAULT_PALETTE[4]);
    s.tool = ToolKind::Eraser;
    s.pointer_down(at(30.0, 20.0, 0), false).unwrap();
    s.pointer_up(at(30.0, 20.0, 0), false);
    let b = line_at(&mut s, (10.0, 80.0), (60.0, 80.0), 2000);
    s.prune_faded(1700, 1000);
    assert_eq!(s.undo_steps(), 1, "only B's step is left");
    assert!(s.undo().is_some());
    assert!(s.drawing().item(b).is_none());
    assert!(!s.can_undo(), "nothing left that would do anything");
    assert!(s.undo().is_none());
}

#[test]
fn a_faded_items_steps_go_when_a_clear_holds_it() {
    let mut s = Session::new();
    let a = line_at(&mut s, (10.0, 20.0), (60.0, 20.0), 0);
    s.recolour(a, DEFAULT_PALETTE[4]);
    let b = line_at(&mut s, (10.0, 50.0), (60.0, 50.0), 2000);
    s.clear();
    let c = line_at(&mut s, (10.0, 80.0), (60.0, 80.0), 2000);
    // At 1700 A has faded (in the Clear's hands); B and C haven't.
    s.prune_faded(1700, 1000);
    // C's add, the Clear (now of B alone) and B's add are left.
    assert_eq!(undo_all(&mut s), 3);
    assert!(s.drawing().is_empty());
    while s.can_redo() {
        let before = s.redo_steps();
        s.redo();
        assert!(s.redo_steps() < before);
    }
    assert_eq!(
        s.drawing().items().map(|i| i.id).collect::<Vec<_>>(),
        [c],
        "redone: B, the Clear, then C"
    );
    assert!(s.drawing().item(b).is_none());
}

#[test]
fn redo_steps_about_faded_items_go_too() {
    let mut s = Session::new();
    let a = line_at(&mut s, (10.0, 20.0), (60.0, 20.0), 0);
    let b = line_at(&mut s, (10.0, 50.0), (60.0, 50.0), 2000);
    s.recolour(a, DEFAULT_PALETTE[4]);
    s.recolour(b, DEFAULT_PALETTE[4]);
    // Undo both recolours and B's add: redo holds B, and two recolours.
    s.undo();
    s.undo();
    s.undo();
    assert_eq!(s.redo_steps(), 3);
    // A fades in the drawing; B, held by redo, doesn't yet.
    s.prune_faded(1700, 1000);
    assert!(s.drawing().is_empty());
    assert_eq!(
        s.redo_steps(),
        2,
        "A's recolour went; B's add and recolour stay"
    );
    s.redo();
    s.redo();
    assert_eq!(s.drawing().item(b).unwrap().colour, DEFAULT_PALETTE[4]);
    // Later, B fades in the drawing too, and A's add is long gone.
    s.prune_faded(10_000, 1000);
    assert!(!s.can_undo() && !s.can_redo());
}

/// A, then Clear, then B; B and the Clear undone. Once A has faded, the
/// Clear would clear nothing when its turn comes (B is only redone after it), so it goes: one
/// redo press brings B back.
#[test]
fn a_redo_clear_that_would_clear_nothing_at_its_turn_goes() {
    let mut s = Session::new();
    line_at(&mut s, (10.0, 20.0), (60.0, 20.0), 0);
    s.clear();
    let b = line_at(&mut s, (10.0, 80.0), (60.0, 80.0), 2000);
    s.undo();
    s.undo();
    s.prune_faded(1700, 1000);
    assert!(s.drawing().is_empty());
    assert_eq!(s.redo_steps(), 1, "only B's add is left");
    assert!(s.redo().is_some());
    assert!(s.drawing().item(b).is_some());
    assert!(!s.can_redo());
}

/// Every press still does something, whatever order steps were taken and undone in.
#[test]
fn after_pruning_every_undo_and_redo_press_changes_something() {
    let mut s = Session::new();
    let a = line_at(&mut s, (10.0, 20.0), (60.0, 20.0), 0);
    let b = line_at(&mut s, (10.0, 40.0), (60.0, 40.0), 2000);
    s.recolour(a, DEFAULT_PALETTE[4]);
    s.clear();
    let c = line_at(&mut s, (10.0, 60.0), (60.0, 60.0), 0);
    s.recolour(c, DEFAULT_PALETTE[2]);
    s.tool = ToolKind::Eraser;
    s.pointer_down(at(30.0, 60.0, 0), false).unwrap();
    s.pointer_up(at(30.0, 60.0, 0), false);
    line_at(&mut s, (10.0, 90.0), (60.0, 90.0), 2000);
    for _ in 0..3 {
        s.undo();
    }
    // A and C fade (in the Clear's, the erase's and the redo side's hands); B and D don't.
    s.prune_faded(1700, 1000);
    let shown = |s: &Session| {
        s.drawing()
            .items()
            .map(|i| (i.id, i.colour))
            .collect::<Vec<_>>()
    };
    while s.can_redo() {
        let before = shown(&s);
        s.redo();
        assert_ne!(shown(&s), before, "a redo press changed nothing");
    }
    while s.can_undo() {
        let before = shown(&s);
        s.undo();
        assert_ne!(shown(&s), before, "an undo press changed nothing");
    }
    assert!(s.drawing().item(b).is_none());
}

#[test]
fn blur_is_refused_on_every_backdrop_but_frozen() {
    use crate::backdrop::Backdrop;
    for backdrop in Backdrop::ALL {
        let mut s = Session::new();
        s.backdrop = backdrop;
        s.tool = ToolKind::Blur;
        let result = s.pointer_down(at(10.0, 10.0, 0), false);
        if backdrop == Backdrop::Frozen {
            assert!(result.is_ok());
        } else {
            assert_eq!(result, Err(ToolError::BlurNeedsFrozen), "{backdrop:?}");
            assert!(s.drawing().is_empty());
        }
    }
}
