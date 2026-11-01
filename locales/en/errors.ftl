# Problems with Overmark's own folder and settings, shown on start as a tray notice
# and in Settings. $path: a folder or file; $detail: what went wrong, as the system says it.

## The app's folder

error-folder-not-writable-title = Overmark can't save its settings
error-portable-not-writable = Overmark is running portable, but its folder { $path } can't be written to. Settings are kept in memory until it closes, and nothing is saved there.
error-app-folder-not-writable = Overmark's folder { $path } can't be written to. Settings are kept in memory until it closes, and nothing is saved there.
error-app-folder-unknown = Overmark can't work out where to keep its files: { $detail }
# For error-app-folder-unknown's $detail. $reason: what went wrong, as the system says it.
error-exe-folder-unknown = the folder Overmark runs from isn't known: { $reason }
error-no-appdata = the %APPDATA% folder isn't set

## The settings file

error-settings-unreadable-title = Overmark's settings can't be read
error-settings-unreadable = { $path } can't be read, so the defaults are used until it's fixed: { $detail }
error-settings-not-saved = Settings couldn't be saved to { $path }: { $detail }
# Where in the file a problem is, for error-settings-unreadable's $detail.
error-settings-at = line { $line }, column { $column }: { $detail }

## Settings that loaded with something ignored or changed: shown in Settings and logged.

# $key: the setting's dotted name ("drawing.colour").
warning-settings-unknown-key = { $key } isn't an Overmark setting, so it's ignored.
# $value: what the file has; $used: the nearest allowed value, used instead.
warning-settings-out-of-range = { $key } = { $value } is out of range, so { $used } is used.
warning-settings-bad-pattern = The file name pattern can't be used, so the default is: { $detail }
# $key: the setting ("backdrops.whiteboard_colour").
warning-settings-translucent-backdrop = { $key } had some transparency, so it's used fully opaque: a whiteboard or blackboard covers the screen.
# $count: how many colours the file has; $size: how many it needs.
warning-settings-palette-size =
    { $count ->
        [one] The palette has 1 colour but needs { $size }; the rest are the defaults.
       *[other] The palette has { $count } colours but needs { $size }; missing ones are the defaults, extra ones are dropped.
    }

## A colour in the settings file that can't be read. $text: what the file has, quoted.

error-not-a-colour = { $text } isn't a colour (#RRGGBB or #RRGGBBAA)

## The capture file name pattern

# $code: the code as written ("{hours}").
pattern-unknown-code = { $code } isn't a file name code. The codes are {"{"}year{"}"}, {"{"}yy{"}"}, {"{"}month{"}"}, {"{"}day{"}"}, {"{"}hour{"}"}, {"{"}minute{"}"}, {"{"}second{"}"} and {"{"}millisecond{"}"}.
pattern-unclosed = A "{"{"}" in the file name pattern has no "{"}"}" after it.

## Captures

error-open-folder = { $path } couldn't be opened: { $detail }

## The command line: shown on the console, when Overmark is started from one.
## $argument: what was given, quoted; $reason: what went wrong, as the system says it.

cli-dim-needs-number = Overmark: --dim needs a number from 0 to 0.8
cli-unknown-argument = Overmark: unknown argument { $argument }, ignored
cli-no-pointer-mouse = Overmark: the mouse can't come as pointer messages: { $reason }
cli-dim-running = Overmark: the running Overmark didn't quit, so --dim can't be used
cli-already-running = Overmark: already running, so it was asked to open
cli-instance-check-failed = Overmark: couldn't check for another instance: { $reason }
# $folder: the logs folder.
cli-no-log-file = Overmark: no log file in { $folder }: { $reason }
