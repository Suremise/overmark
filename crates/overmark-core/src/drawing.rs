//! The drawing: what's on one monitor's layer, as a list of items.
//!
//! Coordinates are **physical pixels of that monitor's layer**, with (0, 0) at its top-left.
//! An item is a stroke, a shape, a stamp or text, each a tool with the data it needs; items
//! keep a tool, a colour, a width and points.
//!
//! The drawing keeps each item's outline ready to fill, so drawing a frame doesn't recompute
//! it, and every change says which area changed, so the layer can redraw just that
//! ([`crate::render::render_region`]).
//!
//! ```
//! use overmark_core::colour::Colour;
//! use overmark_core::drawing::{Drawing, InkPoint};
//! use overmark_core::tool::ToolKind;
//!
//! let mut drawing = Drawing::new();
//! let (id, _) = drawing.begin(ToolKind::Pen, Colour::rgb(229, 57, 53), 6.0, InkPoint::mouse(10.0, 10.0, 0));
//! let dirty = drawing.push_point(id, InkPoint::mouse(40.0, 20.0, 8));
//! assert!(dirty.is_some());
//! drawing.finish(id);
//! assert_eq!(drawing.items().len(), 1);
//! ```

use kurbo::{BezPath, Point, Rect};

use crate::colour::Colour;
use crate::geometry::{self, AA_MARGIN, Geometry};
use crate::pen;
use crate::tool::ToolKind;

/// A pen's tilt, in degrees from upright along each axis (-90 to 90), as Windows reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tilt {
    pub x: f32,
    pub y: f32,
}

/// One input sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InkPoint {
    /// Physical pixels from the layer's left edge.
    pub x: f64,
    /// Physical pixels from the layer's top edge.
    pub y: f64,
    /// The pen's pressure, 0 to 1. `None` for a mouse (or a pen that reports none): the
    /// stroke's speed then sets its width.
    pub pressure: Option<f32>,
    /// The pen's tilt, if it reports one. Kept for later tools; strokes don't use it yet.
    pub tilt: Option<Tilt>,
    /// When the sample was taken, in milliseconds on any steady clock the platform picks (the
    /// same one for a whole drawing). Disappearing ink and the laser use it.
    pub time_ms: u64,
}

impl InkPoint {
    /// A mouse sample: no pressure, no tilt.
    pub fn mouse(x: f64, y: f64, time_ms: u64) -> Self {
        InkPoint {
            x,
            y,
            pressure: None,
            tilt: None,
            time_ms,
        }
    }

    /// A pen sample with pressure (0 to 1, clamped).
    pub fn pen(x: f64, y: f64, pressure: f32, time_ms: u64) -> Self {
        InkPoint {
            x,
            y,
            pressure: Some(if pressure.is_nan() {
                0.0
            } else {
                pressure.clamp(0.0, 1.0)
            }),
            tilt: None,
            time_ms,
        }
    }

    pub fn position(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// An item's id: unique within its drawing, never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub u64);

/// One thing drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: ItemId,
    pub tool: ToolKind,
    pub colour: Colour,
    /// The base width in physical pixels: a stroke's diameter at medium pressure.
    pub width: f32,
    /// The input, in order.
    pub points: Vec<InkPoint>,
    /// Whether the pointer has lifted. A finished stroke ends exactly at its last point; one
    /// still being drawn trails slightly behind it (smoothing).
    pub finished: bool,
    /// The number a numbered stamp shows (1, 2, 3…); `None` for every other item.
    pub number: Option<u32>,
    /// A text item's text and caret; `None` for every other item.
    pub text: Option<TextBox>,
}

/// A text item's content: lines split at newlines, with the caret's byte index while it's
/// edited.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextBox {
    pub text: String,
    /// A byte index at a character boundary, 0 to `text.len()`.
    pub caret: usize,
}

impl Item {
    /// Whether this item is ink (the eraser and the laser never are).
    pub fn is_drawn(&self) -> bool {
        !matches!(self.tool, ToolKind::Eraser | ToolKind::Laser)
    }

    /// When it was last drawn: its last point's time, for disappearing ink.
    pub fn drawn_at(&self) -> u64 {
        self.points.iter().map(|p| p.time_ms).max().unwrap_or(0)
    }
}

/// The items on one layer, in drawing order (later ones on top).
#[derive(Debug, Clone, Default)]
pub struct Drawing {
    items: Vec<(Item, Geometry)>,
    next_id: u64,
}

impl Drawing {
    pub fn new() -> Self {
        Self::default()
    }

    /// The items, bottom first.
    pub fn items(&self) -> impl ExactSizeIterator<Item = &Item> + '_ {
        self.items.iter().map(|(item, _)| item)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.index(id).map(|i| &self.items[i].0)
    }

    /// Where the item is in drawing order (0 is the bottom).
    pub fn index(&self, id: ItemId) -> Option<usize> {
        self.items.iter().position(|(item, _)| item.id == id)
    }

    /// Starts a stroke at `first`, on top of the rest. Returns its id and the area to redraw.
    pub fn begin(
        &mut self,
        tool: ToolKind,
        colour: Colour,
        width: f32,
        first: InkPoint,
    ) -> (ItemId, Option<Rect>) {
        let id = ItemId(self.next_id);
        self.next_id += 1;
        let item = Item {
            id,
            tool,
            colour,
            width,
            points: vec![first],
            finished: false,
            number: None,
            text: None,
        };
        let dirty = self.insert(self.items.len(), item);
        (id, dirty)
    }

    /// Changes an item through `change` (its points, text, width…), then recomputes its shape.
    /// Returns the area that changed. Its id and place stay as they are.
    pub fn update(&mut self, id: ItemId, change: impl FnOnce(&mut Item)) -> Option<Rect> {
        let i = self.index(id)?;
        let (item, geometry) = &mut self.items[i];
        change(item);
        item.id = id;
        let new = geometry_of(item);
        let dirty = changed_area(geometry, &new);
        *geometry = new;
        dirty
    }

    /// Adds a finished item on top, made whole at once (a stamp, say): its tool, colour,
    /// width, points and number. Returns its id and the area it covers.
    pub fn add(
        &mut self,
        tool: ToolKind,
        colour: Colour,
        width: f32,
        points: Vec<InkPoint>,
        number: Option<u32>,
    ) -> (ItemId, Option<Rect>) {
        let id = ItemId(self.next_id);
        self.next_id += 1;
        let item = Item {
            id,
            tool,
            colour,
            width,
            points,
            finished: true,
            number,
            text: None,
        };
        let dirty = self.insert(self.items.len(), item);
        (id, dirty)
    }

    /// Changes an item's colour. Returns its old colour and the area to redraw.
    pub fn set_colour(&mut self, id: ItemId, colour: Colour) -> Option<(Colour, Option<Rect>)> {
        let i = self.index(id)?;
        let (item, geometry) = &mut self.items[i];
        let old = std::mem::replace(&mut item.colour, colour);
        Some((old, geometry.bounds))
    }

    /// The items a point or a short drag from `from` to `to` touches, within `tolerance`
    /// pixels, top first: the eraser's test. An item is touched when the drag passes over its
    /// filled outline or within `tolerance` of its edge. An item that draws no outline (yet) is
    /// touched within half its width, plus `tolerance`, of its points.
    pub fn hit_test(&self, from: Point, to: Point, tolerance: f64) -> Vec<ItemId> {
        let tolerance = tolerance.max(0.0);
        let reach = Rect::from_points(from, to).inflate(tolerance, tolerance);
        self.items
            .iter()
            .rev()
            .filter(|(item, geometry)| {
                if !geometry.contours.is_empty() {
                    geometry.bounds.is_some_and(|b| overlaps(b, reach))
                        && crate::hit::touches_polygons(&geometry.contours, from, to, tolerance)
                } else if item.is_drawn() {
                    let reach = tolerance + f64::from(item.width.max(0.0)) / 2.0;
                    let points: Vec<Point> = item.points.iter().map(InkPoint::position).collect();
                    crate::hit::touches_polyline(&points, from, to, reach)
                } else {
                    false
                }
            })
            .map(|(item, _)| item.id)
            .collect()
    }

    /// Adds a sample to an unfinished item. Returns the area whose pixels changed, or `None` if
    /// nothing did (or there's no such unfinished item).
    pub fn push_point(&mut self, id: ItemId, point: InkPoint) -> Option<Rect> {
        let i = self.index(id)?;
        let (item, geometry) = &mut self.items[i];
        if item.finished {
            return None;
        }
        item.points.push(point);
        let new = geometry_of(item);
        let dirty = changed_area(geometry, &new);
        *geometry = new;
        dirty
    }

    /// Marks an item finished (the pointer lifted). Returns the area that changed.
    pub fn finish(&mut self, id: ItemId) -> Option<Rect> {
        let i = self.index(id)?;
        let (item, geometry) = &mut self.items[i];
        if item.finished {
            return None;
        }
        item.finished = true;
        let new = geometry_of(item);
        let dirty = changed_area(geometry, &new);
        *geometry = new;
        dirty
    }

    /// Puts an item at `index` in drawing order (clamped to the top), for undo and redo. Its id
    /// is kept, and later ids stay above it. Returns the area it covers.
    pub fn insert(&mut self, index: usize, item: Item) -> Option<Rect> {
        self.next_id = self.next_id.max(item.id.0 + 1);
        let geometry = geometry_of(&item);
        let bounds = geometry.bounds;
        let index = index.min(self.items.len());
        self.items.insert(index, (item, geometry));
        bounds
    }

    /// Takes an item out, with where it was. Returns it, its index and the area it covered.
    pub fn remove(&mut self, id: ItemId) -> Option<(Item, usize, Option<Rect>)> {
        let i = self.index(id)?;
        let (item, geometry) = self.items.remove(i);
        Some((item, i, geometry.bounds))
    }

    /// Removes every item. Returns them, bottom first, and the area they covered.
    pub fn clear(&mut self) -> (Vec<Item>, Option<Rect>) {
        let bounds = union_all(self.items.iter().map(|(_, g)| g.bounds));
        let items = self.items.drain(..).map(|(item, _)| item).collect();
        (items, bounds)
    }

    /// The area an item covers, anti-aliasing included.
    pub fn bounds(&self, id: ItemId) -> Option<Rect> {
        self.index(id).and_then(|i| self.items[i].1.bounds)
    }

    /// The area every item covers.
    pub fn total_bounds(&self) -> Option<Rect> {
        union_all(self.items.iter().map(|(_, g)| g.bounds))
    }

    /// A pen or highlighter stroke's outline as a closed polygon (empty for other items).
    pub fn outline(&self, id: ItemId) -> &[Point] {
        self.index(id)
            .map_or(&[][..], |i| self.items[i].1.outline.as_slice())
    }

    /// An item's outline smoothed into curves: what's filled.
    pub fn outline_path(&self, id: ItemId) -> BezPath {
        pen::smooth_path(self.outline(id))
    }

    pub(crate) fn drawn(&self) -> impl Iterator<Item = (&Item, &Geometry)> + '_ {
        self.items.iter().map(|(item, geometry)| (item, geometry))
    }
}

fn geometry_of(item: &Item) -> Geometry {
    geometry::of(item)
}

/// The bounding box of points, grown by the anti-aliasing margin. The smoothed curves stay
/// inside their polygon's points (each is a quadratic whose control points are polygon points or
/// midpoints between them).
fn outline_bounds(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    let rect = points
        .iter()
        .fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p));
    Some(rect.inflate(AA_MARGIN, AA_MARGIN))
}

/// Whether two rectangles share any point, edges included: a flat rectangle (a horizontal or
/// vertical drag at zero tolerance) still overlaps what it crosses.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn union_all(rects: impl Iterator<Item = Option<Rect>>) -> Option<Rect> {
    rects.flatten().reduce(|a, b| a.union(b))
}

/// The area that differs between two outlines of the same item.
///
/// A stroke growing at its end changes its outline only near the end: the start of the
/// polygon (the left side) and its end (the right side, back to the start cap) stay as they
/// were. So the points they share from each end are skipped, and the rest, of both, with two
/// neighbours each side (the smoothing curves reach between neighbours), are what changed.
fn changed_area(old: &Geometry, new: &Geometry) -> Option<Rect> {
    let (a, b) = (&old.outline, &new.outline);
    if a.is_empty() || b.is_empty() {
        // Not a growing stroke: everything it covered, before and after.
        return match (old.bounds, new.bounds) {
            (Some(x), Some(y)) => Some(x.union(y)),
            (x, y) => x.or(y),
        };
    }
    let prefix = a.iter().zip(b).take_while(|(p, q)| p == q).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suffix)
        .take_while(|(p, q)| p == q)
        .count();
    if prefix == a.len() && a.len() == b.len() {
        return None;
    }
    let changed = |points: &[Point]| -> Vec<Point> {
        let n = points.len();
        if n == 0 {
            return Vec::new();
        }
        let start = prefix.saturating_sub(2);
        let end = (n - suffix.min(n) + 2).min(n);
        let mut out: Vec<Point> = points[start.min(end)..end].to_vec();
        // The closing curve joins the last point to the first.
        if end == n || start == 0 {
            out.push(points[0]);
            out.push(points[n - 1]);
        }
        out
    };
    let mut points = changed(a);
    points.extend(changed(b));
    outline_bounds(&points)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Colour = Colour::rgb(255, 0, 0);

    fn stroke(drawing: &mut Drawing, points: &[(f64, f64)]) -> ItemId {
        let (id, _) = drawing.begin(
            ToolKind::Pen,
            RED,
            8.0,
            InkPoint::mouse(points[0].0, points[0].1, 0),
        );
        for (i, (x, y)) in points.iter().enumerate().skip(1) {
            drawing.push_point(id, InkPoint::mouse(*x, *y, i as u64 * 8));
        }
        id
    }

    #[test]
    fn ids_are_unique_and_order_is_kept() {
        let mut d = Drawing::new();
        let a = stroke(&mut d, &[(0.0, 0.0), (10.0, 0.0)]);
        let b = stroke(&mut d, &[(0.0, 10.0), (10.0, 10.0)]);
        assert_ne!(a, b);
        assert_eq!(d.index(a), Some(0));
        assert_eq!(d.index(b), Some(1));
        let (item, at, area) = d.remove(a).unwrap();
        assert_eq!(at, 0);
        assert!(area.is_some());
        assert_eq!(d.index(b), Some(0));
        d.insert(at, item);
        assert_eq!(d.index(a), Some(0));
        let c = stroke(&mut d, &[(5.0, 5.0)]);
        assert!(c > b, "a new id is above every id ever used");
    }

    #[test]
    fn a_finished_item_takes_no_more_points() {
        let mut d = Drawing::new();
        let id = stroke(&mut d, &[(0.0, 0.0), (30.0, 0.0), (60.0, 5.0)]);
        assert!(d.finish(id).is_some());
        assert!(d.finish(id).is_none());
        assert!(d.push_point(id, InkPoint::mouse(90.0, 9.0, 99)).is_none());
        assert_eq!(d.item(id).unwrap().points.len(), 3);
    }

    #[test]
    fn bounds_cover_the_outline_and_its_width() {
        let mut d = Drawing::new();
        let id = stroke(&mut d, &[(100.0, 100.0), (150.0, 100.0), (200.0, 100.0)]);
        d.finish(id);
        let b = d.bounds(id).unwrap();
        for p in d.outline(id) {
            assert!(b.contains(*p), "{p:?} outside {b:?}");
        }
        assert!(b.x0 < 100.0 && b.x1 > 200.0 && b.y0 < 100.0 && b.y1 > 100.0);
        assert!(b.height() <= 8.0 + 2.0 * AA_MARGIN + 1e-9, "{b:?}");
    }

    #[test]
    fn the_changed_area_is_near_the_end_of_a_long_stroke() {
        let mut d = Drawing::new();
        let points: Vec<(f64, f64)> = (0..200).map(|i| (f64::from(i) * 5.0, 300.0)).collect();
        let id = stroke(&mut d, &points);
        let dirty = d
            .push_point(id, InkPoint::mouse(1000.0, 300.0, 9999))
            .unwrap();
        // The stroke runs from 0 to 1000; only its tip changed.
        assert!(dirty.x0 > 900.0, "{dirty:?}");
        assert!(dirty.width() < 100.0, "{dirty:?}");
    }

    #[test]
    fn the_changed_area_covers_every_changed_point() {
        let mut d = Drawing::new();
        let (id, _) = d.begin(ToolKind::Pen, RED, 12.0, InkPoint::pen(50.0, 50.0, 0.3, 0));
        for i in 1..120 {
            let t = f64::from(i);
            let before: Vec<Point> = d.outline(id).to_vec();
            let p = InkPoint::pen(
                50.0 + t * 3.0,
                50.0 + (t / 9.0).sin() * 40.0,
                (0.3 + (t / 13.0).sin().abs() * 0.6) as f32,
                i as u64,
            );
            let dirty = d.push_point(id, p);
            let after = d.outline(id);
            let changed: Vec<Point> = before
                .iter()
                .filter(|q| !after.contains(q))
                .chain(after.iter().filter(|q| !before.contains(q)))
                .copied()
                .collect();
            if let Some(dirty) = dirty {
                for q in changed {
                    assert!(dirty.contains(q), "step {i}: {q:?} outside {dirty:?}");
                }
            } else {
                assert!(changed.is_empty(), "step {i}");
            }
        }
    }

    #[test]
    fn only_pen_strokes_draw_for_now() {
        let mut d = Drawing::new();
        let (id, area) = d.begin(ToolKind::Laser, RED, 8.0, InkPoint::mouse(0.0, 0.0, 0));
        assert!(area.is_none());
        assert!(d.outline(id).is_empty());
        assert!(d.total_bounds().is_none());
    }

    #[test]
    fn clear_returns_everything() {
        let mut d = Drawing::new();
        stroke(&mut d, &[(0.0, 0.0), (10.0, 0.0)]);
        stroke(&mut d, &[(0.0, 50.0), (10.0, 50.0)]);
        let total = d.total_bounds().unwrap();
        let (items, area) = d.clear();
        assert_eq!(items.len(), 2);
        assert_eq!(area, Some(total));
        assert!(d.is_empty());
    }

    #[test]
    fn pen_pressure_is_clamped() {
        assert_eq!(InkPoint::pen(0.0, 0.0, 1.7, 0).pressure, Some(1.0));
        assert_eq!(InkPoint::pen(0.0, 0.0, -1.0, 0).pressure, Some(0.0));
        assert_eq!(InkPoint::pen(0.0, 0.0, f32::NAN, 0).pressure, Some(0.0));
    }
}
