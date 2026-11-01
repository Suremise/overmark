//! The tools' side of the session: one pointer API for every tool, text editing, the laser
//! and disappearing ink.

use std::fmt;

use kurbo::Rect;

use super::{Change, Session, union};
use crate::drawing::{InkPoint, Item, ItemId, TextBox};
use crate::geometry;
use crate::laser::Laser;
use crate::render;
use crate::tool::ToolKind;
use crate::tr;

/// Why a tool can't be used now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolError {
    /// Blur needs the frozen frame under it ([`Session::backdrop`] Frozen): on Live there's
    /// nothing still to pixelate, and on a whiteboard or blackboard nothing to hide.
    BlurNeedsFrozen,
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::BlurNeedsFrozen => f.write_str(&tr!("tool-blur-needs-frozen", key = "F")),
        }
    }
}

impl std::error::Error for ToolError {}

impl Session {
    // ---- One pointer API for every tool ----

    /// Pointer down with the current tool. `shift` is the Shift key (squares, circles and 45°
    /// lines). Pen and highlighter start a stroke; arrow, rectangle, ellipse, line and blur
    /// start a shape; a stamp is placed; text starts a text item there (finishing one being
    /// typed); the eraser erases; the laser shows its dot.
    ///
    /// Refuses Blur unless [`Session::backdrop`] is Frozen.
    pub fn pointer_down(&mut self, at: InkPoint, shift: bool) -> Result<Option<Rect>, ToolError> {
        let finished = self.text_end(at.time_ms);
        let area = match self.tool {
            ToolKind::Pen | ToolKind::Highlighter => self.stroke_begin(at),
            ToolKind::Arrow
            | ToolKind::Rectangle
            | ToolKind::Ellipse
            | ToolKind::Line
            | ToolKind::Blur => self.shape_begin(at, shift)?,
            ToolKind::Stamp => self.place_stamp(at).1,
            ToolKind::Text => self.text_begin(at).1,
            ToolKind::Eraser => self.erase_begin(at.position()),
            ToolKind::Laser => {
                self.laser.move_to(at.position(), at.time_ms);
                None
            }
        };
        Ok(union(finished, area))
    }

    /// Pointer move (while down, or for the laser, while over the layer).
    pub fn pointer_move(&mut self, at: InkPoint, shift: bool) -> Option<Rect> {
        match self.tool {
            ToolKind::Pen | ToolKind::Highlighter => self.stroke_extend(at),
            ToolKind::Arrow
            | ToolKind::Rectangle
            | ToolKind::Ellipse
            | ToolKind::Line
            | ToolKind::Blur => self.shape_drag(at, shift),
            ToolKind::Eraser => self.erase_to(at.position()),
            ToolKind::Laser => {
                self.laser.move_to(at.position(), at.time_ms);
                None
            }
            ToolKind::Stamp | ToolKind::Text => None,
        }
    }

    /// Pointer up: a stroke or shape becomes one undo step; an eraser drag ends. A text item
    /// stays open for typing until [`Session::text_end`] (Enter, Esc or a click elsewhere).
    pub fn pointer_up(&mut self, at: InkPoint, shift: bool) -> Option<Rect> {
        match self.tool {
            ToolKind::Pen | ToolKind::Highlighter => {
                let moved = self.stroke_extend(at);
                union(moved, self.stroke_end())
            }
            ToolKind::Arrow
            | ToolKind::Rectangle
            | ToolKind::Ellipse
            | ToolKind::Line
            | ToolKind::Blur => {
                let moved = self.shape_drag(at, shift);
                union(moved, self.stroke_end())
            }
            ToolKind::Eraser => {
                let erased = self.erase_to(at.position());
                union(erased, self.erase_end())
            }
            ToolKind::Laser | ToolKind::Stamp | ToolKind::Text => None,
        }
    }

    // ---- Shapes ----

    /// Starts a shape of the current tool (arrow, rectangle, ellipse, line or blur) at `at`.
    pub fn shape_begin(&mut self, at: InkPoint, shift: bool) -> Result<Option<Rect>, ToolError> {
        if self.tool == ToolKind::Blur && !self.backdrop.allows_blur() {
            return Err(ToolError::BlurNeedsFrozen);
        }
        let finished = self.stroke_end();
        let (id, area) = self.drawing.begin(self.tool, self.colour, self.width, at);
        self.active = Some(id);
        self.shape_from = Some(at.position());
        let dragged = self.shape_drag(at, shift);
        Ok(union(union(finished, area), dragged))
    }

    /// Drags the shape's far corner (or end) to `to`. With `shift`, a rectangle, ellipse or
    /// blur is square and a line or arrow snaps to 45°.
    pub fn shape_drag(&mut self, to: InkPoint, shift: bool) -> Option<Rect> {
        let id = self.active?;
        let from = self.shape_from?;
        let tool = self.drawing.item(id)?.tool;
        let end = match (shift, tool) {
            (true, ToolKind::Rectangle | ToolKind::Ellipse | ToolKind::Blur) => {
                geometry::constrain(from, to.position())
            }
            (true, ToolKind::Line | ToolKind::Arrow) => geometry::snap(from, to.position()),
            _ => to.position(),
        };
        let end = InkPoint {
            x: end.x,
            y: end.y,
            ..to
        };
        self.drawing.update(id, |item| {
            item.points.truncate(1);
            item.points.push(end);
        })
    }

    // ---- Text ----

    /// Starts a text item with its top-left at `at`, in the current colour, the width as its
    /// size in pixels. Returns its id and the area to redraw.
    pub fn text_begin(&mut self, at: InkPoint) -> (ItemId, Option<Rect>) {
        let finished = self.stroke_end();
        let (id, area) = self
            .drawing
            .begin(ToolKind::Text, self.colour, self.width, at);
        self.drawing
            .update(id, |item| item.text = Some(TextBox::default()));
        self.active = Some(id);
        (id, union(finished, area))
    }

    /// The text item being typed, if any.
    pub fn editing_text(&self) -> Option<ItemId> {
        let id = self.active?;
        (self.drawing.item(id)?.tool == ToolKind::Text).then_some(id)
    }

    /// Types `s` at the caret.
    pub fn text_insert(&mut self, s: &str) -> Option<Rect> {
        self.edit_text(|t| {
            t.text.insert_str(t.caret, s);
            t.caret += s.len();
        })
    }

    /// Shift+Enter: a new line at the caret.
    pub fn text_newline(&mut self) -> Option<Rect> {
        self.text_insert("\n")
    }

    /// Backspace: deletes the character before the caret.
    pub fn text_backspace(&mut self) -> Option<Rect> {
        self.edit_text(|t| {
            if let Some(c) = t.text[..t.caret].chars().next_back() {
                t.caret -= c.len_utf8();
                t.text.remove(t.caret);
            }
        })
    }

    /// Delete: deletes the character after the caret.
    pub fn text_delete(&mut self) -> Option<Rect> {
        self.edit_text(|t| {
            if t.caret < t.text.len() {
                t.text.remove(t.caret);
            }
        })
    }

    /// Moves the caret one character left.
    pub fn text_left(&mut self) -> Option<Rect> {
        self.edit_text(|t| {
            if let Some(c) = t.text[..t.caret].chars().next_back() {
                t.caret -= c.len_utf8();
            }
        })
    }

    /// Moves the caret one character right.
    pub fn text_right(&mut self) -> Option<Rect> {
        self.edit_text(|t| {
            if let Some(c) = t.text[t.caret..].chars().next() {
                t.caret += c.len_utf8();
            }
        })
    }

    /// Home: the caret to the start of its line.
    pub fn text_home(&mut self) -> Option<Rect> {
        self.edit_text(|t| t.caret = t.text[..t.caret].rfind('\n').map_or(0, |i| i + 1))
    }

    /// End: the caret to the end of its line.
    pub fn text_end_of_line(&mut self) -> Option<Rect> {
        self.edit_text(|t| {
            t.caret += t.text[t.caret..]
                .find('\n')
                .unwrap_or(t.text.len() - t.caret);
        })
    }

    /// Where to draw the caret of the text being typed, in drawing coordinates.
    pub fn text_caret(&self) -> Option<Rect> {
        let item = self.drawing.item(self.editing_text()?)?;
        let text = item.text.as_ref()?;
        let at = item.points.first()?.position();
        let caret = crate::text::shape(&text.text, item.width).caret(text.caret);
        Some(caret + at.to_vec2())
    }

    /// Finishes the text being typed (Enter, Esc or a click elsewhere), as one undo step, at
    /// `now_ms` (when disappearing ink starts counting). Empty text is dropped, with no step.
    pub fn text_end(&mut self, now_ms: u64) -> Option<Rect> {
        let id = self.editing_text()?;
        self.drawing.update(id, |item| {
            if let Some(p) = item.points.first_mut() {
                p.time_ms = p.time_ms.max(now_ms);
            }
        });
        self.stroke_end()
    }

    fn edit_text(&mut self, change: impl FnOnce(&mut TextBox)) -> Option<Rect> {
        let id = self.editing_text()?;
        let before = self.text_caret();
        let area = self.drawing.update(id, |item| {
            if let Some(t) = item.text.as_mut() {
                change(t);
                t.caret = t.caret.min(t.text.len());
            }
        });
        union(union(area, before), self.text_caret())
    }

    // ---- The laser ----

    /// The laser's recent positions, for [`render::RenderOptions::laser`].
    pub fn laser(&self) -> &Laser {
        &self.laser
    }

    /// The laser leaves the layer (or another tool is picked): its dot goes, its trail fades.
    pub fn laser_hide(&mut self) {
        self.laser.hide();
    }

    /// Drops laser positions past their trail at `now_ms`. Call it after redrawing
    /// [`render::animated_area`] for the frame, so the last of the trail is cleared first.
    pub fn laser_prune(&mut self, now_ms: u64, trail_ms: u32) {
        self.laser.prune(now_ms, trail_ms);
    }

    // ---- Disappearing ink ----

    /// With disappearing ink on, removes the items that have faded out by `now_ms`, and the
    /// history about them. Call it each frame while there are fading items, after redrawing
    /// [`render::animated_area`]. Returns the area they covered.
    ///
    /// **Undo stays sane:** a faded item is gone as if it had never been drawn, wherever it
    /// was: in the drawing, or held by the history (erased, cleared, or undone). Its own step
    /// leaves the history, and so does every step that would then do nothing: an erase or Clear
    /// of only faded items, a recolour of one, an add whose item is gone. So Ctrl+Z always
    /// undoes the newest change that's still visible, never brings back something that faded
    /// away, and never spends a press on nothing; the same goes for Ctrl+Y.
    pub fn prune_faded(&mut self, now_ms: u64, delay_ms: u64) -> Option<Rect> {
        let gone = |item: &Item| render::opacity(item, now_ms, Some(delay_ms)) <= 0.0;
        // Faded items in the drawing go now.
        let faded: Vec<ItemId> = self
            .drawing
            .items()
            .filter(|item| Some(item.id) != self.active && gone(item))
            .map(|item| item.id)
            .collect();
        let mut area = None;
        for id in &faded {
            if let Some((_, _, bounds)) = self.drawing.remove(*id) {
                area = union(area, bounds);
            }
        }
        // Faded items the history holds go from it.
        for change in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            match change {
                Change::Add { item, .. } => {
                    if item.as_ref().is_some_and(&gone) {
                        *item = None;
                    }
                }
                Change::Erase { removed, .. } => removed.retain(|(_, item)| !gone(item)),
                Change::Clear { items, .. } => items.retain(|item| !gone(item)),
                Change::Recolour { .. } => {}
            }
        }
        self.drop_no_op_changes();
        area
    }

    /// Drops every history step that would do nothing, until none is left.
    ///
    /// Each stack is walked in the order its steps would run, keeping track of which items
    /// would be drawn at each step: the undo stack from its newest step back (an undone erase or
    /// Clear brings items back; an undone add takes one away), and the redo stack from its next
    /// step on (a redone add brings one back; an erase or Clear takes them away). A step is
    /// dropped when, at its turn, it would change nothing: an add, recolour or erase of items
    /// that wouldn't be drawn then, or a Clear with nothing to clear. Dropping a step changes
    /// what later ones see, so it repeats until nothing more is dropped.
    fn drop_no_op_changes(&mut self) {
        use std::collections::HashSet;
        loop {
            let before = self.undo.len() + self.redo.len();
            let drawn: HashSet<ItemId> = self.drawing.items().map(|i| i.id).collect();

            // Undo runs from the back of the deque.
            let mut state = drawn.clone();
            let mut keep_undo = vec![true; self.undo.len()];
            for (k, change) in self.undo.iter().enumerate().rev() {
                keep_undo[k] = match change {
                    Change::Add { id, .. } => state.remove(id),
                    Change::Recolour { id, .. } => state.contains(id),
                    Change::Erase { removed, .. } => {
                        state.extend(removed.iter().map(|(_, i)| i.id));
                        !removed.is_empty()
                    }
                    Change::Clear { items, .. } => {
                        state.extend(items.iter().map(|i| i.id));
                        !items.is_empty()
                    }
                };
            }
            let mut flags = keep_undo.into_iter();
            self.undo.retain(|_| flags.next().unwrap_or(true));

            // Redo runs from the back of the vector (it's popped).
            let mut state = drawn;
            let mut keep_redo = vec![true; self.redo.len()];
            for (k, change) in self.redo.iter_mut().enumerate().rev() {
                keep_redo[k] = match change {
                    Change::Add { item, .. } => match item {
                        Some(item) => {
                            state.insert(item.id);
                            true
                        }
                        None => false,
                    },
                    Change::Recolour { id, .. } => state.contains(id),
                    Change::Erase { ids, .. } => {
                        ids.retain(|id| state.remove(id));
                        !ids.is_empty()
                    }
                    Change::Clear { .. } => {
                        let any = !state.is_empty();
                        state.clear();
                        any
                    }
                };
            }
            let mut flags = keep_redo.into_iter();
            self.redo.retain(|_| flags.next().unwrap_or(true));

            if self.undo.len() + self.redo.len() == before {
                return;
            }
        }
    }

    /// Whether the item is still being drawn or typed.
    pub fn is_active(&self, id: ItemId) -> bool {
        self.active == Some(id)
    }
}
