# Hotkey problems: shown in Settings, beside the hotkey, and as a tray notice.
# $hotkey: a hotkey as the app shows it ("Win+Shift+D").

hotkey-taken = { $hotkey } is already used by another app. Pick another key.
# A default that was taken, so Overmark picked the nearest free one. $used: the one it uses.
hotkey-taken-replaced = { $hotkey } is already used by another app, so Overmark uses { $used } instead.
# $action: the name of the other Overmark hotkey ("Draw").
hotkey-duplicate = { $hotkey } is already Overmark's hotkey for { $action }.
hotkey-needs-modifier = A hotkey needs { key-ctrl }, { key-alt }, { key-shift } or { key-win } with its key ({ key-print-screen } can be used alone).
hotkey-ctrl-alt-warning = Avoid { key-ctrl }+{ key-alt }: on many keyboards it's AltGr, which types characters.
hotkey-print-screen-help = Windows 11 opens the Snipping Tool with { key-print-screen }. To give the key to Overmark, turn off "Use the Print screen key to open screen capture" in Windows Settings › Accessibility › Keyboard.

## The tray notice when a hotkey can't be registered

hotkey-notice-title = An Overmark hotkey isn't working
# $action: which hotkey ("Draw", "Draw on a frozen screen").
hotkey-notice-body = { $hotkey } ({ $action }) is used by another app. Click here to pick another key in Settings.
