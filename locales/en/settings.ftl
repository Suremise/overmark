# The Settings window: its sections, field labels and their help.

settings-title = Overmark Settings

## Sections

settings-general = General
settings-drawing = Drawing
settings-capture = Capture
settings-about = About and advanced

## General

settings-start-with-windows = Start with Windows
settings-start-with-windows-help = Overmark starts in the tray when you sign in to Windows.
# $reason: what went wrong, as the system says it.
settings-autostart-failed = Start with Windows couldn't be changed: { $reason }
settings-tray-click = Clicking the tray icon
settings-tray-click-help = A double-click always opens Settings.
settings-hotkeys = Hotkeys
settings-hotkeys-help = They work in any app, while Overmark is in the tray.
settings-hotkey-draw = Draw
settings-hotkey-draw-frozen = Draw on a frozen screen
settings-hotkey-change = Change
# $action: which hotkey ("Draw").
settings-hotkey-change-named = Change the hotkey for { $action }
settings-hotkey-none = None
settings-hotkey-clear = Remove
# $action: which hotkey ("Draw").
settings-hotkey-clear-named = Remove the hotkey for { $action }
settings-hotkey-key = Key
settings-hotkey-use = Use these keys
# A free hotkey near the one asked for. $hotkey: it ("Win+Alt+D").
settings-hotkey-use-instead = Use { $hotkey } instead
# The tray couldn't register it, though no other app was found to have it.
settings-hotkey-failed = Windows wouldn't register { $hotkey }. Pick another key.
# The settings file has something that isn't a hotkey. $hotkey: its text.
settings-hotkey-invalid = "{ $hotkey }" isn't a hotkey Overmark can use. Pick another key.
settings-print-screen = { key-print-screen }
settings-print-screen-off = Off (Windows keeps it)
settings-print-screen-taken = { key-print-screen } is in use by Windows or another app. See the note above to free it.
settings-check-updates = Check for updates
settings-check-updates-help = Off by default. When on, Overmark only reads the latest release on GitHub; it never downloads or installs anything by itself.

## Drawing

settings-default-tool = Default tool
settings-palette = Colour palette
settings-palette-help = Keys 1 to 9 pick these colours. Click one to change it.
settings-default-colour = Starting colour
# $n: a palette colour's number, the key that picks it (1 to 9).
settings-colour-n = Colour { $n }
settings-default-widths = Default widths
settings-width-pen = Pen
settings-width-highlighter = Highlighter
settings-width-shapes = Shapes and arrows
settings-width-text = Text size
settings-width-stamp = Numbered stamps
settings-width-blur = Blur
settings-dim = Dim the screen
settings-dim-help = How much the screen darkens while you draw. 0 turns it off.
settings-dim-off = Off
# $percent: a whole number (35).
settings-percent = { $percent }%
settings-disappearing-ink = Disappearing ink
settings-disappearing-ink-help = Strokes fade a few seconds after you draw them, for calls.
settings-disappearing-ink-delay = Fade after
# $seconds: a number of seconds.
settings-seconds =
    { $seconds ->
        [one] 1 second
       *[other] { $seconds } seconds
    }
settings-laser-trail = Laser trail length
# $ms: a number of milliseconds.
settings-ms = { $ms } ms
# A slider's box, to type its value in, for screen readers. $setting: the slider's name.
settings-value-box = { $setting }: type a value
settings-right-click-undoes = Right-click undoes
settings-right-click-undoes-help = A right-click on the drawing undoes, as Ctrl+Z does. The mouse's right button only, not a pen's button or touch.

## Backdrops and guides

settings-backdrops = Backdrops
settings-whiteboard-colour = Whiteboard colour
settings-blackboard-colour = Blackboard colour
settings-hotkey-whiteboard = Draw on the whiteboard
settings-hotkey-blackboard = Draw on the blackboard
settings-guides = Guides
settings-guide-spacing = Guide spacing
settings-guides-on-shares = Show guides on screen shares
settings-guides-in-saves = Include guides in saves

## Capture

settings-save-location = Save location
settings-save-location-browse = Browse…
settings-save-location-default = Pictures\Overmark (default)
settings-save-location-reset = Use the default
settings-file-name = File name pattern
# $example: the pattern applied to now ("2026-10-21 09.42.17.png").
settings-file-name-example = For example: { $example }
settings-file-name-help = The user guide lists the date and time codes a pattern can use.
settings-after-capture = After a capture
settings-after-capture-copy-and-save = Copy and save
settings-after-capture-copy = Copy only
settings-after-capture-save = Save only
settings-format = Format
settings-format-png = PNG
settings-format-jpg = JPG
settings-jpg-quality = JPG quality
settings-include-drawing = Include the drawing by default
settings-toast = Show a notice after each capture

## About and advanced

settings-version = Version
# $version: the app's version ("0.1.0").
settings-version-value = Overmark { $version }
settings-licence = Licence
settings-licence-value = MIT
settings-author = Author
# $name: the author's name, not translated.
settings-author-value = © 2026 { $name }
settings-repository = Repository
settings-third-party = Third-party notices
settings-third-party-help = The open-source parts Overmark is built from, and their licences.
settings-third-party-crt = Overmark also includes Microsoft's C runtime, so it needs nothing else installed. That part is under Microsoft's terms, not an open-source licence: part 3 of the notices says more.
settings-links = Links
settings-link-website = Website
settings-link-source = Source code
settings-link-releases = Releases
settings-link-issues = Report a problem
settings-mode = Running
settings-mode-portable = Portable
settings-mode-installed = Installed
settings-settings-file = Settings file
settings-open-settings-folder = Open the settings folder
settings-open-log-folder = Open the log folder
settings-reset = Reset settings
settings-reset-help = Every setting goes back to its default, and Start with Windows is turned off.
settings-reset-confirm = Put every setting back to its default? This can't be undone.
settings-reset-yes = Reset
settings-cancel = Cancel
settings-close = Close
settings-warnings-title = Some settings in the file were changed or ignored
# The window can't start. $dir: the fonts folder; $files: the fonts looked for.
settings-no-font = Settings can't show any text: none of { $files } is in { $dir }.

## The shared components' own words (crates/overmark/src/ui/widgets.rs)

widget-copy-path = Copy the full path
# A dropdown that carries its own label, closed: "Tool: Pen".
widget-dropdown-labelled = { $label }: { $value }
