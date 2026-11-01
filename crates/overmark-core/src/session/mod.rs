//! A drawing session: what one opening of the layer holds, with undo and redo.
//!
//! The session owns the [`Drawing`], the current tool, colour and width, the numbered-stamp
//! counter, and the history. **Every change to the drawing is undoable**: adding a stroke or a
//! stamp, a shape or text, erasing, Clear and recolouring an item.
//! A new change after an undo drops what could have been redone. The history keeps the last
//! [`HISTORY_STEPS`] changes (at least 100), for as long as the layer is open; closing
//! it calls [`Session::clear_session`], and nothing is saved.
//!
//! **Memory:** the history holds changes, not copies of the drawing. Each item is kept once:
//! in the drawing, or, once removed (erased, cleared or undone), in the history entry that can
//! put it back. A recolour keeps only the old and new colour.
//!
//! **For redrawing**, every method that changes the drawing returns the area to redraw
//! (drawing coordinates; see [`crate::render::dirty_rect`]):
//!
//! ```
//! use overmark_core::drawing::InkPoint;
//! use overmark_core::session::Session;
//! use overmark_core::tool::ToolKind;
//!
//! let mut session = Session::new();
//! session.tool = ToolKind::Pen;
//! // Pointer down, moves and up: one stroke, one undo step.
//! session.stroke_begin(InkPoint::pen(10.0, 10.0, 0.5, 0));
//! session.stroke_extend(InkPoint::pen(60.0, 30.0, 0.7, 8));
//! session.stroke_end();
//! assert_eq!(session.drawing().items().len(), 1);
//! // Ctrl+Z, Ctrl+Y.
//! session.undo();
//! assert!(session.drawing().is_empty());
//! session.redo();
//! assert_eq!(session.drawing().items().len(), 1);
//! ```

use std::collections::VecDeque;

use kurbo::{Point, Rect};

use crate::backdrop::Backdrop;
use crate::colour::Colour;
use crate::drawing::{Drawing, InkPoint, Item, ItemId};
use crate::laser::Laser;
use crate::settings::DEFAULT_PALETTE;
use crate::tool::ToolKind;

mod tools;
pub use tools::ToolError;

/// How many changes can be undone (at least 100).
pub const HISTORY_STEPS: usize = 100;

/// The eraser's reach beyond the pointer, in physical pixels.
pub const ERASER_TOLERANCE: f64 = 6.0;

/// One change, as the history keeps it: enough to undo it, or (on the redo side) to do it
/// again. Items that left the drawing live here until they're put back or forgotten; items in
/// the drawing are only named by id.
#[derive(Debug, Clone)]
enum Change {
    /// An item was added. While it's in the drawing, only its id is kept; once undone, the item
    /// itself (`item`) and where it was. `stamp_before` is the stamp counter before it.
    Add {
        id: ItemId,
        index: usize,
        item: Option<Item>,
        stamp_before: u32,
    },
    /// Items were erased. Done: `removed` holds them, each with the place it had when it was
    /// removed, in the order they were. Undone: they're back, and `ids` names them.
    Erase {
        removed: Vec<(usize, Item)>,
        ids: Vec<ItemId>,
    },
    /// Everything was cleared. Done: `items` holds them, bottom first. Undone: they're back
    /// (`items` is empty), and redoing clears again. `stamp_before` is the stamp counter before
    /// the Clear.
    Clear { items: Vec<Item>, stamp_before: u32 },
    /// An item's colour changed.
    Recolour {
        id: ItemId,
        from: Colour,
        to: Colour,
    },
}

/// What one opening of the layer holds.
#[derive(Debug, Clone)]
pub struct Session {
    drawing: Drawing,
    /// The tool the next stroke uses.
    pub tool: ToolKind,
    /// The colour the next item gets.
    pub colour: Colour,
    /// The width the next item gets, in physical pixels.
    pub width: f32,
    undo: VecDeque<Change>,
    redo: Vec<Change>,
    /// The number the next stamp shows.
    next_stamp: u32,
    /// What's under the ink (Live, Frozen, Whiteboard, Blackboard): Blur needs the frozen
    /// frame.
    pub backdrop: Backdrop,
    /// The stroke or shape being drawn, or the text being typed.
    active: Option<ItemId>,
    /// Where the shape being drawn started.
    shape_from: Option<Point>,
    /// The laser pointer's recent positions (never part of the drawing).
    laser: Laser,
    /// The eraser drag under way: the items it removed so far, and where the pointer was.
    erasing: Option<(Vec<(usize, Item)>, Point)>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            drawing: Drawing::new(),
            tool: ToolKind::Pen,
            colour: DEFAULT_PALETTE[0],
            width: 4.0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            next_stamp: 1,
            backdrop: Backdrop::Live,
            active: None,
            shape_from: None,
            laser: Laser::new(),
            erasing: None,
        }
    }
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// What's drawn, for rendering.
    pub fn drawing(&self) -> &Drawing {
        &self.drawing
    }

    /// The number the next stamp will show.
    pub fn next_stamp(&self) -> u32 {
        self.next_stamp
    }

    /// Whether a stroke is being drawn.
    pub fn is_drawing(&self) -> bool {
        self.active.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.active.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many changes can be undone now.
    pub fn undo_steps(&self) -> usize {
        self.undo.len()
    }

    /// How many undone changes can be redone now.
    pub fn redo_steps(&self) -> usize {
        self.redo.len()
    }

    /// Closing the layer: the drawing, its history and the stamp counter go. The tool, colour
    /// and width stay for the next opening.
    pub fn clear_session(&mut self) {
        *self = Session {
            tool: self.tool,
            colour: self.colour,
            width: self.width,
            ..Session::default()
        };
    }

    // ---- Strokes ----

    /// Pointer down with a drawing tool: starts a stroke with the current tool, colour and
    /// width. A stroke already under way is finished first.
    pub fn stroke_begin(&mut self, point: InkPoint) -> Option<Rect> {
        let finished = self.stroke_end();
        let (id, area) = self
            .drawing
            .begin(self.tool, self.colour, self.width, point);
        self.active = Some(id);
        union(finished, area)
    }

    /// Pointer move while drawing.
    pub fn stroke_extend(&mut self, point: InkPoint) -> Option<Rect> {
        let id = self.active?;
        self.drawing.push_point(id, point)
    }

    /// Pointer up: the stroke is finished and becomes one undo step. (It also finishes a
    /// shape, or text being typed; text with nothing typed, or a blur with no area, is dropped
    /// with no step.)
    pub fn stroke_end(&mut self) -> Option<Rect> {
        let id = self.active.take()?;
        self.shape_from = None;
        let empty = self.drawing.item(id).is_some_and(|item| match item.tool {
            ToolKind::Text => item.text.as_ref().is_none_or(|t| t.text.trim().is_empty()),
            ToolKind::Blur => self.drawing.bounds(id).is_none(),
            _ => false,
        });
        if empty {
            return self.drawing.remove(id).and_then(|(_, _, area)| area);
        }
        let area = self.drawing.finish(id);
        let index = self.drawing.index(id).unwrap_or(0);
        self.record(Change::Add {
            id,
            index,
            item: None,
            stamp_before: self.next_stamp,
        });
        area
    }

    /// Drops the stroke being drawn without keeping it (the pointer was cancelled).
    pub fn stroke_cancel(&mut self) -> Option<Rect> {
        let id = self.active.take()?;
        self.shape_from = None;
        self.drawing.remove(id).and_then(|(_, _, area)| area)
    }

    // ---- Stamps ----

    /// Places the next numbered stamp at `at`, with the current colour and width. Returns its
    /// number and the area to redraw. Undoing it gives the number back.
    pub fn place_stamp(&mut self, at: InkPoint) -> (u32, Option<Rect>) {
        let finished = self.stroke_end();
        let number = self.next_stamp;
        let (id, area) = self.drawing.add(
            ToolKind::Stamp,
            self.colour,
            self.width,
            vec![at],
            Some(number),
        );
        let index = self.drawing.index(id).unwrap_or(0);
        self.next_stamp = number.saturating_add(1);
        self.record(Change::Add {
            id,
            index,
            item: None,
            stamp_before: number,
        });
        (number, union(finished, area))
    }

    /// Adds a finished item made whole at once (for shapes and text): one
    /// undo step. Returns its id and the area to redraw.
    pub fn add_item(
        &mut self,
        tool: ToolKind,
        colour: Colour,
        width: f32,
        points: Vec<InkPoint>,
    ) -> (ItemId, Option<Rect>) {
        let finished = self.stroke_end();
        let (id, area) = self.drawing.add(tool, colour, width, points, None);
        let index = self.drawing.index(id).unwrap_or(0);
        self.record(Change::Add {
            id,
            index,
            item: None,
            stamp_before: self.next_stamp,
        });
        (id, union(finished, area))
    }

    // ---- The eraser ----

    /// Eraser down: removes the items under `at`. The whole drag, to [`Session::erase_end`],
    /// is one undo step.
    pub fn erase_begin(&mut self, at: Point) -> Option<Rect> {
        let finished = self.stroke_end();
        let ended = self.erase_end();
        self.erasing = Some((Vec::new(), at));
        let area = self.erase_to(at);
        union(union(finished, ended), area)
    }

    /// Eraser move: removes the items the drag from the last point to `at` touches.
    pub fn erase_to(&mut self, at: Point) -> Option<Rect> {
        let (_, from) = self.erasing.as_ref()?;
        let from = *from;
        let hits = self.drawing.hit_test(from, at, ERASER_TOLERANCE);
        let mut area = None;
        let (removed, last) = self.erasing.as_mut()?;
        for id in hits {
            if let Some((item, index, bounds)) = self.drawing.remove(id) {
                removed.push((index, item));
                area = union(area, bounds);
            }
        }
        *last = at;
        area
    }

    /// Eraser up: what the drag removed becomes one undo step (none if it removed nothing).
    pub fn erase_end(&mut self) -> Option<Rect> {
        let (removed, _) = self.erasing.take()?;
        if !removed.is_empty() {
            self.record(Change::Erase {
                removed,
                ids: Vec::new(),
            });
        }
        None
    }

    // ---- Clear and recolour ----

    /// Clear (Delete): removes everything, as one undo step. The stamp counter starts again at 1;
    /// undoing the Clear brings it back.
    pub fn clear(&mut self) -> Option<Rect> {
        self.stroke_end();
        self.erase_end();
        if self.drawing.is_empty() {
            return None;
        }
        let (items, area) = self.drawing.clear();
        let stamp_before = std::mem::replace(&mut self.next_stamp, 1);
        self.record(Change::Clear {
            items,
            stamp_before,
        });
        area
    }

    /// Changes an item's colour, as one undo step. `None` if there's no such item or it's
    /// already that colour.
    pub fn recolour(&mut self, id: ItemId, colour: Colour) -> Option<Rect> {
        if self.drawing.item(id)?.colour == colour {
            return None;
        }
        let (from, area) = self.drawing.set_colour(id, colour)?;
        self.record(Change::Recolour {
            id,
            from,
            to: colour,
        });
        area
    }

    /// The items a point or short drag touches, top first, within `tolerance` pixels: for
    /// picking an item to recolour, say.
    pub fn hit_test(&self, from: Point, to: Point, tolerance: f64) -> Vec<ItemId> {
        self.drawing.hit_test(from, to, tolerance)
    }

    // ---- Undo and redo ----

    /// Undoes the last change (Ctrl+Z). A stroke still being drawn is dropped instead.
    pub fn undo(&mut self) -> Option<Rect> {
        if self.active.is_some() {
            return self.stroke_cancel();
        }
        self.erase_end();
        let change = self.undo.pop_back()?;
        let (change, area) = self.revert(change);
        self.redo.push(change);
        area
    }

    /// Redoes the last undone change (Ctrl+Y).
    pub fn redo(&mut self) -> Option<Rect> {
        if self.active.is_some() || self.erasing.is_some() {
            return None;
        }
        let change = self.redo.pop()?;
        let (change, area) = self.apply(change);
        self.undo.push_back(change);
        area
    }

    /// Records a new change: what could be redone is dropped, and the oldest change is
    /// forgotten beyond [`HISTORY_STEPS`].
    fn record(&mut self, change: Change) {
        self.redo.clear();
        self.undo.push_back(change);
        while self.undo.len() > HISTORY_STEPS {
            self.undo.pop_front();
        }
    }

    /// Undoes `change`; returns what redoes it, and the area to redraw.
    fn revert(&mut self, change: Change) -> (Change, Option<Rect>) {
        match change {
            Change::Add {
                id, stamp_before, ..
            } => {
                let (item, index, area) = match self.drawing.remove(id) {
                    Some((item, index, area)) => (Some(item), index, area),
                    None => (None, 0, None),
                };
                if item.as_ref().is_some_and(|i| i.number.is_some()) {
                    self.next_stamp = stamp_before;
                }
                let redo = Change::Add {
                    id,
                    index,
                    item,
                    stamp_before,
                };
                (redo, area)
            }
            Change::Erase { removed, .. } => {
                // Put back in reverse order, each where it was when it was taken out.
                let mut area = None;
                let ids = removed.iter().map(|(_, item)| item.id).collect();
                for (index, item) in removed.into_iter().rev() {
                    area = union(area, self.drawing.insert(index, item));
                }
                let redo = Change::Erase {
                    removed: Vec::new(),
                    ids,
                };
                (redo, area)
            }
            Change::Clear {
                items,
                stamp_before,
                ..
            } => {
                let mut area = None;
                for (index, item) in items.into_iter().enumerate() {
                    area = union(area, self.drawing.insert(index, item));
                }
                self.next_stamp = stamp_before;
                let redo = Change::Clear {
                    items: Vec::new(),
                    stamp_before,
                };
                (redo, area)
            }
            Change::Recolour { id, from, to } => {
                let area = self.drawing.set_colour(id, from).and_then(|(_, a)| a);
                (Change::Recolour { id, from, to }, area)
            }
        }
    }

    /// Redoes `change`, as [`Session::revert`] returned it; returns what undoes it again.
    fn apply(&mut self, change: Change) -> (Change, Option<Rect>) {
        match change {
            Change::Add {
                id,
                index,
                item,
                stamp_before,
            } => {
                let mut area = None;
                if let Some(item) = item {
                    if let Some(n) = item.number {
                        self.next_stamp = n.saturating_add(1);
                    }
                    area = self.drawing.insert(index, item);
                }
                let undo = Change::Add {
                    id,
                    index,
                    item: None,
                    stamp_before,
                };
                (undo, area)
            }
            Change::Erase { ids, .. } => {
                let mut area = None;
                let mut removed = Vec::new();
                for id in ids {
                    if let Some((item, index, bounds)) = self.drawing.remove(id) {
                        removed.push((index, item));
                        area = union(area, bounds);
                    }
                }
                let undo = Change::Erase {
                    removed,
                    ids: Vec::new(),
                };
                (undo, area)
            }
            Change::Clear { stamp_before, .. } => {
                let (items, area) = self.drawing.clear();
                self.next_stamp = 1;
                let undo = Change::Clear {
                    items,
                    stamp_before,
                };
                (undo, area)
            }
            Change::Recolour { id, from, to } => {
                let area = self.drawing.set_colour(id, to).and_then(|(_, a)| a);
                (Change::Recolour { id, from, to }, area)
            }
        }
    }
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_tests;
