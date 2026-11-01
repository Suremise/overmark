//! Whether a hotkey is free: registered with `RegisterHotKey` for this thread and straight
//! away unregistered (never a hook). Settings uses it to suggest the nearest free key.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

/// The id the probe registers with: in the range an app may use (0 to 0xBFFF), and unlike any
/// `global-hotkey` gives (it uses the hotkey's hash, but on its own window, not this thread).
const PROBE_ID: i32 = 0xB0B0;

/// Whether `vk` with these modifiers can be registered now: no app (this one included) has it.
pub fn is_free(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u32) -> bool {
    let mut mods = MOD_NOREPEAT;
    for (on, m) in [
        (ctrl, MOD_CONTROL),
        (alt, MOD_ALT),
        (shift, MOD_SHIFT),
        (win, MOD_WIN),
    ] {
        if on {
            mods |= m;
        }
    }
    // SAFETY: registers for this thread (no window) and unregisters the same id at once.
    unsafe {
        if RegisterHotKey(None, PROBE_ID, HOT_KEY_MODIFIERS(mods.0), vk).is_err() {
            return false;
        }
        let _ = UnregisterHotKey(None, PROBE_ID);
    }
    true
}
