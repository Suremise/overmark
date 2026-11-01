//! The laser pointer: a glowing dot with a short fading trail.
//!
//! It's never part of the drawing or the history and leaves nothing behind: the session keeps
//! its recent positions, and the renderer draws them when given the time
//! ([`crate::render::RenderOptions::laser`]). The trail's length is Settings'
//! `laser_trail_ms`.

use std::collections::VecDeque;

use kurbo::{Point, Rect};

use crate::geometry::AA_MARGIN;

/// The dot's radius, in physical pixels.
pub const DOT_RADIUS: f64 = 5.0;
/// The glow's radius around the dot.
pub const GLOW_RADIUS: f64 = 12.0;

/// Where the laser has been lately.
#[derive(Debug, Clone, Default)]
pub struct Laser {
    /// Positions with their times, oldest first.
    trail: VecDeque<(Point, u64)>,
    /// Whether the pointer is on the layer, so the dot shows.
    shown: bool,
}

impl Laser {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pointer moved to `at` at `time_ms`.
    pub fn move_to(&mut self, at: Point, time_ms: u64) {
        self.trail.push_back((at, time_ms));
        self.shown = true;
    }

    /// The pointer left (or another tool was picked): the dot goes; the trail fades as usual.
    pub fn hide(&mut self) {
        self.shown = false;
    }

    /// Forgets positions older than the trail, at `now_ms`.
    pub fn prune(&mut self, now_ms: u64, trail_ms: u32) {
        while self.trail.len() > 1
            && self
                .trail
                .front()
                .is_some_and(|(_, t)| now_ms.saturating_sub(*t) > u64::from(trail_ms))
        {
            self.trail.pop_front();
        }
        if !self.shown
            && self
                .trail
                .back()
                .is_some_and(|(_, t)| now_ms.saturating_sub(*t) > u64::from(trail_ms))
        {
            self.trail.clear();
        }
    }

    /// Whether there's anything to draw at `now_ms`.
    pub fn is_visible(&self, now_ms: u64, trail_ms: u32) -> bool {
        self.shown || !self.visible_trail(now_ms, trail_ms).is_empty()
    }

    /// The dot's position, if it shows.
    pub fn dot(&self) -> Option<Point> {
        self.shown
            .then(|| self.trail.back().map(|(p, _)| *p))
            .flatten()
    }

    /// The trail at `now_ms`: each position with how far it has faded, 0 (new) to 1 (gone).
    pub fn visible_trail(&self, now_ms: u64, trail_ms: u32) -> Vec<(Point, f64)> {
        let trail = f64::from(trail_ms.max(1));
        self.trail
            .iter()
            .filter_map(|(p, t)| {
                let age = now_ms.saturating_sub(*t) as f64 / trail;
                (age < 1.0).then_some((*p, age))
            })
            .collect()
    }

    /// Everything the laser may have painted and not yet cleared: every position kept (expired
    /// ones too, until [`Laser::prune`] drops them), with the dot's glow. Redrawing this area
    /// at a frame's time draws what's left, or clears it.
    pub fn painted_bounds(&self) -> Option<Rect> {
        let reach = GLOW_RADIUS + AA_MARGIN;
        self.trail
            .iter()
            .map(|(p, _)| Rect::from_points(*p, *p).inflate(reach, reach))
            .reduce(|a, b| a.union(b))
    }

    /// The area the laser covers at `now_ms` (the dot and the trail, with their glow).
    pub fn bounds(&self, now_ms: u64, trail_ms: u32) -> Option<Rect> {
        let reach = GLOW_RADIUS + AA_MARGIN;
        self.visible_trail(now_ms, trail_ms)
            .into_iter()
            .map(|(p, _)| p)
            .chain(self.dot())
            .map(|p| Rect::from_points(p, p).inflate(reach, reach))
            .reduce(|a, b| a.union(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trail_fades_over_its_length_and_the_dot_stays() {
        let mut laser = Laser::new();
        laser.move_to(Point::new(0.0, 0.0), 0);
        laser.move_to(Point::new(10.0, 0.0), 100);
        laser.move_to(Point::new(20.0, 0.0), 200);
        let at_250 = laser.visible_trail(250, 200);
        // The first point is 250 ms old, past a 200 ms trail.
        assert_eq!(at_250.len(), 2);
        assert!((at_250[0].1 - 0.75).abs() < 1e-9);
        assert_eq!(laser.dot(), Some(Point::new(20.0, 0.0)));
        // Long after, only the dot.
        assert!(laser.visible_trail(5000, 200).is_empty());
        assert!(laser.is_visible(5000, 200));
    }

    #[test]
    fn hidden_and_faded_it_leaves_nothing() {
        let mut laser = Laser::new();
        laser.move_to(Point::new(5.0, 5.0), 0);
        laser.hide();
        assert!(laser.is_visible(100, 300), "the trail still fades out");
        assert!(laser.bounds(100, 300).is_some());
        laser.prune(1000, 300);
        assert!(!laser.is_visible(1000, 300));
        assert!(laser.bounds(1000, 300).is_none());
    }

    #[test]
    fn pruning_keeps_the_newest_position() {
        let mut laser = Laser::new();
        for i in 0..100u32 {
            laser.move_to(Point::new(f64::from(i), 0.0), u64::from(i) * 10);
        }
        laser.prune(1000, 200);
        assert!(laser.trail.len() <= 22);
        assert_eq!(laser.dot(), Some(Point::new(99.0, 0.0)));
    }
}
