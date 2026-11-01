use super::*;

/// Draws a short horizontal stroke at height `y` and returns its id.
fn line(s: &mut Session, y: f64) -> ItemId {
    s.stroke_begin(InkPoint::mouse(10.0, y, 0));
    for i in 1u32..6 {
        s.stroke_extend(InkPoint::mouse(
            10.0 + f64::from(i) * 10.0,
            y,
            u64::from(i) * 8,
        ));
    }
    s.stroke_end();
    s.drawing().items().last().unwrap().id
}

fn ids(s: &Session) -> Vec<ItemId> {
    s.drawing().items().map(|i| i.id).collect()
}

#[test]
fn a_stroke_is_one_step_and_undo_redo_restore_it() {
    let mut s = Session::new();
    let a = line(&mut s, 20.0);
    assert_eq!(s.undo_steps(), 1);
    let before = s.drawing().item(a).unwrap().clone();
    assert!(s.undo().is_some());
    assert!(s.drawing().is_empty());
    assert!(s.redo().is_some());
    assert_eq!(s.drawing().item(a), Some(&before));
    assert!(!s.can_redo());
}

#[test]
fn the_history_keeps_the_last_hundred_of_150_steps() {
    const { assert!(HISTORY_STEPS >= 100) };
    let mut s = Session::new();
    for k in 0..150 {
        line(&mut s, f64::from(k) * 3.0);
    }
    assert_eq!(s.drawing().items().len(), 150);
    assert_eq!(s.undo_steps(), HISTORY_STEPS);
    let mut undone = 0;
    while s.undo().is_some() {
        undone += 1;
    }
    assert_eq!(undone, HISTORY_STEPS);
    assert_eq!(s.drawing().items().len(), 150 - HISTORY_STEPS);
    assert!(!s.can_undo());
    // And all of it redoes.
    while s.redo().is_some() {}
    assert_eq!(s.drawing().items().len(), 150);
}

#[test]
fn redo_after_undo_and_a_new_change_drops_it() {
    let mut s = Session::new();
    let a = line(&mut s, 10.0);
    let b = line(&mut s, 30.0);
    s.undo();
    s.undo();
    assert_eq!(s.redo_steps(), 2);
    s.redo();
    assert_eq!(ids(&s), [a]);
    // A new stroke drops the redo branch.
    let c = line(&mut s, 50.0);
    assert!(!s.can_redo());
    assert!(s.redo().is_none());
    assert_eq!(ids(&s), [a, c]);
    assert!(s.drawing().item(b).is_none());
}

#[test]
fn undoing_a_clear_brings_everything_back_in_order() {
    let mut s = Session::new();
    let a = line(&mut s, 10.0);
    let b = line(&mut s, 30.0);
    let c = line(&mut s, 50.0);
    assert!(s.clear().is_some());
    assert!(s.drawing().is_empty());
    assert!(s.clear().is_none(), "clearing nothing isn't a step");
    assert_eq!(s.undo_steps(), 4);
    s.undo();
    assert_eq!(ids(&s), [a, b, c]);
    s.redo();
    assert!(s.drawing().is_empty());
    s.undo();
    assert_eq!(ids(&s), [a, b, c]);
}

#[test]
fn undoing_an_erase_puts_items_back_where_they_were() {
    let mut s = Session::new();
    let a = line(&mut s, 10.0);
    let b = line(&mut s, 30.0);
    let c = line(&mut s, 50.0);
    let d = line(&mut s, 70.0);
    // One drag down through b and c (at x = 30), then off to the right between c and d: one
    // step.
    s.erase_begin(Point::new(30.0, 25.0));
    s.erase_to(Point::new(30.0, 55.0));
    s.erase_to(Point::new(300.0, 58.0));
    s.erase_end();
    assert_eq!(ids(&s), [a, d]);
    assert_eq!(s.undo_steps(), 5);
    s.undo();
    assert_eq!(ids(&s), [a, b, c, d]);
    s.redo();
    assert_eq!(ids(&s), [a, d]);
    s.undo();
    assert_eq!(ids(&s), [a, b, c, d]);
}

#[test]
fn an_eraser_drag_that_misses_is_no_step() {
    let mut s = Session::new();
    line(&mut s, 10.0);
    s.erase_begin(Point::new(500.0, 500.0));
    s.erase_to(Point::new(520.0, 520.0));
    s.erase_end();
    assert_eq!(s.undo_steps(), 1);
    assert_eq!(s.drawing().items().len(), 1);
}

#[test]
fn the_eraser_reaches_a_little_past_the_ink() {
    let mut s = Session::new();
    s.width = 4.0;
    line(&mut s, 100.0);
    // At most 2 px of ink each side of y = 100; the eraser reaches ERASER_TOLERANCE past it.
    s.erase_begin(Point::new(30.0, 100.0 + 2.0 + ERASER_TOLERANCE + 3.0));
    s.erase_end();
    assert_eq!(s.drawing().items().len(), 1);
    s.erase_begin(Point::new(30.0, 100.0 + ERASER_TOLERANCE - 1.0));
    s.erase_end();
    assert!(s.drawing().is_empty());
}

#[test]
fn the_stamp_counter_follows_undo_redo_and_clear() {
    let mut s = Session::new();
    let at = |x| InkPoint::mouse(x, 50.0, 0);
    assert_eq!(s.place_stamp(at(10.0)).0, 1);
    assert_eq!(s.place_stamp(at(40.0)).0, 2);
    assert_eq!(s.place_stamp(at(70.0)).0, 3);
    s.undo();
    assert_eq!(s.next_stamp(), 3, "undoing stamp 3 gives the number back");
    s.undo();
    assert_eq!(s.next_stamp(), 2);
    s.redo();
    assert_eq!(s.next_stamp(), 3);
    assert_eq!(s.place_stamp(at(70.0)).0, 3);
    // A stroke between stamps doesn't move the counter when undone.
    line(&mut s, 200.0);
    s.undo();
    assert_eq!(s.next_stamp(), 4);
    // Clear starts again at 1; undoing it brings the count back.
    s.clear();
    assert_eq!(s.next_stamp(), 1);
    s.undo();
    assert_eq!(s.next_stamp(), 4);
    let numbers: Vec<_> = s.drawing().items().filter_map(|i| i.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
}

#[test]
fn recolouring_is_one_step() {
    let mut s = Session::new();
    let a = line(&mut s, 10.0);
    let old = s.drawing().item(a).unwrap().colour;
    let blue = DEFAULT_PALETTE[4];
    assert!(s.recolour(a, blue).is_some());
    assert!(
        s.recolour(a, blue).is_none(),
        "the same colour isn't a step"
    );
    assert_eq!(s.drawing().item(a).unwrap().colour, blue);
    s.undo();
    assert_eq!(s.drawing().item(a).unwrap().colour, old);
    s.redo();
    assert_eq!(s.drawing().item(a).unwrap().colour, blue);
    assert_eq!(s.undo_steps(), 2);
}

#[test]
fn undo_while_drawing_drops_the_stroke() {
    let mut s = Session::new();
    let a = line(&mut s, 10.0);
    s.stroke_begin(InkPoint::mouse(10.0, 40.0, 0));
    s.stroke_extend(InkPoint::mouse(60.0, 40.0, 8));
    assert!(s.is_drawing());
    s.undo();
    assert!(!s.is_drawing());
    assert_eq!(ids(&s), [a]);
    assert_eq!(s.undo_steps(), 1);
}

#[test]
fn clear_session_starts_clean_but_keeps_the_tool() {
    let mut s = Session::new();
    s.tool = ToolKind::Highlighter;
    s.width = 20.0;
    line(&mut s, 10.0);
    s.place_stamp(InkPoint::mouse(5.0, 5.0, 0));
    s.undo();
    s.clear_session();
    assert!(s.drawing().is_empty());
    assert!(!s.can_undo() && !s.can_redo());
    assert_eq!(s.next_stamp(), 1);
    assert_eq!((s.tool, s.width), (ToolKind::Highlighter, 20.0));
}

/// At the cap, the history holds changes, not copies: every item is in the drawing or in
/// exactly one history entry.
#[test]
fn memory_is_one_copy_of_each_item() {
    let mut s = Session::new();
    for k in 0..60 {
        line(&mut s, f64::from(k));
    }
    s.clear();
    for k in 0..60 {
        line(&mut s, f64::from(k));
    }
    for _ in 0..30 {
        s.undo();
    }
    let in_history: usize = s
        .undo
        .iter()
        .chain(&s.redo)
        .map(|c| match c {
            Change::Add { item, .. } => usize::from(item.is_some()),
            Change::Erase { removed, .. } => removed.len(),
            Change::Clear { items, .. } => items.len(),
            Change::Recolour { .. } => 0,
        })
        .sum();
    let in_drawing = s.drawing().items().len();
    // The Clear entry holds 60 items, the redo side 30 undone strokes; 30 are drawn.
    assert_eq!((in_drawing, in_history), (30, 90));
    assert!(s.undo.len() <= HISTORY_STEPS);
}

#[test]
fn hit_testing_finds_the_top_item_first() {
    let mut s = Session::new();
    s.width = 10.0;
    let a = line(&mut s, 50.0);
    let b = line(&mut s, 52.0);
    let hits = s.hit_test(Point::new(30.0, 51.0), Point::new(30.0, 51.0), 0.0);
    assert_eq!(hits, [b, a]);
    assert!(
        s.hit_test(Point::new(30.0, 90.0), Point::new(30.0, 90.0), 2.0)
            .is_empty()
    );
}

/// At zero tolerance a horizontal or vertical drag has a flat bounding box,
/// and it must still find the ink it crosses, starting outside it.
#[test]
fn flat_drags_at_zero_tolerance_cross_the_ink() {
    let mut s = Session::new();
    let a = line(&mut s, 50.0);
    // Vertical, from above to below the stroke.
    assert_eq!(
        s.hit_test(Point::new(30.0, 0.0), Point::new(30.0, 100.0), 0.0),
        [a]
    );
    // Horizontal, along it from left of its start to right of its end.
    assert_eq!(
        s.hit_test(Point::new(0.0, 50.0), Point::new(100.0, 50.0), 0.0),
        [a]
    );
    // A single point on the ink, and flat drags that miss.
    assert_eq!(
        s.hit_test(Point::new(30.0, 50.0), Point::new(30.0, 50.0), 0.0),
        [a]
    );
    assert!(
        s.hit_test(Point::new(200.0, 0.0), Point::new(200.0, 100.0), 0.0)
            .is_empty()
    );
    assert!(
        s.hit_test(Point::new(0.0, 90.0), Point::new(100.0, 90.0), 0.0)
            .is_empty()
    );
    // And the eraser, at its own tolerance, erases with a vertical swipe.
    s.erase_begin(Point::new(30.0, 0.0));
    s.erase_to(Point::new(30.0, 100.0));
    s.erase_end();
    assert!(s.drawing().is_empty());
}
