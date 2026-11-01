//! Global hotkeys through the `global-hotkey` crate, which calls `RegisterHotKey` (never a
//! keyboard hook): the app's hotkeys come to its own hidden window as `WM_HOTKEY`.

use std::sync::Mutex;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::core::hotkey::{Hotkey, Key};
use crate::win32::clock;

/// What a hotkey does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the layer Live.
    Draw,
    /// Open the layer Frozen.
    DrawFrozen,
    /// Open the layer on the whiteboard (Settings' `backdrops.hotkey_whiteboard`, unset by
    /// default).
    DrawWhiteboard,
    /// Open the layer on the blackboard (`backdrops.hotkey_blackboard`).
    DrawBlackboard,
}

/// Why a hotkey couldn't be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Another app (or Windows) has it.
    Taken,
    /// Windows refused it for another reason.
    Failed(String),
}

/// Presses that came, with when they arrived (100 ns units, as `clock::now_100ns`): set by the
/// crate's handler, which runs on our thread while its window's message is dispatched.
static PRESSED: Mutex<Vec<(u32, i64)>> = Mutex::new(Vec::new());

fn code(key: Key) -> Code {
    match key {
        Key::Letter(c) => match c {
            'A' => Code::KeyA,
            'B' => Code::KeyB,
            'C' => Code::KeyC,
            'D' => Code::KeyD,
            'E' => Code::KeyE,
            'F' => Code::KeyF,
            'G' => Code::KeyG,
            'H' => Code::KeyH,
            'I' => Code::KeyI,
            'J' => Code::KeyJ,
            'K' => Code::KeyK,
            'L' => Code::KeyL,
            'M' => Code::KeyM,
            'N' => Code::KeyN,
            'O' => Code::KeyO,
            'P' => Code::KeyP,
            'Q' => Code::KeyQ,
            'R' => Code::KeyR,
            'S' => Code::KeyS,
            'T' => Code::KeyT,
            'U' => Code::KeyU,
            'V' => Code::KeyV,
            'W' => Code::KeyW,
            'X' => Code::KeyX,
            'Y' => Code::KeyY,
            _ => Code::KeyZ,
        },
        Key::Digit(d) => [
            Code::Digit0,
            Code::Digit1,
            Code::Digit2,
            Code::Digit3,
            Code::Digit4,
            Code::Digit5,
            Code::Digit6,
            Code::Digit7,
            Code::Digit8,
            Code::Digit9,
        ][usize::from(d.min(9))],
        Key::F(n) => [
            Code::F1,
            Code::F2,
            Code::F3,
            Code::F4,
            Code::F5,
            Code::F6,
            Code::F7,
            Code::F8,
            Code::F9,
            Code::F10,
            Code::F11,
            Code::F12,
            Code::F13,
            Code::F14,
            Code::F15,
            Code::F16,
            Code::F17,
            Code::F18,
            Code::F19,
            Code::F20,
            Code::F21,
            Code::F22,
            Code::F23,
            Code::F24,
        ][usize::from(n.clamp(1, 24) - 1)],
        Key::PrintScreen => Code::PrintScreen,
        Key::Space => Code::Space,
        Key::Tab => Code::Tab,
        Key::Enter => Code::Enter,
        Key::Delete => Code::Delete,
        Key::Insert => Code::Insert,
        Key::Home => Code::Home,
        Key::End => Code::End,
        Key::PageUp => Code::PageUp,
        Key::PageDown => Code::PageDown,
        Key::Left => Code::ArrowLeft,
        Key::Right => Code::ArrowRight,
        Key::Up => Code::ArrowUp,
        Key::Down => Code::ArrowDown,
    }
}

/// The crate's form of a hotkey.
pub fn to_hotkey(h: &Hotkey) -> HotKey {
    let mut mods = Modifiers::empty();
    if h.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if h.alt {
        mods |= Modifiers::ALT;
    }
    if h.shift {
        mods |= Modifiers::SHIFT;
    }
    if h.win {
        mods |= Modifiers::META;
    }
    HotKey::new((!mods.is_empty()).then_some(mods), code(h.key))
}

/// The Windows virtual-key code of `key`.
pub fn vk(key: Key) -> u32 {
    match key {
        Key::Letter(c) => c as u32,
        Key::Digit(d) => u32::from(b'0' + d.min(9)),
        Key::F(n) => 0x70 + u32::from(n.clamp(1, 24) - 1),
        Key::PrintScreen => 0x2C,
        Key::Space => 0x20,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Delete => 0x2E,
        Key::Insert => 0x2D,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::Left => 0x25,
        Key::Up => 0x26,
        Key::Right => 0x27,
        Key::Down => 0x28,
    }
}

/// Whether `hotkey` is free now: no app has it, this one included.
pub fn is_free(hotkey: &Hotkey) -> bool {
    crate::win32::hotkey_probe::is_free(
        hotkey.ctrl,
        hotkey.alt,
        hotkey.shift,
        hotkey.win,
        vk(hotkey.key),
    )
}

/// The nearest free hotkey to `hotkey`, from [`Hotkey::alternatives`], skipping
/// those in `ours` (Overmark's own: free for us, but they're another action's).
pub fn nearest_free(hotkey: &Hotkey, ours: &[Hotkey]) -> Option<Hotkey> {
    hotkey
        .alternatives()
        .into_iter()
        .filter(|h| !ours.contains(h))
        .find(is_free)
}

/// The app's registered hotkeys. Dropping it unregisters them.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<(HotKey, Action)>,
}

impl Hotkeys {
    /// Makes the hidden window hotkeys come to. Call it on the thread that runs the message
    /// loop.
    pub fn new() -> Result<Hotkeys, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
            if e.state() == HotKeyState::Pressed {
                PRESSED.lock().unwrap().push((e.id(), clock::now_100ns()));
            }
        }));
        Ok(Hotkeys {
            manager,
            registered: Vec::new(),
        })
    }

    /// Registers `hotkey` for `action`.
    pub fn register(&mut self, action: Action, hotkey: &Hotkey) -> Result<(), RegisterError> {
        let hk = to_hotkey(hotkey);
        match self.manager.register(hk) {
            Ok(()) => {
                self.registered.push((hk, action));
                Ok(())
            }
            Err(global_hotkey::Error::AlreadyRegistered(_)) => Err(RegisterError::Taken),
            Err(e) => Err(RegisterError::Failed(e.to_string())),
        }
    }

    /// The hotkeys pressed since the last call, oldest first, with when each arrived.
    pub fn take_pressed(&self) -> Vec<(Action, i64)> {
        let pressed = std::mem::take(&mut *PRESSED.lock().unwrap());
        pressed
            .into_iter()
            .filter_map(|(id, t)| {
                self.registered
                    .iter()
                    .find(|(hk, _)| hk.id() == id)
                    .map(|(_, a)| (*a, t))
            })
            .collect()
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        for (hk, _) in &self.registered {
            let _ = self.manager.unregister(*hk);
        }
        GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hotkey someone already has is reported as taken, for the app's notice. The test takes a
    /// rare one first (the first of Win+Shift+F13 to F22 that's free: Win+Shift+F23 is the
    /// Copilot key's), from its own window, and frees it after.
    #[test]
    fn a_taken_hotkey_is_reported_as_taken() {
        let mut first = Hotkeys::new().unwrap();
        let rare = (13..=22)
            .map(|n| Hotkey::parse(&format!("Win+Shift+F{n}")).unwrap().unwrap())
            .find(|h| first.register(Action::Draw, h).is_ok())
            .expect("a free Win+Shift+F13-F22");
        let other = GlobalHotKeyManager::new().unwrap();
        assert!(matches!(
            other.register(to_hotkey(&rare)),
            Err(global_hotkey::Error::AlreadyRegistered(_))
        ));
        drop(other);
        let mut second = Hotkeys {
            manager: GlobalHotKeyManager::new().unwrap(),
            registered: Vec::new(),
        };
        // Windows says "taken" whoever holds it, this process included.
        assert_eq!(
            second.register(Action::Draw, &rare),
            Err(RegisterError::Taken)
        );
        drop(second);
        drop(first);
    }

    /// A hotkey this process holds isn't free, and the nearest free one is suggested instead;
    /// once let go it's free again.
    #[test]
    fn a_held_hotkey_is_not_free_and_a_near_one_is_suggested() {
        let mut held = Hotkeys::new().unwrap();
        let rare = (13..=22)
            .map(|n| {
                Hotkey::parse(&format!("Win+Ctrl+Shift+F{n}"))
                    .unwrap()
                    .unwrap()
            })
            .find(|h| held.register(Action::Draw, h).is_ok())
            .expect("a free Win+Ctrl+Shift+F13-F22");
        assert!(!is_free(&rare));
        let near = nearest_free(&rare, &[]).expect("a free key near it");
        assert_ne!(near, rare);
        assert!(!near.is_ctrl_alt());
        // Overmark's own keys are never suggested for another action.
        let not_that = nearest_free(&rare, &[near]).unwrap();
        assert_ne!(not_that, near);
        drop(held);
        assert!(is_free(&rare));
    }

    #[test]
    fn virtual_keys_are_windows_codes() {
        assert_eq!(vk(Key::Letter('D')), 0x44);
        assert_eq!(vk(Key::Digit(7)), 0x37);
        assert_eq!(vk(Key::F(1)), 0x70);
        assert_eq!(vk(Key::F(24)), 0x87);
        assert_eq!(vk(Key::PrintScreen), 0x2C);
    }

    #[test]
    fn maps_the_defaults_onto_the_crates_hotkeys() {
        let draw = Hotkey::parse("Win+Shift+D").unwrap().unwrap();
        assert_eq!(
            to_hotkey(&draw),
            HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyD)
        );
        let f12 = Hotkey::parse("Ctrl+F12").unwrap().unwrap();
        assert_eq!(
            to_hotkey(&f12),
            HotKey::new(Some(Modifiers::CONTROL), Code::F12)
        );
        let prt = Hotkey::parse("Print Screen").unwrap().unwrap();
        assert_eq!(to_hotkey(&prt), HotKey::new(None, Code::PrintScreen));
    }
}
