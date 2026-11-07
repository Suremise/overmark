# The tray icon: its tooltip and its right-click menu.

## Tooltip: Overmark's name (app-name) or what it's doing on the first line, then the hotkeys
## that work, those sharing their modifiers on one line ("Win+Shift: D Draw · F Frozen"). A
## tooltip holds 127 characters: lines that don't fit are left out from the end, so keep these
## short.

# A line of hotkeys sharing their modifiers. $modifiers: the keys held ("Win+Shift"); $keys:
# each hotkey's tray-tooltip-key, joined by tray-tooltip-separator.
tray-tooltip-group = { $modifiers }: { $keys }
# One hotkey on its line. $key: its last key ("D"); $action: its short name below.
tray-tooltip-key = { $key } { $action }
tray-tooltip-separator = {" · "}
# The actions' short names, for the tooltip only (the menu has the full ones).
tray-tooltip-draw = Draw
tray-tooltip-draw-frozen = Frozen
tray-tooltip-capture = Capture
tray-tooltip-record = Record
tray-tooltip-live = Overmark: drawing (Live)
tray-tooltip-frozen = Overmark: drawing (Frozen)
# Drawing on a whiteboard or blackboard; $backdrop is its name (backdrop-whiteboard, ...).
tray-tooltip-board = Overmark: drawing ({ $backdrop })
# A video is being recorded.
tray-tooltip-recording = Overmark: recording
# Capture a region, as a hotkey notice names it (the menu's item without its "…").
tray-tooltip-capture-region = Capture a region
# Record a region, as a hotkey notice names it.
tray-action-record-region = Record a region
# When no hotkey could be registered, or there are none.
tray-tooltip-no-hotkey = Overmark: click to draw

## Menu. The app adds the hotkeys beside the first three items.

tray-draw = Draw
tray-draw-frozen = Draw on a frozen screen
tray-capture-region = Capture a region…
tray-record-region = Record a region…
tray-open-captures = Open captures folder
tray-settings = Settings…
tray-start-with-windows = Start with Windows
tray-check-updates = Check for updates…
tray-about = About Overmark
tray-quit = Quit

## What left-clicking the tray icon does (Settings, General)

tray-click-live = Draw (Live)
tray-click-frozen = Draw on a frozen screen
tray-click-settings = Open Settings

## When something from the menu doesn't work. $reason: what went wrong, as the system says it.

tray-settings-failed-title = Settings couldn't open
tray-settings-failed = Overmark couldn't start its Settings window: { $reason }
tray-open-captures-failed-title = The captures folder couldn't be opened
# $folder: the captures folder.
tray-open-captures-failed = Explorer couldn't open { $folder }.

## The update check (only when it's turned on in Settings: it reads the latest release on
## GitHub and never downloads anything). $version: a version ("0.2.0").

update-available-title = A new Overmark is out
update-available-body = Overmark { $version } is available. Click here to see what's new and download it.
update-latest-title = Overmark is up to date
update-latest-body = You have the latest version, { $version }.
update-failed-title = Overmark couldn't check for updates
# $reason: what went wrong, as the system says it.
update-failed-body = { $reason }
# For update-failed-body's $reason. $status: an HTTP status code ("404").
update-failed-status = GitHub answered { $status } instead of the latest release.
# $version: this version ("0.1.0").
update-failed-version = This version ({ $version }) can't be compared with GitHub's.
