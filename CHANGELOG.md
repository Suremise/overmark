# Changelog

All notable changes to Overmark are recorded here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Overmark follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-11-07

Record a part of the screen to a small MP4 video, while you draw on it or while you use your
apps.

**Not signed yet:** Windows SmartScreen may warn the first time you run the installer or the exe
("Windows protected your PC"). Check the download against the release's `SHA256SUMS.txt` first
(`sha256sum -c SHA256SUMS.txt --ignore-missing`, or compare `Get-FileHash <file>` with it), and
if it matches, choose **More info**, then **Run anyway**. Installing it over 0.2.1 keeps your
settings.

### Added

- **Record a video** of part of the screen with your drawing in it: Record on the toolbar, drag
  a box, then record, pause, stop or discard from the small bar on the box's edge, and go on
  drawing while it records. Videos are small MP4s, saved in the captures folder with the usual
  notice; there's no sound, and the outline, the bar and the toolbar are never in the video.
  Closing the layer while recording saves first, and a recording that can no longer see the
  screen (a locked PC, an unplugged monitor) saves what it has and says why.
- **Record without drawing:** Record a region… in the tray menu, or Win+Alt+V, records a box of
  the screen while you go on using your apps; press the hotkey again to stop and save. The
  hotkey can be changed in Settings.
- Settings > Capture > Recording sets the frame rate, the quality, whether the mouse pointer
  shows, and the encoder (your graphics card's, or Windows' own).
- A recording's problems are always told, even with capture notices turned off. On Windows N
  editions recording needs the Media Feature Pack, and Overmark says so.

### Changed

- The tray icon's tooltip groups the hotkeys that share their modifiers on one line
  ("Win+Shift: D Draw · F Frozen · X Capture"), so every hotkey fits, and says when Overmark is
  recording.

## [0.2.1] - 2026-11-04

Choosing a region works on every monitor, the toolbar keeps working after Alt+Tab, and a
Diagnostic logging setting helps when reporting a problem.

**Not signed yet:** Windows SmartScreen may warn the first time you run the installer or the exe
("Windows protected your PC"). Check the download against the release's `SHA256SUMS.txt` first
(`sha256sum -c SHA256SUMS.txt --ignore-missing`, or compare `Get-FileHash <file>` with it), and
if it matches, choose **More info**, then **Run anyway**. Installing it over 0.2.0 keeps your
settings.

### Added

- **Diagnostic logging** in Settings > About and advanced, off by default: it adds detail to
  the log (the monitors the layer covers, which window has the keyboard, hotkey presses,
  choosing a region, how a capture is saved) for reporting a problem. Windows are named by
  their program, never their title, and the log never holds what you drew or typed. The user
  guide says how to use it when you report a problem.
- The user guide's troubleshooting covers the layer's keys doing nothing when it opens over a
  full-screen remote desktop or virtual machine window (click the layer once), and the keys
  going to another app after Alt+Tab.

### Fixed

- Choosing a region (Ctrl+Shift+S, or the Capture a region hotkey) works on any monitor, not
  just the one the pointer was on, and nothing is drawn, erased or undone on another monitor
  while you choose.
- After Alt+Tab with the layer open, the toolbar keeps working: its clicks no longer go to the
  drawing beneath it, and a click on it gives the keyboard back to the layer.

## [0.2.0] - 2026-11-01

Right-click to undo, a hotkey to capture a region, and the tray icon's tooltip stays.

**Not signed yet:** Windows SmartScreen may warn the first time you run the installer or the exe
("Windows protected your PC"). Check the download against the release's `SHA256SUMS.txt` first
(`sha256sum -c SHA256SUMS.txt --ignore-missing`, or compare `Get-FileHash <file>` with it), and
if it matches, choose **More info**, then **Run anyway**. Installing it over 0.1.0 keeps your
settings.

### Added

- A right-click on the drawing undoes, as Ctrl+Z does (the mouse's right button only, never
  mid-stroke). It's on by default; turn it off in Settings > Drawing.
- A hotkey for **Capture a region**, Win+Shift+X by default: change it in Settings > General
  beside Draw's. The tray menu and tooltip show it.

### Fixed

- The tray icon's tooltip stays after drawing: it went once the icon first changed (opening
  the layer, a balloon, a theme or DPI change) and didn't come back.
- Settings' number boxes beside the sliders are one width in each group, so the sliders line
  up, and they no longer grow as you type. A number typed with its unit ("12 px") now counts.

## [0.1.0] - 2026-11-01

The first release: draw over your screen, Live or Frozen, then capture it.

**Not signed yet:** Windows SmartScreen may warn the first time you run the installer or the exe
("Windows protected your PC"). Check the download against the release's `SHA256SUMS.txt` first
(`sha256sum -c SHA256SUMS.txt --ignore-missing`, or compare `Get-FileHash <file>` with it), and
if it matches, choose **More info**, then **Run anyway**.

### Added

- Draw over your screen in **Live** mode, with the screen still moving underneath, or on a
  **Frozen** screen, held still as a screenshot is; switch between them with F, and your
  drawing stays.
- Overmark waits in the tray using no CPU: **Win+Shift+D** opens the layer Live and
  **Win+Shift+F** Frozen, in well under a tenth of a second, on every monitor; **Esc** closes it
  and the drawing goes with it. A slight dim says it's open, and isn't shown on screen shares.
  If another app has a hotkey, a notice says so.
- A **whiteboard** and a **blackboard** to draw on (W, and Shift+W to go back), and a **grid,
  dots or ruled lines** under the ink (G, Shift+G), shown on screen shares and in saves unless
  you turn them off in Settings. Your drawing stays through every change, and a frozen screen
  comes back as it was when you return to it from a board. Each board can have its own hotkey.
- **Tools:** a pen with pressure (with a mouse, speed sets the width), a highlighter that tints
  without covering text, arrows, rectangles, ellipses, lines (Shift for squares, circles and
  45°), text, numbered stamps, blur for a frozen screen, an eraser that removes whole strokes,
  and a laser pointer that leaves nothing behind.
- **Drawing with a mouse, a pen or touch:** the pen's pressure and tilt, and its eraser end
  for erasing; keys pick the tools (P, H, A, R, E, L, T, N, B, X, Z) and 1-9 the colours;
  hold a tool's key and turn the wheel to change its width. Overmark gives the focus back to
  the window you were in when the layer closes.
- **A toolbar** at the top of the screen you're on: the backdrop and guides, the tool, colour and
  width, undo and redo, clear, save, save a region, copy, save without the drawing, freeze and
  close, each with its key in its tooltip. Drag it where you like (it stays there on that
  monitor) or collapse it to a small pill. Screen shares and recordings never show it, and
  screen readers read every button, each colour by its name. It can turn disappearing ink on
  and off. Only the toolbar itself takes the pointer: you can start drawing right up to its
  edge, even where a tooltip shows below it.
- Click the toolbar's backdrop, guides, tool, colour or width for a **strip of choices** just
  below it, with icons: it stays open while you draw, so you can try a few, and its up arrow
  closes it. The width's is a slider that steps as the wheel does, and the arrow keys move it.
  The backdrop, guides and tool show their current choice's icon (each backdrop in its own
  colour, Frozen a blue snowflake) and the width fits its widest value, greyed out for the
  eraser and the laser, so choosing another never resizes the toolbar. Freeze is an amber
  pause button while Live and a green play button while Frozen.
- Choosing a region says how (drag, Esc cancels) and shows its size as you drag. When a capture
  can't be made, or its notice can't open the picture or its folder, the notice says why.
- **Undo and redo** of at least 100 steps, including Clear: Ctrl+Z, and Ctrl+Y or
  Ctrl+Shift+Z.
- **Disappearing ink** for calls: strokes fade a few seconds after you draw them.
- **Captures** copied to the clipboard and saved as PNG (or JPG), by default to
  `Pictures\Overmark`, with names like `2026-10-21 09.42.17.png`.
- **Ctrl+S** saves the monitor under the cursor with your drawing, **Ctrl+Shift+S** a region you
  drag (Esc cancels it), **Ctrl+C** copies, and **Ctrl+Shift+Alt+S** saves without the drawing. A
  small notice says where it went: click it to open the picture, right-click for its folder.
  Captures never include the dim, the toolbar or the notice.
- `settings.toml`, readable, with every setting explained, beside the exe for a portable copy or
  in `%APPDATA%\Overmark\` when installed.
- **Portable or installed:** put an empty file named `portable` (or a `settings.toml`) beside
  `overmark.exe` and it keeps everything beside itself; otherwise it uses `%APPDATA%\Overmark\`.
  A small log, `logs\overmark.log`, sits beside the settings. If the folder can't be written to,
  or the settings file can't be read, Overmark says so and carries on with the defaults, without
  writing anywhere else or over your file.
- The app icon and tray icons, which show whether the layer is open, Live or Frozen.
  `overmark.exe` carries the app icon and its version, which Explorer and the file's Details
  show.
- **The tray** shows whether the layer is open, Live or Frozen, in the taskbar's light or dark
  theme; hover over it for Overmark's hotkeys, each with what it does. Click it to draw (or to
  draw Frozen, or open Settings: your choice), double-click it for Settings; right-click for
  Draw, Draw on a frozen screen, Capture a region…, Open captures folder, Settings…, Start with
  Windows, About and Quit.
- **Settings**, where every change applies at once: the hotkeys (with a message, and the nearest
  free key offered, when another app has one), Print Screen as an optional key, Start with
  Windows, the tools, palette, widths, dim (0 is off), disappearing ink, laser, backdrops and
  guides, where and how captures are saved, and About with the author, the repository's link,
  the third-party notices, the log folder and Reset.
- **A user guide** (`docs/user-guide.md`): every tool and key, capturing, the toolbar, the tray
  and Settings, portable or installed, Print Screen, and troubleshooting.
- **Check for updates**, off by default: when you turn it on, Overmark reads the latest release
  on GitHub and shows a link to it. It never downloads or installs anything, and with it off
  Overmark makes no network requests at all.
- **Two downloads:** an MSI that installs per user with no admin rights, or per machine for IT
  (`ALLUSERS=1`, silently with `/qn`), and a portable zip that keeps everything beside the exe.
  Each release lists their SHA-256 checksums and every third-party licence.
