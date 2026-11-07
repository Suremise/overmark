# Overmark user guide

Overmark puts a transparent layer over your screen to draw on, then captures what you see, with
your drawing, to the clipboard and a file. This guide covers everything it does. The
[README](../README.md) has the short version, and how to install it.

- [Opening the layer: Live and Frozen](#opening-the-layer-live-and-frozen)
- [Backdrops and guides](#backdrops-and-guides)
- [Tools](#tools)
- [Colours and widths](#colours-and-widths)
- [Undo, redo and Clear](#undo-redo-and-clear)
- [The laser and disappearing ink](#the-laser-and-disappearing-ink)
- [Capturing](#capturing)
- [The toolbar](#the-toolbar)
- [The tray](#the-tray)
- [Settings](#settings)
- [Hotkeys and Print Screen](#hotkeys-and-print-screen)
- [Portable or installed](#portable-or-installed)
- [Privacy and updates](#privacy-and-updates)
- [Troubleshooting](#troubleshooting)
- [Every key](#every-key)

## Opening the layer: Live and Frozen

Overmark waits in the tray, using no CPU, until you call it:

- **Win+Shift+D** opens the layer in **Live** mode: the screen keeps moving underneath (a video,
  a call, an app you're demonstrating), and you draw over it.
- **Win+Shift+F** opens it **Frozen**: the screen is held still as it was when you pressed the
  keys, like a screenshot, so you can mark up that exact moment.

**F** switches between Live and Frozen at any time, and your drawing stays. Freezing from Live
takes the screen as it is at that moment, without the drawing or Overmark's own windows on it.

While the layer is open:

- It takes the mouse, pen and touch, so clicks go to your drawing, not the apps underneath.
- The screen is dimmed slightly, so you can tell the layer is open. Frozen also has a cooler tint
  and a blue snowflake on the toolbar, so you always know whether the screen underneath is still
  moving. The dim can be changed, or turned off, in Settings.
- **Every monitor** gets its own layer. Captures take the monitor the pointer is on.

**Esc** closes the layer, and the drawing goes with it: each time starts clean. (While you're
typing text, Esc finishes the text first; while you're choosing a region, it cancels the region.)

The tray icon and the hotkeys can also open the layer: see [The tray](#the-tray).

## Backdrops and guides

**W** changes what's under your drawing, in turn: Live, Frozen, a **whiteboard** and a
**blackboard**. **Shift+W** goes back. The boards cover the screen with a plain colour, so you
can sketch or explain without anything behind; their colours are in Settings, and each can have a
hotkey of its own that opens the layer straight onto it.

**G** lays **guides** under the ink, in turn: none, a **grid**, **dots** and **ruled lines**.
**Shift+G** goes back. Their spacing is in Settings.

Your drawing stays through every change. Leaving Frozen for a board keeps the frozen picture, so
**W** back to Frozen shows the same moment.

The boards and the guides are part of what people see on a screen share, and part of what you
save, as the drawing is. Settings can keep the guides off shares (**Show guides on screen
shares**) and out of saves (**Include guides in saves**). The dim is only ever over Live and
Frozen, never over a board.

## Tools

Pick a tool with its key, or from the toolbar's **Tool** strip; the toolbar shows the one you have. The
pointer is a crosshair whichever it is.

| Tool | Key | How it works |
|---|---|---|
| **Pen** | P | Draw freehand. Smooth, with the width following your pen's pressure; with a mouse, drawing faster makes the line thinner. |
| **Highlighter** | H | A wide, translucent marker that tints what's under it without hiding text, as a real highlighter does. |
| **Arrow** | A | Drag from the tail to the point. The head grows with the width. |
| **Rectangle** | R | Drag a box. Hold **Shift** for a square. |
| **Ellipse** | E | Drag an ellipse. Hold **Shift** for a circle. |
| **Line** | L | Drag a straight line. Hold **Shift** to snap it to 0°, 45° or 90°. |
| **Text** | T | Click where it goes and type. **Enter** finishes; **Shift+Enter** starts a new line; **Esc** or a click elsewhere finishes too. Backspace, Delete, the arrow keys, Home and End work while you type. |
| **Numbered stamps** | N | Each click places the next number, 1, 2, 3…, for the steps in a bug report or a how-to. |
| **Blur** | B | Drag over what to hide (a password, an email address) before you share or save. It works on a **frozen screen only**: in Live mode the screen keeps changing, so there's nothing fixed to blur. |
| **Eraser** | X | Touch a stroke, shape or text to remove it whole. A pen's eraser end erases too, whatever tool is picked. |
| **Laser pointer** | Z | A glowing dot with a short trail, to point things out. It leaves nothing behind. |

While you're typing text, letter and number keys type; Ctrl's keys (undo, redo, the captures)
still work.

## Colours and widths

**1** to **9** pick a colour from the palette: red, amber, yellow, green, blue, purple, black,
white, and a ninth of your own (teal until you change it). Settings changes any of the nine, and
which one the layer starts with.

**To change the width,** hold the tool's key (P for the pen, H for the highlighter…) and turn the
mouse wheel, or click the width on the toolbar for a slider. The width stays for that tool until you close Overmark. Each tool's starting width
is in Settings; for text it's the font size, for stamps their size, and for blur the size of its
blocks.

## Undo, redo and Clear

- **Ctrl+Z** undoes, and **Ctrl+Y** (or **Ctrl+Shift+Z**) redoes, at least 100 steps back. Every
  stroke, shape, text, stamp, blur and erase is a step.
- **A right-click on the drawing** undoes too, one step a click, as Ctrl+Z does. It's the mouse's
  right button only (not a pen's button or touch), and does nothing while you're drawing. Turn it
  off in Settings (Drawing) if you'd rather it didn't.
- **Delete** clears the drawing on every monitor, as one step: **Ctrl+Z** brings it all back.
- Undo and redo work across monitors: Ctrl+Z undoes your last change, wherever it was.

## The laser and disappearing ink

The **laser pointer** (Z) shows where you're pointing without drawing anything: a glowing dot
with a trail that fades in half a second (the trail's length is in Settings).

**Disappearing ink**, for calls, makes each stroke fade a few seconds after you draw it (3 by
default), so you can keep explaining without stopping to clear. Turn it on and off from the
toolbar, or in Settings (Drawing), where you also set the delay. Faded strokes are gone from undo
too.

## Capturing

With the layer open:

| Keys | What it does |
|---|---|
| **Ctrl+S** | Captures the **monitor the pointer is on**, with your drawing. |
| **Ctrl+Shift+S** | Captures a **region**: drag a rectangle over the picture (in Live mode the picture is taken when you press the keys, then you choose from it). A hint at the top says how, and the region's size in pixels shows as you drag. **Esc** cancels. |
| **Ctrl+C** | Copies the monitor the pointer is on to the clipboard, without saving a file. |
| **Ctrl+Shift+Alt+S** | Captures without your drawing: just the screen. |

By default a capture is **copied to the clipboard and saved** as a PNG, in `Pictures\Overmark`,
named for its time: `2026-10-21 09.42.17.png`. A name that's taken gets a number,
`2026-10-21 09.42.17 (2).png`; nothing is ever overwritten.

A small notice in the corner says where it went: **click it** to open the picture, or
**right-click it** to open its folder. If that can't be done (the picture was moved, say), the
notice says why. If the capture itself can't be made, the notice says so too. It never shows in
the capture itself, and neither does the dim or the toolbar.

In Settings (Capture) you can change:

- **where** captures go (Browse…), back to the default with **Use the default**;
- the **file name pattern**: `{year}`, `{yy}`, `{month}`, `{day}`, `{hour}`, `{minute}`,
  `{second}` and `{millisecond}` are replaced by the capture's time, and anything else is kept
  as it is. Settings shows an example as you type;
- whether a capture **copies and saves**, **copies only** or **saves only**;
- **PNG or JPG**, and JPG's quality;
- whether captures **include the drawing** by default (Ctrl+Shift+Alt+S always leaves it out);
- whether the **notice** shows.

From the tray, **Capture a region…**, or its hotkey **Win+Shift+X** from anywhere, freezes the
screen, lets you choose a region, and closes the layer again once it's captured.

## Recording a video

With the layer open, **Record** on the toolbar (a dot in a ring) records a box of the screen to
a small MP4 video, **with your drawing in it**: you go on drawing while it records. There is no
sound.

1. Press **Record**, then drag a box over the screen, as for Save a region. **Esc** cancels.
2. An outline marks the box, and a small bar sits on its top edge: just above the box, or just
   inside it when there's no room above (a box the size of the screen). Press **Record** on the
   bar to start.
3. While it records, the outline turns red and the bar shows **Pause**, **Stop**, the time
   recorded and **✕**. **Pause** stops the recording until you **Resume** (the video has no gap
   where it was paused). Draw anywhere meanwhile: the outline and the bar never take your clicks
   from the drawing, except on the bar's own buttons.
4. **Stop** saves the video in your captures folder, named as captures are
   (`2026-11-07 10.15.02.mp4`), and the notice says where: **click it** to play the video, or
   **right-click it** to open its folder.

**✕** before you start closes the box. On a recording, it asks first, in the bar: **Discard**
throws the recording away, **Keep** carries on. **Esc** before you start closes the box (the
layer stays open); **Esc** while recording, or anything else that closes the layer, **stops and
saves first**: closing never loses a recording.

The box must be on one monitor. A recording stops by itself if the screen can no longer be
recorded (you lock the PC, or unplug the monitor): what was recorded until then is saved, and the
notice says why. The outline, the bar and the toolbar are never in the video; the screen and your
drawing are.

In Settings (Capture, Recording) you can change the **frame rate** (15, 24 or 30 frames a second;
15 keeps files small), the **quality** (Small, Balanced or Sharp: how much a busy video may take;
a still screen stays small at any of them), whether the **mouse pointer** shows, and the
**encoder**: **Automatic** uses your graphics card's encoder when it can take the video, and
Windows' own otherwise; **Software** always uses Windows' own, which makes smaller videos of a
still screen and works the processor harder. A minute of a small, mostly still box (Task Manager,
say) is under a megabyte or two.

### Recording without drawing

To record an app while you use it (Task Manager's graphs, say), with no drawing layer: **Record a
region…** in the tray's menu, or its hotkey **Win+Alt+V** from anywhere.

1. Drag a box over the screen, as above (**Esc** cancels). Then you're back in the app you were
   using: the box and its bar sit over it, and your clicks and keys go to your apps as usual,
   including inside the box.
2. **Record** on the bar starts; **Pause**, **Resume**, **Stop** and **✕** work as above.
   Clicking the bar never takes the keyboard from your app.
3. **Win+Alt+V again stops and saves** (Esc can't reach the bar, as it never has the keyboard).
   Before you've started, it closes the box.

The bar stays on top of your windows: if another always-on-top window (Task Manager's, say)
covers it, it comes back over it within half a second. Windows' own Start menu, its flyouts,
Win+Tab, a permission prompt and the lock screen still go above it while they're open; a
permission prompt and locking the PC end the recording and save it. Opening the drawing layer
while a screen recording runs leaves the recording alone. With the drawing layer open,
Win+Alt+V only says to close it first (or to use Record on its toolbar, to record with your
drawing); your drawing stays as it is. Pressed while you're choosing the box, it stops
choosing.

The hotkey can be changed or turned off in Settings (General, Hotkeys), as the others can.
Turning off "Show a notice after each capture" also hides the notice after a recording is saved;
a problem (a recording that couldn't start, or stopped by itself) is always told.

On Windows N editions recording needs the Media Feature Pack (Settings › Apps › Optional
features); without it, Overmark says so when you press Record.

## The toolbar

While the layer is open, a small toolbar floats at the top centre of the monitor the pointer is
on, above your drawing. It has:

- **The grip:** drag it to move the toolbar. It stays on its monitor, and Overmark remembers
  where you left it on each monitor.
- **Backdrop:** the one you have, by its icon and colour, so you always know the mode: Live
  (red), Frozen (a blue snowflake), Whiteboard (white) or Blackboard (black). W and Shift+W
  change it; F freezes or unfreezes. Hover over it for its name and what it means.
- **Guides:** the icon of the guides you have: none, Grid, Dots or Ruled lines (G and Shift+G).
- **Tool:** the icon of the tool you have; each has its own key.
- **Colour:** the colour you have, from the palette's nine (1 to 9), each by its name ("Red")
  while it's the default palette's.
- **The width** of the tool you have, in pixels; greyed out for the eraser and the laser, which
  have none. Or hold the tool's key and turn the mouse wheel.

Choosing another backdrop, guides, tool or width never changes the toolbar's size, so its
buttons stay where they are.

**Click Backdrop, Guides, Tool, Colour or the width** for a strip of its choices just below the
toolbar: icons for the backdrops (in their colours), guides and tools (hover over one for its name and key), the
palette's colours, or a slider for the width (drag it, or use the arrow keys, and the width
changes as you go). The one you have is highlighted. The strip stays open while you draw and
choose, so you can try a few; another group's button switches it, and its **up arrow** at the
far right, or clicking its button again, closes it. Esc still closes the layer.

- **Disappearing ink**, on or off (Settings' own setting, saved).
- **Undo** and **Redo**, greyed out when there's nothing to undo or redo, and **Clear**.
- **Save**, **Save a region** and **Copy**, and **More**, with **Save without the drawing**.
- **Record**, which records a video of a box of the screen with your drawing (see Recording a
  video); greyed out while a box is open.
- **Freeze**, an amber pause button while Live, or **Unfreeze**, a green play button while Frozen
  (F does the same); greyed out on the whiteboard and blackboard.
- **Collapse**, which shrinks the toolbar to a small pill showing the grip and the backdrop's icon;
  **Expand** brings it back. It stays collapsed while Overmark runs.
- **Close**, which closes the layer, as Esc does.

Hover over any control to see what it does and its key. Each control does just what its key
does, even while you're typing text.

- It's **hidden from screen sharing and recording**, as the dim is: people on a call see your
  drawing, not the toolbar. It never appears in your captures either.
- Clicking it never draws on the layer underneath, and it never takes the keyboard: your keys
  keep going to the drawing.
- On a narrow monitor its controls wrap onto a second row, so all of them stay in reach.
- Every control has a name a screen reader such as Narrator reads, and Narrator can press it.

## The tray

Overmark's icon in the notification area shows its state: an outlined nib when it's waiting, a
filled nib with an amber dot while you draw Live, and with a pause badge while you draw Frozen.
It follows the taskbar's light or dark theme. If it's hidden, Windows' **^** arrow beside the
clock shows it; drag it onto the taskbar to keep it there. Hover over it to see Overmark's
hotkeys, those with the same modifiers on one line ("Win+Shift: D Draw · F Frozen · X Capture"),
only those that work (one another app has is left out), and, while you draw or record, that you
are.

- **Click** it to draw. Settings chooses what a click does: draw Live (the default), draw on a
  frozen screen, or open Settings.
- **Double-click** it to open Settings.
- **Right-click** it for the menu:

  | Item | What it does |
  |---|---|
  | **Draw** | Opens the layer Live (its hotkey beside it) |
  | **Draw on a frozen screen** | Opens it Frozen |
  | **Capture a region…** | Freezes the screen and lets you choose a region to capture (its hotkey beside it) |
  | **Record a region…** | Lets you choose a box and record it to a video while you use your apps (its hotkey beside it); see [Recording without drawing](#recording-without-drawing) |
  | **Open captures folder** | Opens the folder captures go to |
  | **Settings…** | Opens Settings |
  | **Start with Windows** | Ticked when Overmark starts as you sign in; click to change it |
  | **Check for updates…** | Only when it's turned on in Settings: see [Privacy and updates](#privacy-and-updates) |
  | **About Overmark** | Opens Settings on About |
  | **Quit** | Closes Overmark. Its hotkeys stop working until you start it again. |

Starting Overmark again while it's running doesn't start a second copy: it opens the layer of the
one running.

## Settings

Open Settings by double-clicking the tray icon, or from its menu. **Every change applies at
once** and is saved: there's no Save button. Changes to the drawing (a width, the dim, a colour)
apply even while the layer is open.

- **General:** Start with Windows; what clicking the tray icon does; the **hotkeys** (Draw, Draw
  on a frozen screen, Capture a region, and the whiteboard's and blackboard's, which are off
  until you set them);
  and **Print Screen**. See [Hotkeys and Print Screen](#hotkeys-and-print-screen).
- **Drawing:** the tool the layer starts with; the **palette** (click a colour to change it) and
  the colour it starts with; each tool's **starting width**; the **dim** (0 turns it off);
  **disappearing ink** and its delay; the **laser's trail**; whether a **right-click undoes**; the
  whiteboard's and blackboard's **colours**; and the **guides**: their spacing, whether screen
  shares see them, and whether saves include them.
- **Capture:** where captures go and how they're named and saved: see [Capturing](#capturing).
- **About and advanced:** the version and licence, the author and the link to Overmark's
  repository; whether this copy is portable or installed and
  where its settings file is, with buttons to open the settings and log folders; the links;
  **Check for updates**; **Diagnostic logging** (see [the logs](#the-logs)); the **third-party
  notices** (the open-source parts Overmark is built from, and their licences); and **Reset settings**, which puts every setting back to its
  default and turns Start with Windows off.

Settings lives in a readable file, `settings.toml`, with each setting explained in it (see
[Portable or installed](#portable-or-installed) for where). You can edit it by hand while
Overmark is closed; Settings rewrites it when you change something, so comments you add aren't
kept. A setting the file has wrong (out of range, say) is put right, and Settings says what was
changed at the top of its window.

## Hotkeys and Print Screen

**To change a hotkey,** choose **Change** beside it in Settings (General), pick **Win**,
**Ctrl**, **Alt** and **Shift** as you want them, choose the key, and **Use these keys**.
**Remove** takes a hotkey away (the whiteboard's and blackboard's have none to start with).

- If another app already has the keys, Settings says so and offers the **nearest free one**:
  choose **Use … instead** to take it.
- Two Overmark hotkeys can't share keys.
- **Avoid Ctrl+Alt:** on many keyboards it's AltGr, which types characters such as € and @, so
  Settings warns about it.
- A hotkey needs Win, Ctrl, Alt or Shift with its key, except Print Screen, which can be used
  alone.

**Print Screen** can open the layer too: in Settings, choose **Draw** or **Draw on a frozen
screen** beside **Print Screen** (it's off by default, so Windows keeps the key). Windows 11
sends Print Screen to the Snipping Tool, so to give it to Overmark, turn that off first:

1. Open Windows **Settings** (Win+I).
2. Go to **Accessibility** > **Keyboard**.
3. Turn off **Use the Print screen key to open screen capture**.

On Windows 10 the setting is in
**Ease of Access** > **Keyboard** (**Use the PrtScn button to open screen snipping**).

## Portable or installed

Overmark runs either way, from the same `overmark.exe`:

- **Installed** (the MSI): settings are in `%APPDATA%\Overmark\settings.toml`, with its logs in
  `%APPDATA%\Overmark\logs\`.
- **Portable** (the zip): everything stays in the folder beside `overmark.exe`: `settings.toml`
  and `logs\`. Overmark is portable when that folder has a file named `portable` (the zip has
  one) or a `settings.toml`. It runs from a USB stick, with no admin rights, and writes nothing to
  the registry unless you turn on **Start with Windows** (which adds a value under
  `HKEY_CURRENT_USER\…\Run`, and removes it when turned off).

Settings (About and advanced) says which this copy is, and where its settings are.

## Privacy and updates

**Overmark never sends anything anywhere:** no telemetry, no account, no uploads. Captures stay
where you save them.

**Check for updates** (Settings, About and advanced) is **off by default**, and while it's off
Overmark makes no network connection at all. Turned on, it asks GitHub which release is the
latest, when Overmark starts and when you choose **Check for updates…** in the tray menu. If
there's a newer one, a notice says so; click it to open the release's page. It never downloads
or installs anything itself.

## Troubleshooting

### A hotkey doesn't work

Another app may have it: Overmark shows a notice when it starts ("An Overmark hotkey isn't
working"), and Settings shows the problem beside the hotkey. Click the notice, or open Settings,
and pick other keys: Settings offers the nearest free ones. If no notice showed, check Overmark
is running (its icon is in the tray, or under the **^** beside the clock).

### Windows says "Windows protected your PC"

Overmark isn't code-signed yet, so SmartScreen warns the first time you run the MSI or the exe.
Check the download against the release's `SHA256SUMS.txt` (the [README](../README.md#smartscreen-and-checksums)
shows how), then choose **More info** and **Run anyway**. Some company PCs block unsigned apps
altogether; your IT department can allow it, or install the MSI for you.

### "Overmark can't save its settings"

The folder Overmark keeps its settings in can't be written to: often a portable copy in a
read-only folder, a locked-down drive, or a CD. Overmark carries on with the settings it has, in
memory, until it closes, and writes nowhere else. To keep your changes, move the portable folder
somewhere you can write to, or use the MSI.

If the message is that the settings **can't be read**, `settings.toml` has a mistake (from a hand
edit, say): the notice says where. Overmark uses the defaults and leaves the file alone until you
fix it, or delete it to start again.

### The logs

Overmark keeps a small log, `logs\overmark.log`, beside its settings: `%APPDATA%\Overmark\logs`
when installed, or the `logs` folder beside a portable `overmark.exe`. **Open the log folder** in
Settings (About and advanced) takes you there. It holds at most three files of 1 MB
(`overmark.log`, `overmark.1.log`, `overmark.2.log`) and never records what you drew or typed.

The log is always kept. It records what Overmark did, with times: its version when it starts,
opening and closing the layer, the tools you picked, each capture, and any warnings and errors
(a hotkey another app has taken, a capture that couldn't be saved, a crash).

**Diagnostic logging** (Settings, About and advanced) adds detail for reporting a problem: the
monitors the layer covers, which window has the keyboard, the toolbar being brought back on top,
each hotkey press, choosing a region, and how a capture is copied or saved. Windows are named by
their program and kind, never by their title. It takes effect at once, and is off by default.

To [report a problem](https://github.com/Suremise/overmark/issues), turn **Diagnostic logging**
on, make the problem happen again, then attach the log. You can turn it off afterwards.

### The layer's keys do nothing until you click it

Opened over a full-screen remote desktop or virtual machine window, the layer may not get the
keyboard: the keys you press (Ctrl+Shift+S, Esc, a tool's letter) still go to that window. Click
the layer once (or leave the remote window before you open it) and its keys work.

### After Alt+Tab, the keys go to the other app

With the layer open, Alt+Tab switches to another app beneath it: the layer and the toolbar stay
on top, but your keys go to that app. Click the layer or the toolbar and the keys come back to
Overmark (the toolbar's buttons work either way).

### The layer opens on the wrong monitor, or captures the wrong one

Captures and the toolbar follow the **pointer**: move it to the monitor you want before you
press the keys.

## Every key

**Anywhere** (while Overmark is in the tray; changeable in Settings):

| Keys | What they do |
|---|---|
| **Win+Shift+D** | Open the layer, Live |
| **Win+Shift+F** | Open it on a frozen screen |
| **Win+Shift+X** | Capture a region |
| **Print Screen** | Off by default; can open the layer (see above) |

**While the layer is open:**

| Keys | What they do |
|---|---|
| **P**, **H**, **A**, **R**, **E**, **L**, **T**, **N**, **B**, **X**, **Z** | Pen, highlighter, arrow, rectangle, ellipse, line, text, numbered stamps, blur, eraser, laser |
| **1**–**9** | A palette colour |
| A tool's key held, and the mouse wheel | That tool's width |
| **Shift** (while drawing) | Square, circle, or a line at 0°, 45° or 90° |
| **F** | Freeze or unfreeze |
| **W**, **Shift+W** | The next or previous backdrop: Live, Frozen, whiteboard, blackboard |
| **G**, **Shift+G** | The next or previous guides: none, grid, dots, ruled lines |
| **Ctrl+Z** | Undo |
| Right-click (the mouse) | Undo, unless turned off in Settings |
| **Ctrl+Y**, **Ctrl+Shift+Z** | Redo |
| **Delete** | Clear (Ctrl+Z brings it back) |
| **Ctrl+S** | Save the monitor the pointer is on |
| **Ctrl+Shift+S** | Save a region |
| **Ctrl+C** | Copy the monitor the pointer is on |
| **Ctrl+Shift+Alt+S** | Save without the drawing |
| **Esc** | Close the layer (or finish the text, or cancel the region) |

**While typing text:** **Enter** finishes, **Shift+Enter** starts a new line, **Esc** finishes,
and Backspace, Delete, the arrows, Home and End edit.
