//! What keys and pointer samples mean while the layer is open, kept apart
//! from Windows so it can be tested: the key map, the history across monitors, and turning a
//! screen sample into a point on a monitor's layer.

use overmark_core::drawing::{InkPoint, Tilt};
use overmark_core::tool::ToolKind;

/// What a key press asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// P, H, A, R, E, L, T, N, B, X, Z.
    Tool(ToolKind),
    /// 1 to 9: the palette's colour (0-based).
    Colour(usize),
    Undo,
    Redo,
    /// Delete: clear everything (undoable).
    Clear,
    /// F: Live and Frozen swap.
    ToggleFreeze,
    /// W: the next backdrop (Live, Frozen, whiteboard, blackboard); Shift+W (`back`) the one
    /// before.
    Backdrop {
        back: bool,
    },
    /// G: the next guides (off, grid, dots, ruled lines); Shift+G the one before.
    Guides {
        back: bool,
    },
    /// Esc: closes the layer.
    Close,
    /// While typing text.
    Text(TextKey),
    /// Nothing.
    None,
}

/// A key that edits the text being typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKey {
    /// Enter or Esc: the text is done.
    Finish,
    /// Shift+Enter.
    NewLine,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

const VK_BACK: u32 = 0x08;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_RIGHT: u32 = 0x27;
const VK_DELETE: u32 = 0x2E;

/// The tool a letter key picks (core's `ToolKind::key`).
pub fn tool_for_key(vk: u32) -> Option<ToolKind> {
    [
        ToolKind::Pen,
        ToolKind::Highlighter,
        ToolKind::Arrow,
        ToolKind::Rectangle,
        ToolKind::Ellipse,
        ToolKind::Line,
        ToolKind::Text,
        ToolKind::Stamp,
        ToolKind::Blur,
        ToolKind::Eraser,
        ToolKind::Laser,
    ]
    .into_iter()
    .find(|t| t.key() as u32 == vk)
}

/// What `vk` (a virtual-key code, as `WM_KEYDOWN` gives it) means, with Ctrl, Shift and Alt as
/// they were and `typing` while a text item is being typed. While typing, letters and digits
/// are text (they come as characters, `WM_CHAR`), so F, the tool keys and the colours do
/// nothing; Ctrl's keys still work.
pub fn key_action(vk: u32, ctrl: bool, shift: bool, alt: bool, typing: bool) -> KeyAction {
    if ctrl && !alt {
        return match (vk, shift) {
            (0x5A, false) => KeyAction::Undo,
            (0x59, false) | (0x5A, true) => KeyAction::Redo,
            _ => KeyAction::None,
        };
    }
    if alt || ctrl {
        return KeyAction::None;
    }
    if typing {
        return KeyAction::Text(match vk {
            VK_RETURN if shift => TextKey::NewLine,
            VK_RETURN | VK_ESCAPE => TextKey::Finish,
            VK_BACK => TextKey::Backspace,
            VK_DELETE => TextKey::Delete,
            VK_LEFT => TextKey::Left,
            VK_RIGHT => TextKey::Right,
            VK_HOME => TextKey::Home,
            VK_END => TextKey::End,
            _ => return KeyAction::None,
        });
    }
    match vk {
        VK_ESCAPE => KeyAction::Close,
        VK_DELETE => KeyAction::Clear,
        0x46 => KeyAction::ToggleFreeze,
        0x57 => KeyAction::Backdrop { back: shift },
        0x47 => KeyAction::Guides { back: shift },
        0x31..=0x39 => KeyAction::Colour((vk - 0x31) as usize),
        _ => tool_for_key(vk).map_or(KeyAction::None, KeyAction::Tool),
    }
}

/// A screen sample as a point on a monitor's layer: physical pixels from the layer's top-left
/// (`origin`, the monitor's), its time in milliseconds on the drawing's clock.
pub fn ink_point(
    x: i32,
    y: i32,
    origin: (i32, i32),
    pressure: Option<f32>,
    tilt: Option<(f32, f32)>,
    time_100ns: i64,
) -> InkPoint {
    let mut p = match pressure {
        Some(pr) => InkPoint::pen(
            f64::from(x - origin.0),
            f64::from(y - origin.1),
            pr,
            ms(time_100ns),
        ),
        None => InkPoint::mouse(
            f64::from(x - origin.0),
            f64::from(y - origin.1),
            ms(time_100ns),
        ),
    };
    p.tilt = tilt.map(|(x, y)| Tilt { x, y });
    p
}

/// The drawing's clock: milliseconds of the performance counter (as `clock::now_100ns`).
pub fn ms(time_100ns: i64) -> u64 {
    (time_100ns.max(0) / 10_000) as u64
}

/// The undo history across monitors: each monitor's session keeps its own steps, and this
/// keeps which monitors each user action changed, in order, so Ctrl+Z undoes the newest action
/// wherever it was, and Delete's Clear of every monitor undoes as one.
///
/// Each monitor's part of an action keeps the ids of the items its step added or removed: a
/// session drops a step by itself when every item it concerns has faded away (disappearing
/// ink), wherever that step is in its history, and [`History::forget_faded`] drops exactly
/// those parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    done: Vec<Group>,
    undone: Vec<Group>,
}

/// One action: each monitor it changed, with the items its step there concerns.
pub type Group = Vec<(usize, Vec<u64>)>;

fn screens(g: &Group) -> Vec<usize> {
    g.iter().map(|(s, _)| *s).collect()
}

impl History {
    /// A new action changed these monitors (each with its items): it's the newest, and what
    /// could be redone is gone.
    pub fn record(&mut self, group: Group) {
        if group.is_empty() {
            return;
        }
        self.done.push(group);
        self.undone.clear();
    }

    /// The monitors to undo the newest action on, moved to the redo side.
    pub fn undo(&mut self) -> Option<Vec<usize>> {
        let g = self.done.pop()?;
        let s = screens(&g);
        self.undone.push(g);
        Some(s)
    }

    /// The monitors to redo the newest undone action on, moved back.
    pub fn redo(&mut self) -> Option<Vec<usize>> {
        let g = self.undone.pop()?;
        let s = screens(&g);
        self.done.push(g);
        Some(s)
    }

    /// Drops each monitor's part of an action whose items have all faded away (`faded(screen,
    /// id)`), on both sides, as its session dropped that step; an action left with no part goes.
    pub fn forget_faded(&mut self, faded: impl Fn(usize, u64) -> bool) {
        for side in [&mut self.done, &mut self.undone] {
            for g in side.iter_mut() {
                g.retain(|(s, ids)| ids.is_empty() || !ids.iter().all(|&id| faded(*s, id)));
            }
            side.retain(|g| !g.is_empty());
        }
    }

    /// Every (monitor, item id) a part of an action concerns, on both sides.
    pub fn ids(&self) -> impl Iterator<Item = (usize, u64)> + '_ {
        self.done
            .iter()
            .chain(&self.undone)
            .flatten()
            .flat_map(|(s, ids)| ids.iter().map(move |&id| (*s, id)))
    }

    /// Keeps the history in step with monitor `screen`'s session, which has `undo_steps` and
    /// `redo_steps` now: past its 100 steps a session forgets its oldest, so the history forgets
    /// as many of that monitor's oldest. (Faded steps go by [`History::forget_faded`] first,
    /// wherever they are.)
    pub fn sync(&mut self, screen: usize, undo_steps: usize, redo_steps: usize) {
        forget(&mut self.done, screen, undo_steps, false);
        forget(&mut self.undone, screen, redo_steps, true);
    }

    /// Forgets everything (the layer closed).
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}

/// Takes `screen` out of `groups` until at most `keep` of them hold it, from the front (or the
/// back, `from_back`); a group left empty goes.
fn forget(groups: &mut Vec<Group>, screen: usize, keep: usize, from_back: bool) {
    let holds = |g: &Group| g.iter().any(|(s, _)| *s == screen);
    let mut n = groups.iter().filter(|g| holds(g)).count();
    while n > keep {
        let found = if from_back {
            groups.iter().rposition(holds)
        } else {
            groups.iter().position(holds)
        };
        let Some(k) = found else {
            return;
        };
        groups[k].retain(|(s, _)| *s != screen);
        if groups[k].is_empty() {
            groups.remove(k);
        }
        n -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group of monitors, each with no items (the length-only tests).
    fn g(screens: &[usize]) -> Group {
        screens.iter().map(|&s| (s, Vec::new())).collect()
    }

    fn sides(h: &History) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        (
            h.done.iter().map(screens).collect(),
            h.undone.iter().map(screens).collect(),
        )
    }

    #[test]
    fn history_forgets_what_a_session_forgot() {
        let mut h = History::default();
        h.record(g(&[0]));
        h.record(g(&[0, 1]));
        h.record(g(&[1]));
        h.record(g(&[0]));
        // Monitor 0's oldest step went (its 101st): its oldest group goes; its share of [0, 1]
        // stays.
        h.sync(0, 2, 0);
        assert_eq!(sides(&h).0, vec![vec![0, 1], vec![1], vec![0]]);
        h.sync(0, 1, 0);
        assert_eq!(sides(&h).0, vec![vec![1], vec![1], vec![0]]);
        assert_eq!(h.undo(), Some(vec![0]));
        assert_eq!(h.undo(), Some(vec![1]));
        h.sync(1, 1, 0);
        assert_eq!(sides(&h), (vec![vec![1]], vec![vec![0]]));
    }

    #[test]
    fn faded_parts_go_wherever_they_are() {
        let mut h = History::default();
        h.record(vec![(0, vec![1])]); // A on monitor 0
        h.record(vec![(0, vec![2])]); // C on monitor 0
        h.record(vec![(1, vec![1])]); // B on monitor 1 (ids are per monitor)
        h.record(vec![(0, vec![1])]); // A erased
        h.record(vec![(0, vec![2]), (1, vec![1])]); // Clear of C and B
        h.forget_faded(|s, id| s == 0 && id == 1);
        assert_eq!(sides(&h).0, vec![vec![0], vec![1], vec![0, 1]]);
        // On the redo side too.
        assert_eq!(h.undo(), Some(vec![0, 1]));
        h.forget_faded(|s, id| (s, id) == (0, 2));
        assert_eq!(sides(&h), (vec![vec![1]], vec![vec![1]]));
    }

    #[test]
    fn the_tool_keys() {
        for (k, t) in [
            ('P', ToolKind::Pen),
            ('H', ToolKind::Highlighter),
            ('A', ToolKind::Arrow),
            ('R', ToolKind::Rectangle),
            ('E', ToolKind::Ellipse),
            ('L', ToolKind::Line),
            ('T', ToolKind::Text),
            ('N', ToolKind::Stamp),
            ('B', ToolKind::Blur),
            ('X', ToolKind::Eraser),
            ('Z', ToolKind::Laser),
        ] {
            assert_eq!(
                key_action(k as u32, false, false, false, false),
                KeyAction::Tool(t)
            );
        }
        assert_eq!(
            key_action('Q' as u32, false, false, false, false),
            KeyAction::None
        );
    }

    #[test]
    fn colours_history_clear_freeze_and_close() {
        assert_eq!(
            key_action(0x31, false, false, false, false),
            KeyAction::Colour(0)
        );
        assert_eq!(
            key_action(0x39, false, false, false, false),
            KeyAction::Colour(8)
        );
        // Ctrl+Z undoes; Ctrl+Y and Ctrl+Shift+Z redo.
        assert_eq!(key_action(0x5A, true, false, false, false), KeyAction::Undo);
        assert_eq!(key_action(0x59, true, false, false, false), KeyAction::Redo);
        assert_eq!(key_action(0x5A, true, true, false, false), KeyAction::Redo);
        assert_eq!(
            key_action(VK_DELETE, false, false, false, false),
            KeyAction::Clear
        );
        assert_eq!(
            key_action(0x46, false, false, false, false),
            KeyAction::ToggleFreeze
        );
        assert_eq!(
            key_action(VK_ESCAPE, false, false, false, false),
            KeyAction::Close
        );
        // Ctrl+Alt is AltGr: nothing.
        assert_eq!(key_action(0x5A, true, false, true, false), KeyAction::None);
    }

    #[test]
    fn w_and_g_cycle_the_backdrop_and_the_guides_both_ways() {
        assert_eq!(
            key_action('W' as u32, false, false, false, false),
            KeyAction::Backdrop { back: false }
        );
        assert_eq!(
            key_action('W' as u32, false, true, false, false),
            KeyAction::Backdrop { back: true }
        );
        assert_eq!(
            key_action('G' as u32, false, false, false, false),
            KeyAction::Guides { back: false }
        );
        assert_eq!(
            key_action('G' as u32, false, true, false, false),
            KeyAction::Guides { back: true }
        );
    }

    #[test]
    fn while_typing_letters_are_text_and_f_does_nothing() {
        for vk in ['F', 'W', 'G', 'P', 'Z', '1'] {
            assert_eq!(
                key_action(vk as u32, false, false, false, true),
                KeyAction::None
            );
        }
        assert_eq!(
            key_action(VK_RETURN, false, false, false, true),
            KeyAction::Text(TextKey::Finish)
        );
        assert_eq!(
            key_action(VK_RETURN, false, true, false, true),
            KeyAction::Text(TextKey::NewLine)
        );
        assert_eq!(
            key_action(VK_ESCAPE, false, false, false, true),
            KeyAction::Text(TextKey::Finish)
        );
        assert_eq!(
            key_action(VK_DELETE, false, false, false, true),
            KeyAction::Text(TextKey::Delete)
        );
        assert_eq!(
            key_action(VK_BACK, false, false, false, true),
            KeyAction::Text(TextKey::Backspace)
        );
        // Undo still works while typing.
        assert_eq!(key_action(0x5A, true, false, false, true), KeyAction::Undo);
    }

    #[test]
    fn a_sample_lands_on_its_monitors_layer() {
        let p = ink_point(
            -1900,
            50,
            (-1920, 0),
            Some(0.5),
            Some((10.0, -5.0)),
            123_450_000,
        );
        assert_eq!((p.x, p.y), (20.0, 50.0));
        assert_eq!(p.pressure, Some(0.5));
        assert_eq!(p.tilt, Some(Tilt { x: 10.0, y: -5.0 }));
        assert_eq!(p.time_ms, 12_345);
        let m = ink_point(5, 6, (0, 0), None, None, 0);
        assert_eq!((m.pressure, m.tilt), (None, None));
    }

    #[test]
    fn history_undoes_the_newest_action_wherever_it_was() {
        let mut h = History::default();
        h.record(g(&[0]));
        h.record(g(&[1]));
        h.record(g(&[0, 1])); // Delete on both.
        assert_eq!(h.undo(), Some(vec![0, 1]));
        assert_eq!(h.undo(), Some(vec![1]));
        assert_eq!(h.redo(), Some(vec![1]));
        h.record(g(&[0])); // A new action drops what could be redone.
        assert_eq!(h.redo(), None);
        assert_eq!(h.undo(), Some(vec![0]));
        h.record(Vec::new()); // Nothing changed: not an action.
        assert_eq!(h.undo(), Some(vec![1]));
        assert_eq!(h.undo(), Some(vec![0]));
        assert_eq!(h.undo(), None);
    }
}
