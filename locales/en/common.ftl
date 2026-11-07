# Strings used in more than one place: the app's name, the modes, colours and key names.
# Conventions: docs/ARCHITECTURE.md, "Strings and languages".

app-name = Overmark
# A build for another system: Overmark v1 runs on Windows only.
app-windows-only = Overmark runs on Windows only in this version.

## The mode badge on the toolbar, and its tooltip. $key: the freeze key ("F").

mode-live = Live
mode-live-tooltip = Live: the screen keeps moving underneath. Press { $key } to freeze it.
mode-frozen = Frozen
mode-frozen-tooltip = Frozen: the screen is held still underneath. Press { $key } to let it move again.

## What's under the ink: the backdrop (W cycles them) and the guides (G cycles them)

backdrop-live = Live
backdrop-frozen = Frozen
backdrop-whiteboard = Whiteboard
backdrop-blackboard = Blackboard
guides-off = No guides
guides-grid = Grid
guides-dots = Dots
guides-lines = Ruled lines

## The colour palette (keys 1 to 9): each colour's accessible name and tooltip

colour-red = Red
colour-amber = Amber
colour-yellow = Yellow
colour-green = Green
colour-blue = Blue
colour-purple = Purple
colour-black = Black
colour-white = White
colour-custom = Custom colour
# $colour: one of the names above; $key: its number key.
colour-tooltip = { $colour } ({ $key })

## Key names, as hotkeys and tooltips show them. A hotkey is its keys joined with "+",
## for example "Win+Shift+D". Letter and number keys show as themselves.

key-ctrl = Ctrl
key-alt = Alt
key-shift = Shift
key-win = Win
key-esc = Esc
key-enter = Enter
key-space = Space
key-delete = Delete
key-tab = Tab
key-print-screen = Print Screen
key-insert = Insert
key-home = Home
key-end = End
key-page-up = Page Up
key-page-down = Page Down
key-left = Left
key-right = Right
key-up = Up
key-down = Down

## Window names, which screen readers and Windows' window lists use (the windows have no
## title bar).

window-selection-name = Overmark selection
window-notice-name = Overmark notice
window-record-outline-name = Overmark recording outline
window-record-bar-name = Overmark recording controls
