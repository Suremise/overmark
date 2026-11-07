# Overmark architecture

How Overmark is built, and why. The README says what it does; this file records how, and each
decision as it is made.

## Layout

```
crates/
  overmark-core/  # platform-free: strokes, tools, geometry, rendering, undo history,
                  # eraser hit-testing, the settings model. Builds and tests on any OS.
  overmark-win/   # Windows only (behind cfg(windows)): overlay windows, capture, hotkeys,
                  # pointer input, display affinity, tray, clipboard, notifications,
                  # autostart, portable/installed detection.
  overmark/       # the exe: wires core and platform, the toolbar and settings UI.
                  # windows_subsystem = "windows" in release builds.
locales/en/       # Fluent strings (English in v1)
installer/        # the WiX installer definition
assets/           # icons (SVG masters, exported ICO/PNG), README screenshots
docs/             # this file, TESTING.md, the user guide, RELEASING.md
```

Every dependency's version is set once, in `[workspace.dependencies]` in the root `Cargo.toml`;
a crate is added there only when the code needs it. The toolchain is pinned in
`rust-toolchain.toml`.

## Decisions

- **Licences:** everything Overmark ships is MIT or equally
  permissive: crates, fonts, icons and data. `deny.toml` is the gate, run in CI with
  `cargo deny check`: it allows MIT, Apache-2.0 (and its LLVM-exception variant), BSD-2-Clause,
  BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0, CC0-1.0, Unicode-3.0 and Unicode-DFS-2016, and a dual
  licence passes when one side is on that list. Everything else fails, copyleft included
  (GPL, LGPL, AGPL, MPL, EPL, CDDL, MS-RL, OFL, the Ubuntu Font Licence). Unknown registries
  and git sources are denied, and advisories (unmaintained and yanked crates too) are errors.
  Legible's MPL exception isn't taken. **One exception, a deliberate choice (1 Nov): Microsoft's C
  runtime**, linked into `overmark.exe` statically (`.cargo/config.toml`, `+crt-static`) and
  unmodified, under Microsoft's redistribution terms ("Distributable Code" in the Visual Studio
  licence), not an MIT-style licence. Every exe built with Microsoft's compiler carries or needs
  it; linking it in means no Visual C++ redistributable (an admin install) is needed, so the
  per-user MSI with no admin and the portable zip work on any PC. `cargo deny` and `cargo about`
  don't see it (it isn't a crate), so THIRD-PARTY-NOTICES names it in a part of its own
  (`scripts/third-party-notices.ps1`), and the README's licence line has a footnote.
- **The MSI has no licence page** (1 Nov, as Legible and Replica): it uses no
  WixUI, whose dialogs and art are MS-RL, so Windows shows its own progress window. `LICENSE.txt`
  is installed beside the exe, and the README and About show it (docs/RELEASING.md, The MSI).
- **The platform split:** `overmark-core` has no Windows dependency, and CI's Linux job runs its
  tests to prove it. Windows code lives only in `overmark-win`, behind `cfg(windows)`.
- **Fonts: Windows' own, loaded at run time; nothing bundled** (see Crate check, Fonts).
- **Text tool: `parley` for layout and shaping, `swash` for glyphs** (see Crate check, Text),
  with **the system font handed in at run time** and none bundled (see Tools, Text).
- **Strokes: a port of perfect-freehand in `overmark-core`** (`freehand`), not the
  `perfect_freehand` crate (see Crate check, and Strokes).
- **No network calls and no telemetry.** Global hotkeys use `RegisterHotKey` only, never a
  keyboard or mouse hook.
- **The ink layer: a layered window presented with `UpdateLayeredWindow`,** one top-most window
  per monitor, fed from tiny-skia's pixels, its empty pixels at alpha 1 so it takes every click
  (revising the ink-layer spike's DirectComposition pick: see The resident app). Both paths draw as
  fast; DirectComposition's device costs 85-110 ms to make and 30+ MB to keep, which the
  100 ms open and the 30 MB idle budgets can't both afford.
- **Clean grabs (freezing from Live, saves) through the Magnification API, in a helper
  process** (`overmark.exe --grabber`, alive only while the overlay is open): a magnifier
  control renders the monitor without our windows while they stay on screen, so a share never
  sees the ink blink, and the memory the runtime keeps goes when the helper exits. Core then
  composites the strokes onto it (`overmark_core::composite`). Opening Frozen grabs with GDI
  before any window of ours shows. Without the helper, the fallback excludes the ink layer for
  a GDI grab, which a share can see as a one-frame blink (see Saving).
- **The backdrop is a choice, not a two-way mode:** the overlay shows core's `Backdrop`
  (Live, Frozen, Whiteboard or Blackboard): F swaps Live and Frozen, W cycles the backdrops and
  G the guides (see Backdrops and guides).
- **Desktop Duplication, not Windows Graphics Capture,** wherever a capture API is needed: no
  border on Windows 10 or 11 (see Capture API).
- **The highlighter can darken the live screen:** a second DirectComposition layer per monitor
  composed with `MIN_BLEND` (see The highlighter), made when the highlighter is first used, not
  at idle.
- **Resident in the tray, nothing of the overlay at idle:** the app waits in
  `MsgWaitForMultipleObjectsEx` with no timeout, and makes the layers when a hotkey comes (see
  The resident app).
- **Per-monitor DPI awareness v2:** every layer is sized in physical pixels (see DPI).
- **Recording: Desktop Duplication into Media Foundation's H.264 encoder**, the GPU's or
  Microsoft's software one, writing an MP4; Media Foundation loaded at run time so Windows N
  editions still start, and no new crate (see Recording).

## The ink-layer spike

The spike is `crates/overmark-win/examples/ink_spike` on top of `overmark_win::win32` (the safe
wrappers, the only `unsafe` in Overmark). Run it with
`cargo run -p overmark-win --release --example ink_spike -- <draw|bench|capture|dpi>` (and `draw --bench`, `draw --smoke`); its first
lines say what each mode does. Measured on 31 Oct 2026 on the development PC: one ASUS XG32VC,
**2560x1440 at 165 Hz** (6.06 ms a frame), 100 % scale, an NVIDIA RTX 5070 Ti, a Ryzen 7
9800X3D, Windows 11 25H2 (build 26200.9711). No 4K monitor is attached, so the bench draws into
a **3840x2160 layer** placed at the monitor's top-left (it runs past the monitor's edges, but the
renderer, the copy to DWM and DWM's surface are all 4K) and reports the native size scaled by
2.25 as a cross-check.

### Q1. The render path

Both paths draw on the CPU with tiny-skia into a premultiplied BGRA image, as `overmark-core`
will render saves, so live and saved ink come from the same code. They differ in how the
pixels reach DWM:

- **`UpdateLayeredWindow`** (`win32::layered`): the image is a DIB section in a layered window;
  `UpdateLayeredWindowIndirect` hands DWM the dirty rectangle.
- **DirectComposition** (`win32::dcomp`): a premultiplied flip-model swap chain made for
  composition is the only content of a `WS_EX_NOREDIRECTIONBITMAP` window; each frame uploads
  the dirty rectangle into a GPU copy of the image (`UpdateSubresource`), copies that into the
  back buffer and presents with the dirty rectangle. Direct2D isn't needed: drawing stays in
  tiny-skia, and the GPU only presents.

`ink_spike bench 4`: each frame waits for DWM's next composition (`DwmFlush`), draws a fast
scribble's next stretch (six segments, about 4,000 px/s, as a 1 kHz mouse or a pen gives between
frames, 6 px wide) and presents only what it touched. Once a second it clears the whole layer
and presents all of it: an undo's or a clear's worst frame. Milliseconds of CPU time, about 655
frames per run:

| Path | Size | Draw p50 / p99 | Upload p99 | Present p50 / p99 | **Frame p50 / p99 / max** | Full clear + present |
|---|---|---|---|---|---|---|
| UpdateLayeredWindow | 2560x1440 | 0.045 / 0.150 | | 0.053 / 0.272 | 0.099 / 0.347 / 0.630 | 1.4 |
| UpdateLayeredWindow | 3840x2160 | 0.043 / 0.096 | | 0.052 / 0.207 | **0.096 / 0.275 / 0.433** | 3.0 |
| DirectComposition | 2560x1440 | 0.036 / 0.161 | 0.016 | 0.050 / 0.104 | 0.093 / 0.260 / 0.495 | 1.1 |
| DirectComposition | 3840x2160 | 0.037 / 0.175 | 0.015 | 0.049 / 0.102 | **0.094 / 0.276 / 0.562** | 2.1 |

**Both pass** the bar (a 4K frame in about half of 144 Hz's 6.9 ms, 3.47 ms) many times over:
a drawing frame's p99 is 0.28 ms at 4K (0.59-0.78 ms if the native size is scaled by 2.25),
and not one of about 2,600 frames ran past a refresh. Even the worst frame, a full 4K clear
and present, is 2.1 ms (DirectComposition) or 3.0 ms (layered).

**The app's own drawing path, Live and Frozen.** `bench` draws straight into the layer; the
app's path does more per sample: the pen's strokes go into a layer of their own, and the ink
layer's dirty rows are rebuilt from the frozen frame, the highlighter (darken) and the strokes
(`draw`'s `recompose`, core's `composite`). `ink_spike draw --bench 2` times exactly that path,
six samples a frame as above, with nothing copied whole per sample (only the dirty rows are
read, the other layers borrowed in place). Frame cost in milliseconds, about 330 frames each:

| Path | Size | Live pen p50 / p99 | Live highlighter | Frozen pen | Frozen highlighter |
|---|---|---|---|---|---|
| DirectComposition | 2560x1440 | 0.086 / 0.285 | 0.117 / 0.331 | 0.092 / 0.282 | 0.173 / 0.462 |
| DirectComposition | 3840x2160 | 0.103 / 0.289 | 0.136 / 0.342 | 0.102 / 0.258 | 0.183 / 0.358 |
| UpdateLayeredWindow | 2560x1440 | 0.082 / 0.271 | 0.118 / 0.341 | 0.084 / 0.271 | 0.183 / 0.388 |
| UpdateLayeredWindow | 3840x2160 | 0.088 / 0.299 | 0.124 / 0.348 | 0.087 / 0.278 | 0.189 / 0.446 |

Worst p99 0.46 ms: well inside the bar in every mode. (A review caught an
earlier `recompose` that copied the whole highlighter and frozen frame for every sample, about
33 MB each at 4K; this table is the measurement that was missing then.)

**No added input lag.** `bench` also switches a small square on and off 40 times and times each
present to the first frame Windows Graphics Capture sees it in, beside an ordinary GDI window
doing the same as a baseline:

| | p50 | p95 | max |
|---|---|---|---|
| Baseline: a GDI window | 17.1 ms | 23.6 ms | 28.1 ms |
| UpdateLayeredWindow | 17.4 ms | 23.0 ms | 24.2 ms |
| DirectComposition | 17.1 ms | 22.2 ms | 23.4 ms |

The ink layer reaches the screen exactly as fast as any app does. The 17 ms is DWM and the
capture pipeline (about three frames at 165 Hz), not the ink layer.

**The pick: DirectComposition** (revised later: the ink layer is now the layered window,
with alpha-1 empty pixels for input, because DirectComposition's device can't be both absent at
idle and ready within 100 ms of a hotkey; see The resident app. DirectComposition stays for the
`MIN_BLEND` highlighter.) The speed is a tie, so the deciding facts were:

- **Input.** The ink layer must take the mouse and pen everywhere, but Windows hit-tests a
  layered window by its pixels: a click on a fully transparent pixel goes to the window below
  (`ink_spike capture`, Q1: `WindowFromPoint` on a transparent pixel finds the layered window's
  neighbour, and the DirectComposition window itself). The layered path would need a
  1/255-alpha wash over the whole monitor, which tints captures too.
- **The highlighter needs it** (`MIN_BLEND`, below), and so does anything else composed by DWM
  rather than blended by us.
- **Full redraws are cheaper** (2.1 against 3.0 ms at 4K) and don't block on a GDI copy.

The costs: a D3D11 device per process (device loss must be handled: recreate and re-upload on
`DXGI_ERROR_DEVICE_REMOVED`), and the layers took 170-340 ms to come up in `draw`, and 740 ms
on a first run after a build (two D3D devices, Desktop Duplication and a magnifier per monitor). The 100 ms budget from hotkey to layer means
the resident app makes the layers at start-up and only shows and hides them.

### Q2. What screen sharing sees

`ink_spike capture` puts three windows near the top-left: the ink layer (a magenta square), a
dim veil (black at 63 %) and a toolbar (cyan), the last two excluded with
`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. Windows Graphics Capture (what Teams, Zoom
and the Snipping Tool use) and Desktop Duplication both see **the ink, and the screen as it was
under the dim and the toolbar**: `Q2: ok` on every run. A frozen frame is drawn in the ink layer
itself, so it's seen the same way. `WDA_EXCLUDEFROMCAPTURE` needs Windows 10 2004 (build 19041)
or later; earlier builds aren't supported.

### Q3. The capture API

The capture border is drawn by DWM on screen, so Desktop Duplication can look for it while a
capture session runs: yellow pixels along the screen's edges were **64 with no capture running
(the desktop's own), 64 with a Windows Graphics Capture asking for no border, and 2,780 with one
asking for the border** (`IsBorderRequired`). So on this Windows 11 build an unpackaged app can
turn the border off, with no prompt. On **Windows 10** it can't:
`GraphicsCaptureSession.IsBorderRequired` (and `GraphicsCaptureAccess.RequestAccessAsync`)
arrived in build 20348, and Windows 10's last builds are 19041-19045, so every Windows Graphics
Capture there shows the yellow border. Desktop Duplication (Windows 8 and later) never shows
one, and it respects `WDA_EXCLUDEFROMCAPTURE`.

**The pick: DXGI Desktop Duplication** (`win32::capture::dd`), with a device on the adapter
that drives the monitor, wherever a capture API is needed. Clean grabs for saves and freezes go
through the Magnification API instead (see Saving), with Desktop Duplication as the fallback.
What it taught:

- **The first frame stamped after a present can still be a composition from before it.**
  `grab_settled` keeps taking frames until none comes for 25 ms or one comes 25 ms after the
  change.
- **The first frame after duplication starts can be empty**, so the app makes its
  duplications at start-up.
- **A turned monitor's frames come unturned** (the panel's own orientation) with
  `DXGI_OUTDUPL_DESC.Rotation`: `capture::unrotate` turns them to the desktop, as Microsoft's
  DesktopDuplication sample maps its dirty rectangles, and a frame that still doesn't match
  the monitor's size is an error, never a mislabelled image. The mapping is unit-tested on
  synthetic frames; **not tried on a turned monitor** (none here).

### Saving (Q4)

A save needs the screen as it is now, without the ink, the highlighter, the dim or the toolbar,
and then core draws the strokes onto it; "save without drawing" is that image alone. A freeze
needs the same clean image, so the strokes stay separate and can be undone or erased over it.
All of it must happen without the ink leaving the screen, or a call's audience sees it blink.

- **Rejected: exclude the ink layer only while grabbing** (`WDA_EXCLUDEFROMCAPTURE` on the ink
  for one Desktop Duplication grab). `ink_spike capture`, Q4: the grab is clean only 6-10 times in
  10, but a share watching the screen (a second Windows Graphics Capture) saw **the ink vanish
  for a frame in 2-5 of every 10 saves**.
- **Rejected: grab with the ink showing and solve for the screen under it.** Exact under
  translucent ink, but under opaque ink the screen was never on screen, and the
  highlighter's darken blend loses channels: no clean image under strokes, so neither "save
  without drawing" nor a clean freeze.
- **The pick: the Magnification API** (`win32::magnifier`). A magnifier control at 1:1 renders
  the monitor with a filter list that leaves out the windows we name (`MagSetWindowFilterList`,
  `MW_FILTERMODE_EXCLUDE`) and hands the image to a callback (`MagSetImageScalingCallback`).
  Our windows never change, so nothing blinks. `ink_spike capture`, Q4c, with **opaque** ink over
  a backdrop of a known colour: **10 of 10 grabs clean** (the backdrop under the ink, no dim, no
  toolbar), **no frame without the ink** for the share, **24-40 ms** a grab at 2560x1440.
  Outside our windows it matches Desktop Duplication's frame of the same moment except where
  the screen changed between the two grabs (about 1,200-2,600 px of 3.7 M, all inside apps
  that were redrawing): it is the live screen, a playing video included.

  What it needs, found by trying: the control renders only while its host window is on a
  monitor, and its image is the host's size (a 1x1 host gives a 1x1 image; off-screen, the
  callback never runs; `PrintWindow` of the control gives black). So the host is
  **monitor-sized, opacity 0, click-through, never activated and excluded from capture**: no one
  sees it, and it's shown only for the grab. The callback is handed an internal window, not the
  control, so grabs run one at a time on the UI thread.

  **The risk:** `MagSetImageScalingCallback` is documented as deprecated. It works on Windows 11
  25H2; on Windows 10 it is reported to, but that isn't tested here. If it ever gives nothing,
  `grab` returns `None` and the app falls back to Desktop Duplication with our layers excluded
  for the grab (a possible one-frame blink for a share, as above).

Then core composites (`overmark_core::composite`, tested): the clean image, the highlighter's
layer by `darken` (what `MIN_BLEND` shows live), then the ink's layer by `over`. In **Frozen**,
the frozen frame is the clean image and saves need no grab at all (7-9 ms in `draw`).

`ink_spike draw --smoke` runs it end to end on both ink paths, with an opaque pen stroke and a
highlighter stroke drawn in, and checks every save:

- Live S: the stroke composited, the highlighter darkening;
- Live W: no ink under the stroke;
- frozen S and W, the same;
- after C (clear) while frozen, W is still clean: the frozen frame never had the ink in it.

Live grabs took 30-37 ms.

One thing to keep out of the ink layer: anything that must be shared but never saved (a laser
pointer, an eraser's outline) goes in a layer of its own, which the magnifier leaves out too.

### DPI (Q5)

The process is per-monitor DPI aware v2 (the spike calls `SetProcessDpiAwarenessContext`; the
app's manifest will declare it). Every monitor and window rectangle is then in physical pixels,
so a layer per monitor made over `rcMonitor` covers it pixel for pixel, and the scale only sizes
UI. `ink_spike dpi`: the one monitor here is 2560x1440 at 96 dpi (100 %), and its layer's
client area is 2560x1440, window DPI 96. **150 % is not tested here:** this PC has one monitor at
100 %, and the spike doesn't change display settings. The code has no scale arithmetic in the
layer's size. The magnifier works in physical pixels too (its image was 2560x1440 here).

### The highlighter (Q6)

`UpdateLayeredWindow` and an ordinary DirectComposition visual blend only source-over, so the
first plan was a translucent tint in Live and a true multiply in Frozen and saves. **Better
found:** DirectComposition's `DCOMPOSITION_COMPOSITE_MODE_MIN_BLEND` (Windows 8.1 and later)
composes a visual with whatever is behind the window, other apps included, by each channel's
minimum: a darken blend. `ink_spike capture`, Q6, over a backdrop of RGB (200, 100, 50):
highlighter yellow gives (200, 100, 0), exactly the darken result (plain alpha-over gives
yellow), white leaves the backdrop as it is, and transparent pixels leave it too. Captures see
it the same way.

So the highlighter can be a **second layer per monitor under the ink, composed with
`MIN_BLEND`, live**: text under it stays dark, paper turns yellow, like a real highlighter.
Darken differs from multiply only on mid-tones (multiply is darker there); core can draw saves
with tiny-skia's `BlendMode::Darken` so a save looks like the screen, or `Multiply` if wanted.
Not tried yet: a `MIN_BLEND` child visual in the ink layer's own tree instead of a window of
its own (the `draw` mode uses a separate window; H switches to it).

### An eye check

`cargo run -p overmark-win --release --example ink_spike -- draw` covers each monitor with the
DirectComposition layer, takes the mouse and pen through `WM_POINTER` (no hooks), and on exit
prints its frame cost and the time from the oldest input sample to its present. `draw layered`
runs the other path to compare. Keys: Esc quits, C clears the strokes, H switches pen and
highlighter, F freezes (a clean grab under the strokes), D dims, S saves each monitor's clean
screen with the strokes composited to `%TEMP%\overmark-spike-save-N.bmp`, and W saves it
without the strokes to `%TEMP%\overmark-spike-clean-N.bmp`. It was checked by eye on
31 Oct 2026, drawing over a playing video: "drawing over a playing video looks smooth". That
was before the magnifier: the freeze then baked the strokes in, and S saved the screen with
them.

## The resident app

`crates/overmark` is the app: `main.rs` starts logging (to stderr and the log file: see
Portable and installed, Logs) and calls `overmark::run`; the loop is in `src/app.rs`. It waits in
`MsgWaitForMultipleObjectsEx` with no timeout (messages for its windows, and the named events
another launch sets), so it uses no CPU in the tray.

- **The tray** (`overmark_win::win32::tray`, adapted from Replica's tray scaffold): its
  icon (idle, Live or Frozen), its tooltip from the strings, a right-click menu and balloons
  (see The tray and Settings). A click on the icon does what `general.tray_click` says. The
  icon comes back when Explorer restarts.
- **One instance per session:** a named mutex (`Local\Overmark.Instance`) and three named
  events. A second launch sets `Open` (or `OpenFrozen` with `--frozen`), lets the first take the
  foreground (`AllowSetForegroundWindow`) and exits; `overmark.exe --quit` sets `Quit`.
- **Hotkeys** (`overmark_win::hotkeys`) through `global-hotkey`, which calls `RegisterHotKey` on
  a hidden window of ours and never hooks the keyboard; its handler stamps each press's arrival
  for the timing. The settings' text ("Win+Shift+D") is parsed by `overmark_core::hotkey`,
  which also shows hotkeys with the strings' key names and flags Ctrl+Alt (AltGr). A hotkey
  that can't be registered is logged and told in a tray balloon (`hotkey-notice-title`,
  `hotkey-notice-body`), never silently: tried by holding Win+Shift+D in another process first
  (`hotkey taken by another app`, the notice shown, Win+Shift+F still registered).
- **Checked on this PC** (Windows 11 25H2, build 26200): **Win+Shift+D and Win+Shift+F are both
  free**; they register at start-up. Windows 10 isn't tested here. (Win+Shift+F23 is taken: it is
  what the Copilot key sends.) **Capture a region's Win+Shift+X** was free too, where
  Win+Shift+C, R and W were already taken; Win+Shift+A is Windows 11's own (it focuses an
  on-screen tip), and Win+Shift+S the Snipping Tool's.
- **Per-monitor DPI awareness v2** at start-up (`SetProcessDpiAwarenessContext`); the manifest
  will say the same once the exe has its resources.

### The overlay

`overmark_win::overlay` covers each monitor with up to four windows, bottom to top:

1. **the backdrop layer** (any backdrop but Live): the frozen frame, or a whiteboard's or
   blackboard's colour, opaque, visible to capture (a share sees the board);
2. **the veil**: black at the dim's level (`drawing.dim`, 0 is off), and in Frozen a slightly
   cooler tint (RGB 12, 28, 64, at the dim's level or at least 12 %), **excluded from capture**;
   never over a board;
3. **the guides' layer** (while there are guides): transparent but for the guides,
   click-through, visible to capture or excluded as `guides.show_on_shares` says;
4. **the ink layer**: takes the mouse, pen and keys. F swaps Live and Frozen, W and G cycle the
   backdrop and the guides, keeping the strokes; Esc closes, and the strokes vanish
   (`Session::clear_session`).

The tint lives in the veil rather than in the frozen frame, so a share sees neither the dim nor
the tint, and the frozen frame stays the clean image a save starts from.

**Everything is made on open and dropped on close.** Measured warm, per monitor at
2560x1440:

| | Time |
|---|---|
| A DirectComposition surface (its D3D11 device and swap chain) | 85-111 ms |
| A layered window, its DIB, filled and presented | 7 ms |
| A magnifier, then its first grab | 75 ms, then 24-32 ms |
| A GDI grab of the screen | 22-24 ms |

and at idle, private memory and working set: a process with only a hidden window 1.8 / 10 MB;
with a D3D11 device and a DirectComposition layer 66 / 38 MB; with a full layer's pixels and a
magnifier as well 153 / 85 MB. So the layers are layered windows (no device), Frozen opens with
a GDI grab taken before any window of ours shows (so it's clean), and freezing from Live, which
needs a clean grab with our windows showing, goes to the grabber.

**The grabber is a process of its own** (`overmark_win::grabber`, `overmark.exe --grabber`):
the Magnification API keeps 35-50 MB in the process that used it, and more with each grab,
even after every magnifier is gone and `MagUninitialize` has run (measured: 1.5 MB, then 70 MB
with a magnifier, 35 MB once it's dropped, 52 MB if it had grabbed; in the app, 121 MB private
after 16 opens with a freeze each). The app starts the helper once the overlay shows, it makes
a magnifier per monitor, answers grabs over its stdin and stdout (pixels and all), and exits
when the overlay closes, taking that memory with it.

### The budgets

`cargo run -p overmark --release --example resident_check` (with no Overmark running: it stops
if one is) drives the real exe from outside: it
measures it idle, opens the layer by posting `WM_HOTKEY` to the app's own hotkey window (what
Windows sends on a key press), presses F and Esc by posting keys to its ink layer (no global
input), reads the app's own timing from its log, and checks with Windows Graphics Capture, as a
share sees the screen, that a backdrop of a known colour under the dim looks as it is. Every
wait is for a condition with a deadline (a log line, a window, a capture frame); only the idle
measurements are spans of time. A capture is judged only after a **barrier**: once the app logs
the scene ready (it logs after DWM has composed the layers), a tiny marker window of the
check's own changes colour, and the first frame showing the new colour is the one judged
(`capture::wgc::frame_after_barrier`), so no frame queued before the scene was ready can pass
(tested with the case of a clean frame after the request but before readiness, then only
leaked ones, which must fail). A **control** runs first: a dim of the check's own that isn't
excluded must be caught (it shows as (130, 65, 33) over (200, 100, 50)), and the backdrop must
read clean once it's gone. The
app it starts is stopped however the check ends, a panic included, and only that process
(`OVERMARK_CHECK_FAIL=after-open` fails on purpose to show it: the app and its grabber are gone
after). `--quick` (2 rounds, 3 s idles) is for repeated runs: 5 under load, beside the full suite
running in a loop, all ok, with the wait's tests stress-run 50 times there. Run on 31 Oct 2026
(2560x1440 at 165 Hz):

| Budget | Measured |
|---|---|
| Hotkey to a visible layer under 100 ms (`WM_HOTKEY`'s arrival to DWM's first frame with the layers) | **Live 19-44 ms** (medians 24-27), **Frozen 45-76 ms** (medians 56-71: the GDI grab is about 22 of it; 75 ms the slowest, under load), 8 opens each in each of four runs |
| Idle memory under 30 MB | **2.1-2.2 MB private** (1.0-1.5 MB working set) at start and after 16 opens with a freeze in each Live one |
| No CPU when idle | **0.0 ms of CPU in 10 s**, before and after use |
| F | Live to Frozen 38-66 ms (the grabber's clean grab), Frozen to Live 3-6 ms |
| The dim excluded from capture | the veil is shown (opacity 89 of 255, the dim 0.35), and a share sees the backdrop at (199, 100, 50) for (200, 100, 50); the frozen frames, 9 of 9, are the backdrop as it is too |

The app logs each open (`opened backdrop=… grab_ms layers_ms visible_ms`, a warning past
100 ms) and each F. Once the overlay closes it hands its pages back to Windows
(`EmptyWorkingSet`), so the working set is the tray's again; private memory is what it
committed, and the table gives both.

**The dim's default is 0.35** (`DEFAULT_DIM`, unchanged), chosen on 31 Oct after an eye
comparison with Win+Shift+S didn't settle it ("They all look the same … Just decide on the
default"). They looked the same because an Overmark was already running: each later
`overmark.exe --dim …` only asked it to open, so the first level stayed. Now a launch with
`--dim` asks the running one to quit and takes its place. The veil itself does what it says,
measured over a grey backdrop of 200 with the veil not excluded, so Desktop Duplication sees
it: 0.30 gives 140, 0.35 gives 130, 0.50 gives 100, each exactly 200 x (1 - dim).
Windows' snip dim wasn't measured: its overlay can only be closed by input to another app's
window, which the checks never send. So 0.35, the starting point, and Settings' dim
slider (0 is off) lets people tune it.

## Crate check

The candidate crates, checked on 31 Oct 2026 against crates.io: the latest stable version,
its release date, its licence, all-time downloads in millions as a rough health signal. Their
**whole dependency trees** were checked too: a scratch project with every crate below (the
picks, at the features Overmark would use) and this repo's `deny.toml` gives
`advisories ok, bans ok, licenses ok, sources ok` across 446 packages. Those trees' licences are
MIT, Apache-2.0, BSD-2/3-Clause, BSL-1.0, ISC, Zlib, 0BSD and Unicode-3.0, with two to note:

- `self_cell` is `Apache-2.0 OR GPL-2.0-only`: it passes on the Apache side, and we ship it under
  that.
- `memchr` and `byteorder-lite` are `Unlicense OR MIT`: they pass on the MIT side.

Only `overmark-core`'s and `overmark-win`'s own crates are in `Cargo.toml` today; each change adds
what it needs, and `cargo deny check` checks the tree again then.

| Need | Crate | Version | Released | Licence | Downloads | Health | Verdict |
|---|---|---|---|---|---|---|---|
| Windows APIs | windows | 0.62.2 | 2025-10-06 | MIT OR Apache-2.0 | 387 M | active (Microsoft) | keep; only the Win32 features `overmark-win` calls |
| Smooth pressure strokes | perfect_freehand | 0.1.1 | 2026-01-27 | MIT | 0.002 M | one author, two releases, about 2,000 downloads | **replaced, done:** perfect-freehand 1.2.3 is ported into `overmark_core::freehand` on `kurbo` and checked against the library's own output (see Strokes) |
| Curves, smoothing, shapes | kurbo | 0.13.1 | 2026-05-13 | Apache-2.0 OR MIT | 61 M | active (Linebender) | keep |
| Rasterising strokes | tiny-skia | 0.12.0 | 2026-02-02 | BSD-3-Clause | 63 M | active again (now Linebender) | keep |
| Text layout and shaping | parley | 0.12.0 | 2026-10-09 | Apache-2.0 OR MIT | 5 M | active (Linebender), pre-1.0 | **the pick** (below) |
| Glyph outlines and rasterising | swash | 0.2.10 | 2026-07-17 | Apache-2.0 OR MIT | 17 M | active | **the pick** (below) |
| (text, not picked) | cosmic-text | 0.19.0 | 2026-04-22 | MIT OR Apache-2.0 | 11 M | active (System76) | no: pulls `ttf-parser` (through `fontdb`) |
| (text, not picked) | fontdue | 0.9.5 | 2026-10-19 | MIT OR Apache-2.0 OR Zlib | 12 M | active | no: `ttf-parser`, no shaping |
| (text, not picked) | ab_glyph | 0.2.32 | 2025-09-28 | Apache-2.0 | 51 M | maintained | no: `ttf-parser`, no shaping |
| Screen capture | windows-capture | 2.0.1 | 2026-08-08 | MIT | 2.4 M | active, one main author | **not used:** the ink-layer spike picked DXGI Desktop Duplication, called through `windows` (no border on Windows 10 either; see Capture API). The spike's Windows Graphics Capture observer calls `windows` directly too |
| Global hotkeys | global-hotkey | 0.9.0 | 2026-10-16 | Apache-2.0 OR MIT | 7.1 M | active (Tauri) | keep: on Windows it is `RegisterHotKey`, no hook. Or call `RegisterHotKey` directly through `windows` if only one message loop is wanted |
| Tray icon | tray-icon | 0.26.1 | 2026-10-07 | MIT OR Apache-2.0 | 40 M | active (Tauri) | keep (or copy Replica's own Win32 tray, `replica-win/src/tray.rs`) |
| Tray menus | muda | 0.21.2 | 2026-10-08 | Apache-2.0 OR MIT | 47 M | active (Tauri) | keep, with tray-icon |
| Clipboard images | arboard | 3.7.0 | 2026-10-15 | MIT OR Apache-2.0 | 61 M | active (1Password) | keep |
| PNG/JPG | image | 0.25.10 | 2026-03-10 | MIT OR Apache-2.0 | 234 M | active | keep, default features off, `png` and `jpeg` only |
| Settings file | serde | 1.0.229 | 2026-07-18 | MIT OR Apache-2.0 | 1,624 M | active | keep |
| Settings file | toml | 1.1.8 | 2026-10-09 | MIT OR Apache-2.0 | 1,078 M | active | keep |
| Strings | fluent | 0.17.0 | 2025-05-23 | Apache-2.0 OR MIT | 17 M | slow but maintained (Mozilla) | use `fluent-bundle` 0.16.0 (2025-05-22, 23 M) with `unic-langid`, as Replica and Legible do; check it builds on each toolchain bump |
| Toolbar and settings UI | egui | 0.37.0 | 2026-10-15 | MIT OR Apache-2.0 | 29 M | active | keep; **`default_fonts` off** (below) |
| Toolbar and settings UI | eframe | 0.37.0 | 2026-10-15 | MIT OR Apache-2.0 | 23 M | active | keep, `default-features = false` with `accesskit` and `glow` (and `clipboard` if the settings need it) |
| Settings' folder picker | rfd | 0.17.2 | 2026-01-12 | MIT | 41 M | active | keep, `default-features = false` (on Windows the system's folder dialog through COM), as Replica uses it |
| Update check (opt-in) | (none) | | | | | | WinHTTP, part of Windows, through `windows`: no HTTP or TLS crate ships |
| Logging | tracing | 0.1.44 | 2025-12-18 | MIT | 994 M | active (Tokio) | keep |
| Logging | tracing-subscriber | 0.3.23 | 2026-03-13 | MIT | 710 M | active | keep, default features off, `fmt` and `std` |
| Logging | tracing-appender | 0.2.5 | 2026-04-17 | MIT | 140 M | active | likely not needed: Replica's `logging.rs` (one file per process, with clean-up) needs no appender |
| The exe's icon and version resource | embed-resource | 3.0.12 | | MIT | | | build-only (`crates/overmark/build.rs` compiles an `.rc` with the Windows SDK's `rc.exe`; copied from Legible's), never shipped |
| Licence checks | cargo-deny, cargo-about | | | | | | build-only tools, never shipped |

**Replace one:** `perfect_freehand`. It works, but it is a young port with one author and almost
no users, and strokes are the heart of the app. perfect-freehand's algorithm is small and
MIT-licensed: port it into `overmark-core` (credited in THIRD-PARTY-NOTICES), where we can test
it and tune it. **Done** (Strokes, below).

**Watch:** `parley` (pre-1.0, its API moves), `fluent`
(slow releases).

### Text: parley and swash

The Text tool needs fonts found and read, text shaped (kerning, ligatures, any script), laid out
in lines, and glyphs rasterised. The three crates first considered all read fonts with
**`ttf-parser`, which RUSTSEC-2026-0192 marks unmaintained**: the gate fails them as it stands,
and the advisory names `skrifa` as the replacement. `cosmic-text` also uses `skrifa` and
`harfrust`, but its font database (`fontdb`) still pulls `ttf-parser`.

**The pick: `parley`** (Linebender, the same family as `kurbo` and `tiny-skia`) for shaping and
layout, built on `skrifa` and `harfrust`, and **`swash`** for glyph outlines and rasters, also on
`skrifa`. Neither pulls `ttf-parser`. egui 0.37 reads fonts with `skrifa` as well, so the UI and
the Text tool share one font stack. Glyphs go into `tiny-skia` as outlines (`swash`'s scaler
gives paths), so text gets the same anti-aliasing and colour as strokes. It all lives in
`overmark-core`, as the platform split asks; only finding the font file is Windows'.

### Fonts

**The trap:** egui's `default_fonts` feature pulls `epaint_default_fonts` (0.37.0), which
bundles Ubuntu-Light (the Ubuntu Font Licence) and Noto Emoji (OFL-1.1) beside Hack (MIT, with
Bitstream Vera's terms) and emoji-icon-font (MIT). The first two fail §3.1. The crate declares
`(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`, so `cargo deny check` fails as soon as
anything in the tree turns the feature on. `eframe` and `egui` are always taken with
`default-features = false`, so it never gets that far.

**The recommendation: Windows' own fonts, loaded at run time, nothing bundled.** Copy Replica's
`crates/replica/src/ui/fonts.rs` (generic, "ready to copy"): Segoe UI Variable, then Segoe UI,
with Segoe UI Symbol for the symbols they lack, a semibold family, and Cascadia Mono or
Consolas, all from `%WINDIR%\Fonts`. It fails only when no text font at all is found. The Text
tool reads the same files through `parley`/`swash`, so a note looks like the toolbar. Fonts that
are part of Windows are never redistributed, so there is nothing to license; a bundled font
would need an MIT-style licence (the OFL doesn't qualify) and would add size for no gain. The
Mac port later uses the system font the same way.

## Strings and languages

English is the only language in v1, but nothing user-visible is written in the code (a
deliberate choice): every string comes from Fluent (`.ftl`) files in `locales/<language>/`,
through `overmark_core::i18n`. The module is Replica's (`replica-core/src/i18n/mod.rs`, copied
as it is, with its tests); only `embedded.rs` is Overmark's. Logs are diagnostics and stay in
English.

- **The files are compiled in** (`include_str!` in `crates/overmark-core/src/i18n/embedded.rs`),
  so the portable exe needs no extra files. They're split by where the strings show:
  `common.ftl` (the app's name, the Live and Frozen badges, colour names, key names),
  `tray.ftl`, `toolbar.ftl` (tools, buttons and tooltips), `toasts.ftl`, `settings.ftl`,
  `hotkeys.ftl` (hotkey conflicts) and `errors.ftl` (the app's folder and settings file).
- **The API:** `i18n::init(tag)` once at start-up, with the Windows UI language that
  `overmark-win` reads (`en-GB`, `fr_FR` and the like); until then the strings are English.
  Then `tr("tray-quit")`, or `tr!("tray-tooltip-shortcut", action = name, hotkey = text)` with arguments (text or
  numbers; `tr_args` is the function form). `i18n::languages()` says which languages are in use.
  For a language other than the global one, build an `i18n::Localizer`.
- **Choosing the language:** an exact match for the tag, then the same language without a
  region, then any region of it, then English. A key missing in the chosen language falls back
  to English key by key.
- **Nothing fails silently:** a key missing in every language shows as the key itself, never as
  an empty string, and is logged (a `tracing` warning), as is a message that doesn't format (a
  missing argument shows as `{$name}`). Syntax errors and duplicate keys are logged when the
  strings load. The tests below catch all of these first.
- **Keys and shortcuts in text:** a tooltip takes its key as `$key` (`tool-pen-tooltip`, "Pen
  (P): …"), so it follows the binding the app actually uses. Key names (`key-ctrl`,
  `key-shift`, …) are messages of their own: the app joins them with "+" to show a hotkey, and
  other strings refer to them (`{ key-shift }`), so a translation renames a key once.
- No Unicode isolation marks around placeables: they show as stray characters in tooltips and
  logs. A right-to-left language may want them back.

**Tests** (`i18n::embedded`, run on any OS): every file parses with no duplicate keys; every key
has a value; no string keeps a carriage return (Git may check files out with CRLF); every
`.ftl` file on disk is embedded; other languages only have keys English has; **every literal key
the code passes to `tr(`, `tr_args(` or `tr!(` exists in English** (so pass keys as literals: a
key built at run time escapes that check); and the keys the toolbar, tray and Settings need
exist.

**No words in the code:** `no_text_is_written_into_ui_calls`, in the same
module, reads every crate's code (not its tests, examples, benches or build scripts) and fails on a string
literal with words in it given straight to a call that shows text: egui's labels, buttons,
tooltips and accessible names, the shared components (`ui::widgets`), window titles (`with_title(`,
`set_title(`, and `title: Some(` in a window's `Options`), the tray's balloons, the capture
notice, and `eprintln!`/`println!`. A window's title comes from Fluent through
`win32::window::title(key)` (translated once per process); the checks find those windows by the
same Fluent names.

**What the check doesn't see** (it reads lines, it isn't a parser): a literal on a later line
than its call (`"Cancel"` on the line after `button(`); a literal that's all lower case,
hyphens or underscores, which passes as an id (`"tray-click"`, but also `"cancel"`); text
built elsewhere and passed in by a variable; and calls not in its list (`UI_CALLS`), where new
kinds of UI call belong. Text without words (`"+"`) passes, and a line can opt out with
`// not ui text`. Review covers the rest.

**What stays English, on purpose:** the logs (diagnostics); messages from outside Overmark,
passed through as they come (Windows' error texts, already in the user's language, and the toml
parser's description of a mistake in `settings.toml`, inside Overmark's own words); the
values the settings file is written in (keys, `"Win+Shift+D"`, `"png"`), which are its format;
and **the comments Overmark writes in `settings.toml`** (a deliberate choice, 1 Nov):
they document a technical file whose keys are English anyway, as the logs are diagnostics.
Settings is the translated place to change settings.

**To add a string:** put `key = Text` in the `.ftl` file for where it shows, with a `#` comment
naming each `$argument`, and call `tr("key")` or `tr!("key", name = value)` with the key as a
literal. Keys are lower case with hyphens and start with their file's area (`tray-`, `tool-`,
`toolbar-`, `toast-`, `settings-`, `hotkey-`, `error-`). A new file goes in `LOCALES` in
`embedded.rs` (a test fails until it does).

**To add a language:** copy `locales/en/` to `locales/<tag>/` (`de`, `pt-BR`), translate the
values and leave the keys, then add a `Locale { tag, files }` entry for it in `LOCALES`. A key
left out falls back to English. Run `cargo test -p overmark-core`.

## Settings

`overmark_core::settings` is the model of `settings.toml`: what's in it, its
defaults, and reading and writing it at a path it's given. Where the file lives (beside the exe
when portable, `%APPDATA%\Overmark\` when installed) is `overmark-win`'s business. The
loading, the documented writer and the safe save follow Replica's `settings.rs`.

- **Sections:** `[general]` (start with Windows, the tray click, the hotkeys as text such as
  `"Win+Shift+D"`, Print Screen), `[drawing]` (default tool, a 9-colour palette, the default
  colour, the dim, disappearing ink, the laser trail) with `[drawing.widths]` per tool,
  `[capture]`, `[advanced]` (check for updates and diagnostic logging, both off) and `[toolbar.positions]` (keyed by the
  platform's monitor id).
- **Every key is documented:** `Settings::save` writes `to_documented_toml()`, a comment above
  each key giving what it does, its values and its default. `docs/settings.example.toml` is the
  default file; a test fails when it's out of date, and `UPDATE_EXAMPLE=1 cargo test -p
  overmark-core` rewrites it.
- **Forgiving:** a missing key takes its default. An unknown key, a number out of range (clamped
  to the nearest allowed), a bad file-name pattern or a palette of the wrong length (padded with
  or cut to the defaults) is a `Warning` worded in Fluent: the rest of the file loads. Unknown
  keys are dropped the next time the file is saved.
- **Clear when it can't be used:** invalid TOML, or a value of the wrong type (`dim = "dark"`,
  an unknown tool), is `SettingsError::Corrupt` with the file, line and column, worded for
  people (`error-settings-unreadable`). The app shows it and runs on `Settings::default()`.
- **Saving** writes a temporary file beside the target, then renames it over the old one,
  retrying for about a second while Windows refuses (an antivirus scan, say). A failed save
  leaves the old file as it was.
- **The save folder** is kept as text that may start with `{pictures}` (the default is
  `{pictures}\Overmark`); `CaptureSettings::resolve_save_folder` joins the rest to the
  Pictures folder the platform gives it.

**Capture file names** (`settings::file_name`): a pattern of text and codes in braces,
`{year} {yy} {month} {day} {hour} {minute} {second} {millisecond}`, with `{{` and `}}` for a
brace. The default, `{year}-{month}-{day} {hour}.{minute}.{second}`, gives
`2026-10-21 09.42.17.png`. The time is a `CaptureTime` the platform fills in from the local
clock, so core needs no date crate. The name is made safe for Windows (reserved characters
become `_`, trailing dots and spaces go, device names such as `CON` get a `_`, and it's cut to
200 UTF-16 units, the unit Windows counts a name's 255 in, so an emoji counts twice) and a clash
gets ` (2)`, ` (3)` and so on before the extension. `capture_path` checks the folder; the writer still creates the file with `create_new`, as another program
could take the name in between.

## Portable and installed

One exe; portable mode is switched on by a file beside it, the industry standard (ShareX,
Notepad++, Everything and KeePass do the same). `overmark_win::paths` is
Replica's `paths.rs`, copied with its tests:

- **Portable** when a file named `portable` (empty, no extension; a folder of that name doesn't
  count) or a `settings.toml` sits beside `overmark.exe`: everything lives beside the exe.
- **Installed** otherwise: `%APPDATA%\Overmark\`, made if it's missing.
- **Writable or not** is found with a probe file (created and removed), the only reliable test
  on Windows: a folder's read-only attribute doesn't stop files being created, its permissions
  do.

At start-up (`overmark::config::Startup`, run once the single-instance check has passed, so
only the resident instance writes):

- **A first run** writes the documented `settings.toml` (`Settings::save`) where it belongs.
- **A folder that can't be written to** (a read-only share, a locked stick): a tray notice
  (`error-folder-not-writable-title` with `error-portable-not-writable` or
  `error-app-folder-not-writable`), the settings read if they're there and kept in memory, no
  log file, and nothing written anywhere else.
- **A file that can't be read** (invalid TOML, a value of the wrong type): a tray notice
  (`error-settings-unreadable-title`, with the file, line and column), the defaults for this
  run, and the file **left as it is** for the user to fix: nothing saves over it this run
  (`can_save` false). Warnings (an unknown key, a value clamped) are logged; Settings will show
  them.
- **No `%APPDATA%`**: a notice (`error-app-folder-unknown`), defaults in memory.
- **About** (`Startup::about`): portable or installed (`settings-mode-portable` /
  `-installed`), the settings file, the logs folder, and whether changes can be saved.
- **The captures folder**: the setting's `{pictures}` token becomes the user's real Pictures
  folder from `SHGetKnownFolderPath(FOLDERID_Pictures)` (`win32::known_folders::pictures`;
  it may have been moved or be on OneDrive), in both modes.
- Portable writes nothing to the registry but Start with Windows' HKCU Run value, off by
  default (see The tray and Settings).

**Logs** (`overmark_win::logging`): `tracing` to `logs\overmark.log` beside the settings, and to
stderr. The file rolls at 1 MB to `overmark.1.log` and `overmark.2.log`, so at most three files
of 1 MB; a record never straddles two files, and a new run appends. Only the resident instance
logs to the file (a second launch, `--quit` and the grabber helper don't), so one process rolls
it. Panics on any thread are logged (`log_panics`, Replica's). The normal log is `info` and up,
always: the first line of each run says the version, the mode, the settings file, whether it can
save, the log's path and whether diagnostic logging is on. `advanced.diagnostic_logging` (off by
default) adds `debug` from Overmark's own crates only (`overmark`, `overmark_core`,
`overmark_win`; never winit's or another crate's), through a `Targets` filter behind a
`tracing-subscriber` reload layer, so `logging::set_diagnostic` changes it live when the tray
reads the settings again. The `debug` lines are focused detail for a report, a line per event and
none in per-frame, per-pointer or per-paint paths: each layer's monitor and where the keyboard
went on opening, on choosing a region and on closing; a hotkey press with the foreground window;
an activated ink layer and whether the toolbar is back on top of it; the toolbar taking the
keyboard back; the settings sections changed on a reload and the hotkeys let go; a capture's
delivery; and the opening timings. Windows are named by `window::Named` (the owner exe and the
class, never the title), queried only when the line is written.

**Tests:** `crates/overmark/tests/settings_places.rs`, each in temp folders of its own: the
marker (settings made beside the exe, nothing in `%APPDATA%`); a `settings.toml` beside the exe
(read, portable); installed with `%APPDATA%` pointed at a temp folder (made there, nothing
beside the exe, still installed on the next run); no `%APPDATA%`; a portable folder this user is
denied writing to (`icacls /deny`, undone after: a notice naming the folder, defaults in memory,
no logs, nothing written there or anywhere else); a corrupt file (defaults, a notice with the
line, the file unchanged); and the `{pictures}` token in both modes. `resident_check` runs the
real exe with `%APPDATA%` in a temp folder and checks its `settings.toml` and log are made
there; a portable copy (the exe and a `portable` file in a temp folder) made both beside itself.

## Captures

What a capture is: **the monitor the cursor is on**, as it is
at that moment, **with the drawing**; by default **copied and saved as a PNG, every time**.

**The keys** (`overlay::Command::for_key`, tested): **Ctrl+S** saves the screen; **Ctrl+Shift+S**
saves a region; **Ctrl+C** copies only; **Ctrl+Shift+Alt+S** saves without the drawing (not
Ctrl+Alt+S, which is AltGr+S on European keyboards). The ink layer
reads Ctrl, Shift and Alt from its own message queue's key state (`GetKeyState`), no global
input. Checks ask for the same commands with `WM_OVERMARK_COMMAND` posted to the ink layer.

**How a capture is made** (`overmark_win::save`, from `App::capture`):

1. **The clean image** of the cursor's monitor (`Overlay::capture_frame`): in Frozen the frozen
   frame the overlay keeps (taken before the dim or the ink); in Live a grab without our
   windows from the grabber (the Magnification API, in its helper; see Saving). **The
   composited overlay is never screenshotted**, so there's no dim, no toolbar and no notice in
   a capture, and the ink can't be in it twice.
2. **The drawing over it at full resolution** (`save::compose`): core's `render_with`, with the
   clean image as the background and highlighter strokes darkening (`Highlight::Darken`, what
   the live MIN_BLEND layer shows). Without the drawing, the image alone. Each monitor has its
   own drawing: the capture takes the cursor's monitor's.
3. **A region** (Ctrl+Shift+S): the composed image is taken when the key is pressed and shown
   still, darkened, in a selection window over that monitor (excluded from capture: a share
   sees the screen go on), and the user drags a rectangle (white edge, the inside bright). A
   hint at the top says how (`region-hint`: drag, Esc cancels) and the region's size in pixels
   shows by its corner while dragging (`region-size`): white text on a dark pill, laid out with
   core's own text shaping and Windows' fonts and composited into the selection's pixels
   (`overlay::Label`, tested). The fonts are loaded before the hint is made, as the tray's
   Capture a region… can be the first thing done after a start, before the layer's first turn
   loads them (tested cold, in a process of its own). A capture that can't start its selection, or can't be composed,
   says so in the notice (`toast-capture-failed` with Windows' reason, `toast-capture-no-image`);
   no failed grab is silent.
   **Esc cancels the selection first**; a second Esc closes the layer. The result is the
   composed image cropped (`save::crop`).
4. **Delivered** as the settings say (`capture.after_capture`, default copy and save; Ctrl+C
   always copies only): to the clipboard with `arboard`, and to a file in the captures folder
   (made if missing; `{pictures}` is the real Pictures folder), named by the pattern for the
   local time (`file_name::capture_path`: `2026-10-21 09.42.17.png`, then ` (2)`, ...), PNG or
   JPG at its quality (`image`), created with `create_new` so nothing is ever overwritten. A
   pattern that can't be used falls back to the default.

**The notice** (`win32::toast`): a small window of Overmark's own at the bottom right of the
capture's monitor, not a Windows toast, which needs an AppUserModelID a portable copy hasn't
got, so it works the same in both modes. It says where the capture went
(`toast-copied-and-saved`, `toast-saved`, `toast-copied`, or what failed), "Without the
drawing" when it was, and "Click to open it, right-click to open its folder"; a click opens the
file and a right-click shows it in Explorer. If either can't (the file was moved or deleted,
nothing opens PNGs), the notice says so in place of what it said, with Windows' reason in the
user's language (`toast-open-failed`, `error-open-folder`; `shell::open_or_why`), for its full
time again. Excluded from capture, never activated, gone after
5 s or a click; `capture.toast = false` turns it off. Its size comes from its text: each paragraph is
measured (`DT_CALCRECT`) with the fonts, width and wrapping it's painted with, a path breaking
inside a word, so a long path, an error's detail or a longer translation shows whole at any
scale; only text taller than the work area is cut there (tested at 100-200 %, with a
148-character OneDrive path).

**Checked** by `cargo run -p overmark --release --example capture_check [-- --clipboard]` on the
real exe, with `%APPDATA%` and the captures folder in temp folders, `--test-drawing` (two known
strokes from core's stroke calls, made without pointer input: a half-alpha blue pen and a yellow
highlighter) over a backdrop of RGB (200, 100, 50). Every file is read back. Run on 1 Nov 2026:

| | Result |
|---|---|
| The ink on screen while saving | the pen shows at (100, 50, 153) |
| Live save | 2560x1440; between the strokes (200, 100, 50) exactly: **no dim**; the pen (100, 50, 153), the backdrop and half-alpha blue **once** (twice would be about (50, 25, 203)); the highlighter darkening to (200, 100, 0) |
| Live without the drawing | the strokes' places are the backdrop |
| Live region | 640x140 as dragged (posted mouse messages to the selection window), the pen in it once |
| Esc on a selection | cancelled; the layer stays open |
| Frozen from F, and from the Frozen hotkey | as Live |
| Frozen without the drawing | the backdrop |
| The notice | shown, excluded from capture |
| The default (`--clipboard`) | copied and saved: the clipboard has the saved picture exactly; Ctrl+C copies with no file |
| File names | `2026-11-01 00.37.26.png`, `… (2).png` and so on, in the folder it made |

Unit tests cover the compose (once, darken, without), crop, encode, naming with clashes, a
folder that can't be made, and the key map. **Not tested here:** two monitors and 150 % (one
2560x1440 monitor at 100 %); the code works per monitor in physical pixels, the capture of the
cursor's monitor at its own size.

## Drawing on the layer

The app feeds core's tools (Tools, below) from the ink layers. The pieces: the layer's events
(`overmark_win::overlay`), the key map and the cross-monitor history (`overmark::input`,
platform-free and tested), and the drawing while the layer is open (`overmark::ink::Ink`).

**Input, from our own windows only** (no hooks):

- **Pointer:** `EnableMouseInPointer` at start-up, so the mouse comes as `WM_POINTER*` as pens
  and touch do: one path for all three (`win32::pointer::samples`, every coalesced sample of a
  message, oldest first). A pen gives its pressure, its tilt and its eraser end
  (`PEN_FLAG_ERASER` or `INVERTED`); a mouse draws with its left button only, its speed setting
  the width (core); touch draws as a mouse. Samples carry the hardware's time (the performance
  counter, as `clock`), or the time read when a device gives none.
- **Right-click undoes** (a Drawing setting, on by default): `win32::pointer::right_button_down`
  reads the message's button change, and only a mouse's right button going down counts, never a
  pen's barrel button (held while drawing) or touch (whose press-and-hold Windows turns into a
  right-click). The layer handles every pointer message, so Windows makes no mouse messages or
  context menu from them. The ink ignores it while anything touches, so a right-click
  mid-stroke neither undoes nor cuts the stroke, and the app ignores it while a region is chosen.
- **A contact** (touching down until it lifts) draws on the monitor where it started, whichever
  layer its later samples come to. **The pen's eraser end** erases (whole items) for that
  contact, and the tool comes back after it. **The laser** follows the pointer touching or not,
  and hides when it leaves the layer.
- **Typing:** `WM_CHAR` brings UTF-16 code units; a character beyond U+FFFF (an emoji, some
  CJK) comes as two surrogates, joined before the Text tool sees it (`overlay::utf16_char`,
  tested); a surrogate without its other half shows as U+FFFD rather than vanishing.
- **Keys** (`input::key_action`, tested): P, H, A, R, E, L, T, N, B, X, Z pick the tools; Shift
  squares rectangles, ellipses and blurs and snaps lines and arrows to 45°; 1-9 the palette;
  Ctrl+Z undoes, **Ctrl+Y and Ctrl+Shift+Z redo**; Delete clears (undoable);
  F swaps Live and Frozen; Esc closes. **While typing text** the letters and digits are text
  (`WM_CHAR`), so F and the tool keys do nothing; Enter or Esc finishes it, Shift+Enter starts
  a line, and Backspace, Delete, the arrows, Home and End edit; Ctrl's keys still work. **B on
  Live** is refused with a notice (core's `ToolError::BlurNeedsFrozen`, `tool-blur-needs-frozen`)
  and the tool stays. Ctrl+Alt is AltGr, and does nothing.
- **The width:** each tool starts at Settings' width (`drawing.widths`) times the monitor's
  scale; **hold a tool key and turn the wheel** to change it, 10 % a notch, within
  `Widths::RANGE` (`Widths::stepped`), kept for that tool until Overmark quits (not saved).
  The toolbar's width slider (in the width's strip) sets it the same way
  (`Ink::set_width`). The wheel comes as
  `WM_POINTERWHEEL` with the mouse in pointer mode, or `WM_MOUSEWHEEL`; both are handled.
- **The text font:** Windows' Segoe UI Variable, Segoe UI and Segoe UI Symbol, read from
  `%WINDIR%\Fonts` at run time (`overmark_win::fonts`, Text: the font), loaded on the turn after
  the first opening, once the layer shows, so they never slow an opening.
- **The focus:** the layer takes it when it opens (the primary monitor's ink layer), and gives
  it back on close to the window that had it (`window::foreground`, `give_focus_back`). Alt+Tab
  gives it to another app beneath the layer, which stays open; a click on the layer or the
  toolbar takes it back (see The toolbar, the window).

**One session per monitor.** Each monitor's layer has its own core `Session` (its pixels from
its own top-left); the tool, colour and widths are the same on all. Core keeps each session's
steps (100); `input::History` keeps which monitors each user action changed, in order, so Ctrl+Z
undoes the newest action wherever it was, and Delete's Clear of every monitor is one step. A
session drops steps on its own, and the history drops exactly those: past 100 steps its oldest
(`History::sync` forgets that monitor's oldest), and with disappearing ink every step whose
items have all faded, **wherever it is** (an old add and a newer erase of it can both go from the
middle). Each monitor's part of an action keeps the ids of the items it added or removed, and
`Ink` when each was drawn (only that, and only while it's drawn or something in the history or
a contact still refers to it, so a long opening with disappearing ink stays bounded). Each time
core's `prune_faded` runs on a monitor, `History::forget_faded` drops that monitor's parts whose
items have all faded (exactly when core's `render::opacity` reaches 0), on the undo and redo
sides, and no other monitor's. A frame prunes only the monitors with something fading on screen,
so before Ctrl+Z and Ctrl+Y every monitor is pruned: a faded step is gone everywhere before it
could be undone. **A step is told by what it changed**, not by the session's history
growing (at 100 steps a new one pushes the oldest out and the length stays): an action is
recorded for each monitor whose finished items (by id) changed. Each action makes at most one
step per monitor: text being typed is finished as its own step before a click, a Clear or
anything else; an eraser's drag is compared from before it touched to its lift (it removes
items as it moves), and what fades meanwhile isn't counted as the drag's. Tested at the limit
with two monitors (the newest stroke undone wherever it was; a Clear of both undone as one), and
with an old item and its newer erase fading from the middle of one monitor's history (the other
monitor's newer stroke still undone first, and redone in order), with one monitor pruned by a
frame while another holds faded steps unseen, and over 200 draw-and-fade cycles (nothing kept).
Ctrl+Z during a stroke, shape or erase drops it; during typing, drops the text.

**Rendering: dirty rectangles, once per turn.** Events only change the sessions and widen each
monitor's dirty rectangle; at the end of the loop's turn (`Ink::flush`) each monitor redraws
that rectangle alone with core's `render_region_with` into a premultiplied RGBA pixmap of the
layer's size, copies it into the layered window's BGRA surface (transparent pixels at alpha 1,
so clicks still hit the layer) and presents only it (`UpdateLayeredWindow` with a dirty
rectangle). However many samples came in a turn, there's one redraw: no added lag. The live
layer is transparent, so the highlighter is `Highlight::Tint` there (captures darken). The
caret is painted over the surface only, never into the drawing. **Blur over a frozen frame:**
the layer is transparent and a blur pixelates what's under it, so in Frozen its rectangles are
rendered over the frozen frame (with any ink above them) and copied in opaque; on screen it
matches the capture exactly, and it sits over the Frozen tint (it's a cover).

**Animation only while something moves.** A turn waits with no timeout unless something
changes by itself: a frame (16 ms) while ink fades or the laser's trail shows (core's
`animated_area`, then `prune_faded` and `laser_prune`, in that order), and with disappearing
ink, until the next stroke starts to fade. A laser dot held still needs no frames. The app
logs `animating` and `still` as it starts and stops.

**Timings**, logged when the layer closes (`timings`): each sample's age when its pixels were
shown (from the hardware's time), and each redraw's length, as median, 95th percentile and
worst.

**Checked** by `cargo run -p overmark --release --example draw_check` on the real exe, with
`%APPDATA%` and the captures folder in temp folders, over a backdrop of RGB (200, 100, 50) with a
green stripe. It draws through the ink layer's own test messages, `WM_OVERMARK_POINTER` (a
packed sample: a mouse's, or a pen's with pressure and its eraser end, `pointer::pack`) and
`WM_OVERMARK_KEY` (a key with its modifiers), with `WM_CHAR` and `WM_MOUSEWHEEL`; they take the
same path as real input from the moment it's read. Each step is judged from a saved capture.
Run on 1 Nov 2026, all ok: the pen (the mouse, in the first palette colour exactly, and a pen's
samples), highlighter, arrow, rectangle and its Shift square, ellipse, line and its Shift snap,
two lines of text, a stamp, the eraser, the pen's eraser end (the pen draws again after it), the
laser leaving nothing, key 5's colour exactly, the wheel with P held (2 px to 6 px), B refused on
Live with a notice, F typed as text, the blur pixelating the stripe in Frozen and showing on
screen as saved; Delete, Ctrl+Z and Ctrl+Y; 110 strokes, 100 undone (the first 10 left) and all
redone with Ctrl+Y and Ctrl+Shift+Z; the focus back on close; disappearing ink gone by itself,
then 0.0 ms of CPU in a second with the layer open. Timings: redraws median 0.5-0.7 ms, worst 7 ms
(judged: under a frame at the 95th percentile); samples shown a median 5-11 ms after they were
read, posted in bursts of hundreds at once, so mostly queueing behind each other (a real
pointer's come a few at a time): reported, not judged. `resident_check` and `capture_check` still pass:
opens median 37 ms Live and 70 ms Frozen; idle again 7.6 MB private after 16 opens.

**Not checked here:** **a real pen** (no pen tablet on the build PC): pressure, tilt and the
eraser end are read as the pen API documents them, and the check sends a pen's samples from
there on; the pen check is on the release checklist (docs/RELEASING.md). Nor a real
`WM_POINTERWHEEL` (Windows drops one posted from another process; `WM_MOUSEWHEEL` takes the same
path), touch, or two monitors.

## The toolbar

A small floating toolbar at the top centre of the monitor under the cursor, over the ink.
`overmark::toolbar` (what it shows and does, and its
egui frame) and `overmark_win::win32::panel` (its window).

**egui without a GPU.** An eframe window with the glow renderer, measured here: first frame
186-277 ms after it's made, 52-116 MB private memory. Made on each opening that's over the
whole 100 ms budget; kept from start-up it breaks the 30 MB idle budget (as DirectComposition's
device did: see The resident app). So the toolbar is plain egui, with the shared fonts, theme and
widgets (The windows' look), run on the app's own loop:

1. `ui::show` lays it out from the drawing's `State` (backdrop, guides, tool, colour, width,
   whether Undo and Redo can act, collapsed) and returns the `Action`s clicked. It's tested
   with egui_kittest through the accessibility tree.
2. epaint tessellates the frame, and `raster` fills its triangles on the CPU into the window's
   BGRA pixels: a pixel is in a triangle when its centre is, by the top-left rule, so two
   triangles' shared edge is drawn once; premultiplied source-over in sRGB space, as egui's own
   renderers blend; bilinear texture sampling (the font atlas), one sample for a shape's fill.
   Tested: a translucent quad blended once everywhere, scaling, clipping, texture patches.
3. The window (`Panel`): layered (per-pixel alpha), **`WDA_EXCLUDEFROMCAPTURE`**, never
   activated (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`: the keys stay with the ink layer), above the
   ink layer (made after it; brought back on top whenever an ink layer is **activated**).
   Activation puts a window at the top of the top-most windows, and the panel never activates,
   so an ink layer that takes the focus (after a region selection), or is clicked once another
   app has the foreground (Alt+Tab with the layer open), comes over it, and the toolbar's clicks
   would go to the ink. The ink layer says so (`WM_ACTIVATE` gives `OverlayEvent::Activated`;
   not while a region is chosen, when the selection covers the toolbar) and the app raises the
   panel. A press on the toolbar while another app has the keyboard gives it back to the layer
   (`Overlay::has_keyboard`, `take_keyboard`): Windows allows it, the click being this process's
   input. Its transparent pixels let the pointer through to the ink layer; its
   own pixels take it, so a click on the toolbar never draws. **Only what's solid takes it:**
   each frame gives the panel its frames' rectangles (the toolbar's, a strip's, the
   More menu's: egui's visible areas but its tooltips, each a pixel larger for its border), and
   elsewhere the panel answers `WM_NCHITTEST` with `HTTRANSPARENT`, so the pointer goes on to
   the ink layer, a window of the same thread. In use, a press just below the toolbar
   sometimes didn't draw: a tooltip is drawn there, in the toolbar's window, with its shadow,
   and by alpha alone the window took the press. Tested on the off-screen frame (a tooltip's
   pixels and a few round a frame don't take it; every solid pixel does), on a panel of its own
   over a window of the same thread (its hit test, and `WindowFromPoint` on that thread finding
   the window beneath), and by `toolbar_check`, which asks the toolbar's window its hit test
   (`panel::hit_test`) under a tooltip it shows: from another process `WindowFromPoint` doesn't
   ask a window of another thread, and takes it as opaque wherever it's drawn.
4. Frames only when due: input came, the state changed, or egui asked for one (a tooltip's
   delay); the loop waits for egui's delay and otherwise not at all: no CPU while nothing moves.

**Kept warm.** The egui context (fonts, styles, font atlas) and its textures (`toolbar::Look`)
are made at start-up and drawn once off screen, two frames at the primary monitor's scale and
the window's size, and a toolbar window is made and dropped (Windows' UI Automation, which the
screen-reader adapter starts, loads then: some 25 ms the first opening would otherwise pay), so
an opening makes the window and draws one frame (9-17 ms). egui's animations are off for it (a
button greying out is one frame, not several). The layers and
the toolbar are made, then shown together: the app waits for the compositor once
(`Overlay::shown`), so they come in the same frame, and `visible_ms` in the `opened` log is the
hotkey to both on screen. Opening also no longer redraws every ink layer in full (the drawing is
empty): that was some 20 ms.

**What it shows:** a grip; a Frozen badge (a snowflake, in the accent) when frozen, whose tooltip
says what Frozen means and its key (`mode-frozen-tooltip`; collapsed, the mode's pill has Live's
or Frozen's); the
backdrop (Live, Frozen, Whiteboard, Blackboard: its tooltip gives W, Shift+W and F) and the
guides (G, Shift+G); the tool (each with its key) and the colour (the palette's nine, 1 to 9,
each named for its colour while it's the default palette's, "Red (1)", or "Colour n" once
Settings changed it, the ninth "Custom colour"), the width, and a
**disappearing-ink toggle** (Settings' own setting, taken up as a change from Settings is,
`drawing_changed`, so the ink is drawn again: turned off mid-fade, a stroke comes back whole;
and saved as the tray saves a setting); Undo and Redo (greyed out when they can't act; Ctrl+Z,
and Ctrl+Y or Ctrl+Shift+Z), Clear (Delete); Save (Ctrl+S), Save a region (Ctrl+Shift+S), Copy
(Ctrl+C), More with Save without the drawing (Ctrl+Shift+Alt+S); Freeze or
Unfreeze (F); Collapse; Close (Esc). Collapsed, it's a pill: the grip, the mode and Expand. Each
control's tooltip gives its key; its icons are our own line icons drawn in code (`toolbar::icons`,
and `ui::icons`), so no icon set ships. Every string is Fluent (`locales/en/toolbar.ftl`), the key
names too (`key-ctrl`...).

**Each control does what its key does,** as a command of its own (`Ink::command`), not a
synthetic key: while text is typed the keys are text, so a key event would be swallowed (Delete
deleting a character, Esc only finishing the text), and a tool key would stay "held" for the
wheel. A tool, Freeze or Clear finishes the text first, as a click elsewhere does; Undo drops it,
as Ctrl+Z does; Close closes. A strip's backdrop or guides go straight to the one chosen.
Tested while typing, and a tool picked then a plain wheel turn (the width stays).

**The strips:** Backdrop, Guides, Tool, Colour and the width are
buttons showing the current choice (on, for a screen reader, while open); each opens a strip of
its choices in a second egui `Area` just below the toolbar, inside the toolbar's window: icon
toggles (`toolbar::icons`' own glyphs) for the backdrops, guides and tools, the palette's
swatches, or the width slider. One strip at a time, its group in egui's memory
(`ui::open_group`), so the app holds no state for it; it starts under its button and egui keeps
it inside the window, wrapping where it's narrow. It stays open while you draw and choose (a
press on the ink never reaches egui), and closes by its up arrow, its button, collapsing, or the
layer closing (`Toolbar::close`). Esc is the layer's. The toolbar never takes the focus, so
while the width's strip is open the app gives its slider the arrows that come to the ink layer
(`App::width_slider_event`), unless text is being typed. The strip is drawn as the toolbar is,
and only its frame takes the pointer (above), not its shadow: tested from the
off-screen render, for each strip.

**It fits its window:** the window is 1040 points wide but never wider than its monitor, and the
controls wrap onto a second row when there's less room (a 1280-pixel monitor at 150 % has 853
points), so the grip and Close are always in it. Tested at 853 points with the longest names.

**Placement:** the top centre of the monitor under the cursor, 4 points from its top; dragged by
its grip (the window follows the pointer's samples; egui can't move its own window), it's kept on
its monitor and remembered there: `[toolbar.positions]` in Settings, by the monitor's device name,
saved when it's let go. Collapsed or expanded stays while Overmark runs.

**Screen readers:** egui's AccessKit tree goes to AccessKit's own Windows adapter
(`accesskit_windows`, MIT or Apache-2.0, through `SubclassingAdapter` on the panel's window),
which answers `WM_GETOBJECT`; an action asked there (Narrator's, UI Automation's Invoke) is
posted back to the app's loop and given to egui as the click it stands for. Every control has a
name (the grip is "Move the toolbar"); the window's title is `toolbar-name`.

**A contact belongs to its pointer.** Found with the toolbar: the real mouse hovering over the
layer while another pointer touches (a pen; a check's samples) gave hover samples that ended the
stroke. A contact now keeps the pointer that started it and ignores the others until it lifts;
the checks' synthetic samples come from a pointer id of their own (`pointer::CHECK_POINTER_ID`).

**Checked** by `cargo run -p overmark --release --example toolbar_check` on the real exe, through
**UI Automation on the toolbar's own window** (`overmark_win::win32::uia`: what Narrator finds,
and its press, the Invoke pattern) and `WM_PANEL_POINTER` posted to it for the grip; nothing
global. Run on 1 Nov 2026, all ok: every control found by its name; Undo and Redo greyed out,
then enabled; Undo, Redo, Clear, Save, More › Save without the drawing, Freeze and Unfreeze, Save
a region (cancelled), Collapse and Expand, and Close each doing what its key does (judged from
saved captures and the log; Copy isn't pressed, it would take the clipboard); the toolbar is the
window under its buttons, the ink layer under its window's empty part, and on top again after a
region; **a share** (WGC, after a barrier) **sees the ink and, where the toolbar is, the screen**;
dragged 200 by 150, it opens there again and Settings have it; 0.0 ms of CPU in a second with it
open; hotkey to the layer and toolbar on screen median 43 ms, worst 77 over 8 opens.
`resident_check` (later, with Settings): Live median 46 ms, worst 57; Frozen median 85 ms,
worst 87 (of which the toolbar 11 ms); idle 10.4 MB private, 16.3 after use. The `opened` log
gives `layers_ms`, `flush_ms`, `toolbar_ms`, `ready_ms` (all made) and `visible_ms` (on screen). Pictures: `toolbar_shot` draws it through
the same rasterizer (`cargo run -p overmark --release --example toolbar_shot -- <folder>`); the
window itself is excluded from capture, so no screenshot shows it.

**Not checked here:** Narrator itself reading it (UI Automation finds every name, which is what
Narrator reads; a listen is on the release checklist in docs/RELEASING.md), keyboard focus
inside the toolbar (it never takes the focus; every control has its key), two monitors and 150 %.

## Strokes

`overmark_core::freehand` is a port of [perfect-freehand](https://github.com/steveruizok/perfect-freehand)
1.2.3 (MIT, Steve Ruiz), the algorithm behind tldraw's and Excalidraw's strokes: pressure,
thinning, smoothing, streamline, tapers and caps. It follows the TypeScript step for step,
on `kurbo`'s `Point` and `Vec2` in `f64`.

- `stroke_points(input, options)` smooths and measures the input (`getStrokePoints`),
  `stroke_outline_points(points, options)` makes the outline (`getStrokeOutlinePoints`), and
  `stroke_outline` does both (`getStroke`). The outline is a closed polygon: fill it, non-zero,
  to draw the stroke. `StrokeOptions::default()` is the library's defaults.
- **Input points** are `x`, `y` and an optional pressure (0 to 1), as the library's array form.
  With a mouse there is none, and `simulate_pressure` makes it from the speed; with a pen, pass
  the pressure and set `simulate_pressure: false`.
- **Checked against the library itself:** `freehand/fixtures.rs` holds outlines and measured
  points that perfect-freehand 1.2.3 returned for 11 inputs (lines, a wave, a spiral with
  pressure, sharp zigzags, a U-turn, noise, single points, repeats, invalid pressures) under 14
  sets of options (each option, tapers, flat caps, a finished stroke, custom easings, size 0).
  The port matches to 1e-9. `freehand/generate-fixtures.mjs` makes the file: unpack
  `npm pack perfect-freehand@1.2.3` in a folder of its own and pass it its
  `dist/cjs/index.js`. It's build-only; nothing of the library ships but the port.
- **The credit:** the library's licence is in `third_party/ported/perfect-freehand.LICENSE` and
  in `THIRD-PARTY-NOTICES.txt` (its part for ported source).

## Drawing and rendering

What's on one monitor's layer, and how it becomes pixels. All in `overmark-core`; the
overlay feeds it samples and the capture code renders captures with it.

- **The model** (`drawing`): a `Drawing` is a list of items in drawing order, each with an
  `ItemId`, a tool, a colour, a width and its `InkPoint`s: x and y in **physical pixels of that
  monitor's layer**, pressure (0 to 1, `None` for a mouse), an optional tilt (kept for later),
  and a timestamp in milliseconds on one steady clock. `begin`, `push_point`, `finish`,
  `insert`, `remove` and `clear` change it; each returns the area that changed. The other
  tools' items and their data are in Tools, below.
- **The pen** (`pen`): the outline comes from `freehand` (above). A pen's pressure sets the
  width (thinning 0.6); with a mouse there's no pressure, so the distance between samples (its
  speed) sets it (perfect-freehand's simulated pressure, thinning 0.5). Streamline 0.45 and
  smoothing 0.5. The outline polygon is smoothed with `kurbo` into quadratic curves through its
  edges' midpoints. A stroke still being drawn trails slightly behind the pointer (streamline);
  `finish` makes it end at the last point.
- **Each item's outline and path are kept** in the drawing, recomputed only when that item
  changes, so a frame only fills paths.
- **What changed:** a stroke growing at its end keeps the start of its outline polygon and its
  end (the polygon runs along the left side, round the end, back along the right). The changed
  area is the bounding box of the points in between, of the old and new outline, with two
  neighbours each side (the curves reach between neighbours) and 2 px for anti-aliasing. A test
  draws 120 samples and checks every changed outline point is inside.
- **The renderer** (`render`): `render(drawing, target, view, background)` draws everything onto
  a tiny-skia pixmap: `Background::Transparent` for the live layer, `Background::Image(screen)`
  for a capture at full resolution. `View { scale }` maps drawing pixels to target pixels.
  Anti-aliased, non-zero fill, straight-alpha colours. Pixels are premultiplied RGBA (Windows'
  layered windows want premultiplied BGRA: swap red and blue).
- **Redrawing what changed:** `dirty_rect(area, view, w, h)` turns a change's area into
  pixels, and `render_region` redraws only that rectangle, **exactly** as a full render would.
  It draws into a scratch pixmap with the target's pixel grid, reaching past the rectangle, and
  copies the rectangle in. Exactness needs one rule: where a path leaves a pixmap, tiny-skia
  clips its edges, and a clipped edge's anti-aliasing can differ a little along its length
  (a curve tiny-skia splits there moves along all of it). So paths are filled as straight edges
  no longer than 4 px (`pen::MAX_EDGE`, flattened by `kurbo` to 0.05 px), the same edges for
  every render, and the scratch reaches 4 px times the scale plus 4 past the rectangle: any
  clipping effect stays outside what's copied. Tested exactly: a 600-sample random scribble,
  each sample redrawn alone, then finished, matches a full render pixel for pixel at scales
  0.5 to 2 and widths 1 to 200. (The first version filled curves and drifted: its edge errors
  built up along a long stroke.)
- **Golden images:** `crates/overmark-core/tests/golden/` (docs/TESTING.md, Golden images).

**Speed** (`cargo bench -p overmark-core --bench redraw`; 31 Oct 2026, this PC, under load, as
other builds run on this PC all the time). A 3,000-sample pen stroke, about 6,000 px of ink
with varying pressure, drawn over five earlier strokes on a 3840×2160 layer, per sample:

| | median | p95 | max |
|---|---|---|---|
| Update the drawing (outline and changed area) | 0.28 ms | 0.55 ms | 1.22 ms |
| Redraw the changed area | 0.14 ms | 0.29 ms | 0.94 ms |
| **Both** (live layer) | **0.44 ms** | **0.71 ms** | **1.36 ms** |
| Both (capture, over a screen image) | 0.42 ms | 0.67 ms | 1.57 ms |
| A full redraw of the 4K layer, for comparison | 12.9 ms | 16.6 ms | |

So a sample fits a 144 Hz frame (6.9 ms) about 15 times over at the median and 4 times at its
slowest, while a full 4K redraw doesn't fit one: **the layer should redraw only what changed**,
and redraw in full only for an undo, a clear or a new layer. The update grows with the stroke's
length, as its whole outline is recomputed each sample; if very long strokes ever need it, the
outline's unchanged start can be kept instead.

## Session and undo

`overmark_core::session::Session` is what one opening of the layer holds: the
`Drawing`, the current tool, colour and width, the numbered-stamp counter and the undo/redo
history. The overlay's pointer input drives it; each changing method returns the area to redraw.

- **Strokes:** `stroke_begin`, `stroke_extend`, `stroke_end` (one undo step), `stroke_cancel`.
  Every tool goes through `pointer_down`, `pointer_move` and `pointer_up` (see Tools).
- **Stamps:** `place_stamp(at)` gives the next number; undoing it gives the number back. Clear
  starts the count again at 1; undoing the Clear brings it back. `add_item` is there for
  shapes and text.
- **The eraser** removes whole items: `erase_begin`, `erase_to`, `erase_end`, one undo step per
  drag (none if it touched nothing). It hit-tests the drag's segment since the last sample, so
  a quick swipe can't skip a thin stroke, against each item's filled outline with a reach of
  `ERASER_TOLERANCE` (6 px); `hit_test` is public for picking an item.
- **Clear** (Delete) and **recolour** (an item's colour) are one step each.
- **Undo and redo** (Ctrl+Z, Ctrl+Y): a new change drops the redo branch. Undo while a stroke is
  being drawn drops that stroke. `HISTORY_STEPS` = 100 (the requirement was at least 100);
  the oldest change is forgotten beyond it.
- **Memory:** the history holds changes, never copies of the drawing. Each item is kept once: in
  the drawing, or in the one history entry that can put it back (an undone add, an erase, a
  Clear). A recolour keeps two colours. A test counts it.
- **Closing the layer:** `clear_session()` drops the drawing, history and stamp count, and keeps
  the tool, colour and width for the next opening. Nothing is ever saved.

## Tools

Every tool in `overmark-core`: each item's shape (`geometry`), how it's drawn
(`render`), and the session's API for them (`session`). The overlay's pointer input calls them.

**One pointer API for every tool:** `Session::pointer_down(point, shift)`, `pointer_move`,
`pointer_up`, dispatching on `session.tool`. `shift` is the Shift key: squares and circles for
rectangles, ellipses and blur, 45° for lines and arrows (`geometry::constrain`, `snap`). Each
returns the area to redraw. Pen and highlighter make strokes; arrow, rectangle, ellipse, line and
blur make shapes from the press to the release; a stamp is placed on the press; text starts a
text item where clicked; the eraser erases; the laser moves its dot. Strokes, shapes, stamps and
text are one undo step each.

- **Highlighter:** the pen's outline at a constant width with flat ends (a chisel tip). How it
  blends is a render option, `RenderOptions.highlight`:
  - **Live, with the DirectComposition MIN_BLEND layer**: render the ink layer
    with `Highlight::Skip`, and the darken layer with `render::render_highlights` (only the
    highlighter, opaque, on transparent pixels; `render_highlights_region` for a dirty
    rectangle). The live screen is then darkened like ink on paper, text staying dark.
  - **Live without it:** `Highlight::Tint`, the colour at `TINT_ALPHA` (38%) of its alpha.
  - **Frozen mode and captures** (`Background::Image`): `Highlight::Multiply`, a true multiply,
    or `Highlight::Darken` to match MIN_BLEND exactly. `Highlight::Auto`, the default, is
    Multiply over an image and Tint on a transparent layer.
- **Arrow:** a shaft the width across and a head `ARROW_HEAD_LENGTH` (4) widths long and
  `ARROW_HEAD_WIDTH` (3.5) across, at least 10 px long, never longer than the arrow.
- **Rectangle, ellipse, line:** outlines the width thick (mitred corners; round line ends),
  stroked with `kurbo`.
- **Text:** a text item (`Item.text`: the text and its caret's byte index) with its top-left at
  the click, the width as its size, multi-line. `text_insert`, `text_newline` (Shift+Enter),
  `text_backspace`, `text_delete`, `text_left`, `text_right`, `text_home`,
  `text_end_of_line`; `text_caret()` is the caret's rectangle for the overlay to draw (and blink).
  `text_end(now)` (Enter, Esc, or a click elsewhere) finishes it as one undo step; empty text is
  dropped with no step.
- **Numbered stamps:** a disc the width across with its number (from the session's counter),
  white, or near-black on light colours such as yellow.
- **Blur:** a rectangle that pixelates what's under it in blocks the width across, aligned to
  the target's pixel grid. Every pixel of a blur depends on what's under it, so a region redraw
  that touches a blur over a frame grows to cover it whole (and any blur that one overlaps):
  `render_region` and `render_region_with` **return the rectangle they actually redrew**, and
  that's the rectangle to show. **Frozen and captures only:** `pointer_down` refuses it with
  `ToolError::BlurNeedsFrozen` unless `session.frozen` (its message is
  `tool-blur-needs-frozen`), and on a transparent layer a blur draws nothing. It never fades.
- **Laser:** `session.laser()`, kept out of the drawing and the history; the renderer draws it
  over everything when given `RenderOptions.laser` (`LaserLook`: the laser, its colour and
  Settings' trail length) and `now_ms`: a dot with a soft glow, and a trail of fading,
  shrinking dots. `laser_hide()` when it leaves the layer.
- **Disappearing ink:** `RenderOptions.disappearing_ink_ms` (Settings' delay) and `now_ms`.
  A finished item is fully visible until the delay after it was drawn (its last point's time;
  text, when finished), then fades out over `FADE_MS` (600 ms): `render::opacity`.
  **Each frame, in this order:** redraw `render::animated_area` (items fading, or faded and not
  yet removed; the laser's kept positions, expired ones included), then call
  `Session::prune_faded(now, delay)` (redrawing the area it returns) and
  `Session::laser_prune(now, trail)`. The frame that clears the last of a fade or a trail is
  then drawn before it's forgotten. **Undo stays sane:** a faded item is gone as if never
  drawn, wherever it was (in the drawing, or held by the history: erased, cleared or undone).
  Its step goes, and so does every step that would then do nothing: an erase or Clear of only
  faded items, a recolour of one, an add whose item is gone, on the undo and the redo side
  alike. Each side is judged in the order its steps would run (what would be drawn at each
  step's turn: a redo Clear with nothing drawn at its turn goes, even if a later redo brings
  something back), repeated until none is left. So Ctrl+Z always undoes the newest change still visible,
  never brings back something that faded, and never spends a press on nothing.
- **Region redraws stay exact** with every tool (tested: each tool drawn, an eraser swipe and an
  undo, all redrawn piecemeal, match a full render pixel for pixel over a frame and on a
  transparent layer; and recolouring, erasing or fading ink under a blur, and a fade and a laser
  trail animated frame by frame to nothing, the same).
- **Golden images** for every tool are in `crates/overmark-core/tests/golden/`: the highlighter
  live (tint) and over text (multiply), the shapes, two lines of text, stamps on three
  colours, blur over a test screen, the laser's trail and disappearing ink half faded.

### Text: the font

**Core takes font bytes from the platform at run time; nothing is bundled** (the licence rule,
and Fonts above). `overmark-win` reads Windows' own Segoe UI Variable (or Segoe UI), and Segoe
UI Symbol as a fallback, from `%WINDIR%\Fonts`, and calls `overmark_core::text::set_fonts` once at
start-up. Fonts that are part of Windows are never redistributed, so there's nothing to license.
`parley` lays the text out (with `default-features = false`: no system font scanning in core,
so it stays platform-free) and `swash` gives each glyph's outline, filled as ink. Until fonts
are set, text has no glyphs and a stamp shows its disc alone.

The tests use **Tuffy** (`crates/overmark-core/tests/fonts/`, from `fontdb`'s test fonts): its
authors put it in the public domain, with a grant for any use where that isn't possible
(`Tuffy-LICENSE.txt`). It's test data only and never ships.

## Backdrops and guides

What's under the ink is two layers, each cycled by a key. Core
has the models, their rendering, save composition and settings (`overmark_core::backdrop`,
`composite`, `settings`); the overlay wires the keys and windows (On the layer, below).

- **`Backdrop`**, one at a time, W forward and Shift+W back: `Live` → `Frozen` → `Whiteboard`
  → `Blackboard` (`next()`, `prev()`, wrapping). F still toggles Live and Frozen.
  `fill(&settings.backdrops)` says what it is: `Screen` (the live screen; in a save, the clean
  grab), `Frame` (the frozen frame) or `Solid(colour)`. `name()` is its Fluent name
  (`backdrop-*`) and `icon()` its icon's id. `Session::backdrop` holds the current one.
- **`Guides`**, over any backdrop, Live included, G forward and Shift+G back: `Off` → `Grid` →
  `Dots` → `Lines` (ruled), with `name()` (`guides-*`) and `icon()`.
- **Adding an entry** is one place: the enum, its `ALL`, and its arms in the same file.
- **Drawing:** `render_solid(target, colour, region)` fills a solid backdrop.
  `render_guides(target, guides, style, view, region)` draws the guides over what's there:
  lines one pixel wide (rounded up with the scale) and square dots, on whole pixels from the
  top-left, so they're crisp at 100%, 150% and any scale, and a region draws exactly what a full
  render does. `GuideStyle::new(&settings.guides, &settings.backdrops, backdrop)` gives the
  spacing and a colour that suits the backdrop (dark on the whiteboard, light on the
  blackboard, a translucent grey over the screen). **`GuideLayer`** keeps the guides as a layer
  of their own, drawn again only when the size, kind, style or scale changes, so they cost
  nothing per frame. Stack: backdrop, guides, then the highlighter and the ink. On a solid
  backdrop the ink can render over `Background::Image(backdrop + guides)`, so the highlighter
  multiplies there as on a frozen frame.
- **Saves:** `composite::compose_save(base, &SaveLayers { guides, highlight, ink })`. `base` is
  `SaveBase::Image` (the clean grab for Live, the frozen frame for Frozen) or `SaveBase::Solid`
  (the backdrop's colour, in the image's `ChannelOrder`). The guides go over the base (pass
  `None` when Settings' `guides.include_in_saves` is off), then the highlighter darkens, then
  the ink goes over.
- **Blur stays Frozen-only:** `Backdrop::allows_blur()` is true for Frozen alone, and
  `Session::pointer_down` refuses Blur on Live, the whiteboard and the blackboard
  (`ToolError::BlurNeedsFrozen`).
- **Settings:** `[backdrops]` has `whiteboard_colour` (white), `blackboard_colour` (#262A2E, a
  dark grey), and `hotkey_whiteboard` and `hotkey_blackboard` (unset; Live and Frozen keep
  `hotkey_draw` and `hotkey_draw_frozen`). `[guides]` has `spacing` (32 px, 8 to 400),
  `show_on_shares` and `include_in_saves` (both on). The overlay honours `show_on_shares` by
  excluding the guide layer's window from capture when it's off. Each key is documented in
  `docs/settings.example.toml`.
- **Golden images:** each solid backdrop, and each guide on both, at 100% and 150%, and the grid
  as a live layer.

### On the layer

- **Keys** (`input::key_action`, tested): **W** the next backdrop, **Shift+W** the one before;
  **G** the next guides, **Shift+G** the ones before; **F** from any backdrop but Frozen to
  Frozen, and from Frozen to Live. Ignored while typing text, as F is. The strokes stay through
  every change (`Ink::set_backdrop` only tells the sessions, for Blur).
- **The frozen frame is kept across the boards:** leaving Frozen for a whiteboard or blackboard
  keeps it (`Screen::frozen`), so W or F back to Frozen shows **the same moment**, with no new
  grab. Live lets it go, and Frozen from Live grabs the screen anew (the grabber, without our
  windows). A board needs no grab.
- **Windows:** a board fills the backdrop layer with its colour (`Fill::Solid`, the layer made
  if there's none, under the veil). The guides are core's `GuideLayer` pixels in a layer of their
  own between the veil and the ink, drawn again only when the kind, the style (its colour
  follows the backdrop), the size or the scale changes, and dropped when they're off. The veil
  goes under the guides, so the dim never greys them. The clean grab leaves the guides' window
  out with the rest of ours. **Its GDI fallback** (no grabber) excludes from capture, for the
  grab, every layer of ours a capture could see (the ink, the guides and the backdrop, a board
  or the frozen frame), so Frozen from a board grabs the screen under it, not the board.
  `--no-grabber` forces the fallback, for checks.
- **Saves** (`save::compose`): the base is the clean grab (Live), the frozen frame (Frozen) or a
  frame of the board's colour (`Overlay::capture_frame`); the guides go over it when
  `guides.include_in_saves` (`Overlay::save_guides`, at the monitor's scale), then the drawing,
  the highlighter darkening, as core's `composite::compose_save` orders them.
- **Blur** is Frozen's only (core refuses it elsewhere, with the `tool-blur-needs-frozen` notice).
  **What a blur pixelates is what's under it where it's shown:** the frozen frame with the
  guides, built by one function for both (`save::base`). On screen (the ink layer, which a share sees) the guides are
  in it when `guides.show_on_shares` is on; in a save, when `guides.include_in_saves` is on. With
  both on (the defaults) the screen and the save are the same pixel for pixel; each setting
  still governs its own output. The screen's base is kept (`save::Under`) until the guides or
  the frame change, and G redraws the layers so a blur follows the new guides. Over a board a
  blur shows the board, as a save does.
- **Hotkeys:** `backdrops.hotkey_whiteboard` and `hotkey_blackboard` (unset by default) register
  through `RegisterHotKey` like the draw hotkeys (`Action::DrawWhiteboard`, `DrawBlackboard`),
  with the same notice when one is taken, and open the layer straight onto that board.
- **The guides stay** from one opening to the next while Overmark runs (as the tool and colour
  do); every opening starts on its hotkey's backdrop. The tray tooltip names a board
  (`tray-tooltip-board`).
- **Checked** by `cargo run -p overmark --release --example backdrop_check` on the real exe
  (temp `%APPDATA%`, a window of a known colour behind, keys as `WM_OVERMARK_KEY` posted to the
  ink layer: a posted `WM_KEYDOWN` can't carry Shift). Run on 1 Nov 2026, all ok: every W,
  Shift+W, G, Shift+G and F step from the log; a save over each backdrop with the stroke (the
  screen, the frozen frame, white, #262A2E); the window behind changed colour on the whiteboard
  and Frozen again still showed the old frame, Live the new; no veil over a board; **a share**
  (Windows Graphics Capture, after a barrier) **sees the whiteboard and the grid on it**, and
  with `show_on_shares` off sees no grid while the guides' window is excluded and a save still
  has the grid; the board hotkey opens on the whiteboard; a blur on the frozen frame over the grid
reads the same on screen (GDI) as in the save, also after G changes the guides; and with
`--no-grabber`, F from the whiteboard freezes the screen under it (with the backdrop left in the
fallback grab, it froze the white board: the check fails). Switching took 3-15 ms between boards
  and Frozen kept, 40-48 ms to Frozen from Live (a grab); drawing the guides 8-16 ms.

## The windows' look

The toolbar and the Settings window are egui with AccessKit, **no
`default_fonts`** (their fonts are OFL and Ubuntu-licensed). The Settings window is `eframe` 0.37
with the glow renderer; the toolbar is egui on the app's own loop, drawn on the CPU (The toolbar,
below), as a GL window costs too much to open on a hotkey. Their shared part is `crates/overmark/src/ui/`, copied and adapted from Replica's (its docs/ARCHITECTURE.md,
"Shared with other apps"):

- `fonts.rs` (as it is): Windows' own fonts from `%WINDIR%\Fonts` at run time, nothing bundled:
  Segoe UI Variable, then Segoe UI, with Segoe UI Symbol for symbols; a semibold family; Cascadia
  Mono, then Consolas. A missing file is left out and logged; only no text font at all fails
  (`NoTextFont`, for the window to word from Fluent).
- `theme.rs`: light and dark palettes, type, spacing and the focus ring, with **Overmark's copper
  accent** from the icon's tokens (assets/README.md): `#9A4316` light, `#F7A66B` dark, and a warm
  tint for info bars. A test checks every text and control pair against WCAG AA in both themes.
- `widgets.rs` and `icons.rs`: the components (cards, setting rows, primary, secondary, danger and
  icon buttons, toggles, segmented controls, tabs, info bars, chips, path fields, dropdowns,
  toolbars) and line icons drawn in code. Each component has a name in the accessibility tree.
  Their own words are the `widget-*` strings in `locales/en/settings.ftl`. Replica's table,
  folder tree and time and size wording are left out.
- **Windows-only:** `ui` loads Windows' fonts, so the module, its tests, `eframe` and
  `egui_kittest` are all `cfg(windows)`; on other systems `overmark` builds without them.
- **egui's notice:** egui's crates are published without their licence file, so
  `THIRD-PARTY-NOTICES.txt` takes their copyright notice from egui's own `LICENSE-MIT`, kept in
  `third_party/crates/` (its README.md names the crates and the version it was checked against;
  the generator stops when one changes).
- **One call:** each window calls `overmark::ui::install(ctx)` once (it loads the fonts, sets both
  themes' styles, and follows Windows' light or dark mode, live).
- **Cost while no window is open:** the Settings window never runs in the tray: it's a process
  of its own (The tray and Settings, below). The toolbar's egui does: the tray calls
  `install` once at start-up and keeps the toolbar's egui warm, so an opening only makes a window
  and draws a frame (The toolbar). That's idle memory, not CPU, and no GL context: private
  memory in the tray 10.1 MB (2.4 MB before), 16.1 MB after use (7.6 MB), under the 30 MB budget.
  (Before either used it, the linker left `ui` out of the exe: 4,188,672 bytes with it, 4,194,816
  without, on 1 Nov 2026.)
- **Screenshots:** `crates/overmark/tests/ui_shots.rs` (its `Variant` and `save` are Replica's)
  renders screens with egui_kittest's wgpu renderer, in light and dark at 100% and 150%, to
  `target/ui-shots/<OVERMARK_UI_SHOTS or latest>/`:
  `cargo test -p overmark --test ui_shots -- --ignored`. It has `gallery` (the components, in
  Overmark's Settings words), Settings' four pages, and the toolbar expanded and collapsed.
  A plain test drives the gallery through the accessibility tree.

## The tray and Settings

**The tray** (`overmark-win::win32::tray`, extended with Replica's `IconFile`,
`ThemedIcon` and `taskbar_is_light`) shows Overmark's icons: idle, Live (also over the boards) and
Frozen, each in its light- or dark-taskbar variant. Windows tells the tray's hidden window when
the theme, the DPI or Explorer changes (`WM_SETTINGCHANGE`, `WM_THEMECHANGED`, `WM_DPICHANGED`,
`TaskbarCreated`) and the icon is made again at the new size. The app's side is
`crates/overmark/src/app/tray_ui.rs`:

- **Clicks:** a click does what Settings says (Live, Frozen or Settings). As a double-click opens
  Settings, a click that opens the layer waits for Windows' double-click time first (500 ms by
  default), so a double-click opens Settings and nothing else; Enter on the icon acts at once.
- **The menu** has the hotkeys beside Draw, Draw on a frozen screen and Capture a region…, and
  Start with Windows ticked from the Run value. "Check for updates…" is there only when it's on.
  "Capture a region…" freezes the screen, lets the region be chosen, and closes the layer after.
- **Balloons:** a hotkey notice opens Settings when clicked; a new release's opens its page.
- **Hotkeys:** Draw, Draw Frozen, Capture a region and Print Screen (Off, Live or Frozen) are
  registered with `RegisterHotKey`, with the whiteboard's and blackboard's; what became of each
  is published for Settings (below).

**Settings is a process of its own** (`overmark.exe --settings`, `--about` for About), started
by the tray and gone when closed, as the grabber is. egui with its GL context keeps 75 MB
private while open (measured on 1 Nov 2026, a debug build), which the tray's 2 MB idle couldn't
carry once Settings had been opened; a winit event loop can't be made twice in a process, so an
in-process window would stay for good. The two talk through named objects in the session:

- **Saving:** every change is saved to `settings.toml` at once (a slider's when it's let go),
  and the window sets `Local\Overmark.Reload`; the tray reads the file again and registers the
  hotkeys afresh if any changed, with a notice only for those. Changed drawing settings take
  effect at once, the drawing kept (`Ink::settings_changed`, `Overlay::set_dim`): a tool's new
  width replaces one set with the wheel, a new starting tool or colour is picked now and for the
  next opening, a palette colour in use follows its change, and an open layer's dim, board
  colour and guides change (`Overlay::set_look`), with the ink drawn again over them, as G
  does (a blur on the frozen frame is made over the guides: `Ink::look_changed`).
  A file name pattern being typed that can't be used yet is a draft of the window's, never
  saved, and dropped whenever the settings are replaced (Reset, a change on disk). The window writes the file as it
  is on disk for what it doesn't edit (the toolbar's places), and reads it again when it changes
  under it (the tray's Start with Windows).
- **Hotkey state:** the tray publishes each hotkey's state (registered, taken, failed, invalid,
  unset) in named shared memory, `Local\Overmark.HotkeyStatus` (`overmark_core::hotkey_status`,
  `win32::shared_text`, with a sequence number so a reader never sees half a text). Settings shows
  a hotkey that isn't working beside it.
- **Changing a hotkey:** Win, Ctrl, Alt and Shift toggles and a list of keys (Win can't be read
  from a key press inside egui, and the shell takes many Win combinations first). One another app
  has is refused, found by registering it for a moment (`win32::hotkey_probe`, never a hook),
  with the nearest free one offered (`Hotkey::alternatives`: the other usual modifiers, then the
  keys beside it; never Ctrl+Alt); another Overmark hotkey's key is refused too.
- **One window:** a second launch sets `Local\Overmark.SettingsShow` (or `SettingsAbout`) and the
  open window comes forward on that page.
- **Start with Windows** is the Run value `Overmark`, the MSI's own (`autostart.rs`, Replica's
  as it is): the only registry write in portable mode, off by default. The setting beside it
  follows the value; Reset turns it off.
- **About** embeds `THIRD-PARTY-NOTICES.txt` (shown in rows, so 200 KB costs nothing until
  scrolled) and names the C runtime's exception.
- **Tests** drive the window through its accessibility tree with a fake `Host` in temp folders
  (`settings_window/tests.rs`); `ui_shots` has each page in both themes.

**The update check** is off by default and runs only when it's on: at start and from
the menu. It's one request, `GET https://github.com/Suremise/overmark/releases/latest` with
redirects off, through WinHTTP (`win32::http`); the redirect's `Location` names the latest
release's tag, which `overmark_core::update` compares with this version. A newer one gets a
balloon that opens its page. Nothing is downloaded; with it off, nothing in Overmark touches the
network: `win32::http` is the only code that can, and only `update::check_latest` calls it,
which only runs when the setting is on.

## Recording

Recording a box on the screen to a small MP4: the engine both recording modes drive, recording
while annotating (the drawing is on the screen, so it's in the video) and screen capture (no
drawing layer). No audio. `overmark_win::record`, with its platform-free half in
`overmark_core::record`.

**The API** (`record::Recording`): `start(Start { area, settings, path, encoder }, on_end)`
returns once the first image is in, or with why it can't record; then `pause`, `resume`,
`elapsed` (the time recorded), `status` (Recording, Paused or Ended), `ended_by` (why it ended
by itself), `stop` (saves; gives the path, length, frames, bytes, box and encoder) and `discard`
(deletes the file). Dropping a recording stops and saves it. A recording that ends by itself
saves what it had and calls `on_end` (from its own thread: post a message to a window there);
`stop` then gives the file with `ended_by` set. Errors are `RecordError`, each with a message in
the user's language (`locales/en/recording.ftl`).

**The box** (`core::record::place`, tested): physical pixels on the virtual desktop. A box on
two monitors is refused, plainly, not stitched (`record-error-two-monitors`); one on no monitor
too. Otherwise it's cut to its monitor, and its width and height are rounded down to even
numbers, keeping its top-left, as H.264's 4:2:0 chroma needs. Each side needs at least
`MIN_SIDE` (34) pixels: Microsoft's software encoder refuses a side of 32 or less (measured:
48x48, 34x34 and 66x50 are taken; 32x32, 96x32 and 128x16 aren't). Its largest frame is
36,864 macroblocks (4096x2304, H.264 level 5.1); NVIDIA's takes 4096x2304 but not 7680x4320. A
size the encoder refuses is said with its reason (`record-error-encoder-refused`).

**How it records** (one thread per recording, `overmark-record`):

1. **Frames from Desktop Duplication** (`capture::dd::Duplication::next_change`), which sees
   the ink and the frozen backdrop and leaves out every window excluded from capture: the record
   controls never show, and the drawing needs nothing extra. The box is copied out of the
   monitor's frame on the GPU (`CopySubresourceRegion` into a staging texture the box's size,
   kept between frames) and read back. On a turned monitor the box is found in the unturned
   texture (`capture::texture_area`, tested) and turned back.
2. **A frame each tick.** Desktop Duplication gives a frame only when the screen changes, so the
   thread writes one every 1/15, 1/24 or 1/30 s from the latest image: the file plays at a
   constant rate, and a still screen repeats one buffer, which costs almost nothing to encode or
   store. If the PC stalls for more than a second, the gap is one long frame, not a burst.
3. **Timestamps from real time** (the performance counter): frame `n` is at `n / fps` seconds of
   recorded time (`core::record::frame_time`, exact at 15 fps too). **A pause** stops the
   recorded clock (`core::record::Clock`): the paused time is taken off every later time, so
   the video has neither a gap nor a frozen stretch. While paused the thread sleeps.
4. **The pointer drawn in** from Desktop Duplication's shape and position
   (`core::record::pointer::draw`, tested): colour shapes blended by alpha, masked-colour ones
   replacing or inverting, and monochrome ones by their AND and XOR masks, as Windows draws
   them. `RecordSettings::pointer = false` leaves it out. On the real screen the arrow lands on
   `GetCursorPos`, its tip at its top-left.
5. **BGRA to NV12 on the CPU** (`core::record::nv12`, tested): BT.709 in limited range, as both
   media types say, so players show the colours as they were (a round trip through the encoder
   and Windows' decoder is within 8 levels in the tests). Only frames that changed are
   converted.
6. **H.264 into an MP4** through Media Foundation's sink writer (`win32::mf::Mp4Writer`), to
   `<name>.mp4.part` until it's finished, then renamed (a taken name gets ` (2)`). The partial
   file is made with `create_new` before anything else (`<name>.mp4.2.part` and so on if one is
   there), so a recording only ever replaces or deletes its own, never another recording's or
   one left from a failed finish. High profile,
   no B-frames, the index (`moov`) at the end as Media Foundation writes it. (Its option to put
   the index first left stale bytes between the index and the video, which its own reader then
   refused, so it isn't used.) **The encoder** is the GPU's (NVENC, Quick Sync, AMD's) when the
   sink writer may use hardware transforms and it takes the size, else Microsoft's software
   encoder, tried in that order: on this PC a 48x48 box falls back to software and 800x600 gets
   NVENC.

**Media Foundation is loaded at run time** (`win32::mf`): Windows N and KN editions have none
until the Media Feature Pack is installed, and the `windows` crate puts each function it calls
into the exe's import table, so an exe that called `MFStartup` directly wouldn't start there at
all. The few free functions used come from `GetProcAddress` on `mfplat.dll` and
`mfreadwrite.dll` (from System32 only); everything else is COM methods. Without it, Overmark
runs, and recording says how to get it (`record-error-no-media-foundation`).

**Failing plainly:** the screen lost while recording (the secure desktop, a lock, a display mode
change, an unplugged monitor: Desktop Duplication's `DXGI_ERROR_ACCESS_LOST` and the like) ends
the recording and saves what it had (`record-error-capture-lost`); so does an encoder that fails
part way (`record-error-encoding`). A duplication that can't start says why
(`record-error-capture-unavailable`). Discarding deletes the `.part`.

**Frame rate and quality** (`RecordSettings`, defaults 15 fps and Balanced): 15, 24 or 30 fps;
Small, Balanced or Sharp is a bitrate that grows with the box's area and the rate
(`core::record::bitrate`: 0.04, 0.07 and 0.12 bits a pixel a frame, kept between 0.25 and
40 Mbit/s): the encoder's average, with a peak of twice that.

**Rate control: a capped variable bitrate** (`eAVEncCommonRateControlMode_PeakConstrainedVBR`),
not a constant one. A constant bitrate may pad (some encoders write filler to hold the rate)
and can't let a busy moment borrow from the still ones; capped VBR spends what the picture
needs, up to the peak. Measured, neither encoder here padded in CBR (the table's last column),
so the choice guards against the encoders that do. **What keeps a still screen small is the key
frames:** each is a whole picture, and a still minute is nearly all key frames. There's one every
30 s (`KEY_FRAME_SECS`): a still full screen took 8.3 MB a minute with one every 10 s, 3.0 MB at
30 s and 1.6 MB at 60 s (software encoder, the scene below). Players seek to the key frame
before and decode forward, which is quick through repeated frames. Quality-based VBR (quality
50) was about half the size again on still screens, but its scale means something different to
each encoder and doesn't follow the box's area, so it isn't used.

### Sizes

Measured on 7 Nov 2026 on one PC (an NVIDIA GeForce RTX 5070 Ti, Windows 11, one 2560x1440
monitor at 165 Hz), 15 fps, a minute each.

**A known scene** (`record/measure.rs`, made frame by frame and encoded as the engine does, with
no screen): a desktop of text that never changes, and a Task Manager window (800 x 600) whose
graph scrolls and numbers change once a second. MB a minute:

| Box | Encoder | Small | Balanced | Sharp | Balanced, still | Balanced, CBR |
|---|---|---|---|---|---|---|
| 800 x 600 (Task Manager) | software | 0.59 | 0.60 | 0.61 | 0.44 | 0.63 |
| 800 x 600 (Task Manager) | NVIDIA H.264 Encoder MFT | 0.70 | 0.70 | 0.83 | 0.61 | 0.72 |
| 1280 x 1440 (half) | software | 1.65 | 1.66 | 1.67 | 1.50 | 1.68 |
| 1280 x 1440 (half) | NVIDIA H.264 Encoder MFT | 2.04 | 2.06 | 2.36 | 1.98 | 2.14 |
| 2560 x 1440 (full) | software | 3.11 | 3.11 | 3.12 | 2.95 | 3.12 |
| 2560 x 1440 (full) | NVIDIA H.264 Encoder MFT | 3.89 | 3.98 | 4.45 | 3.90 | 3.97 |

The quality hardly changes a mostly still screen: the encoder needs far less than the average
it's allowed. It caps busy ones. With all of 1280 x 1440 scrolling every frame: Small 7.70,
Balanced 7.98, Sharp 8.05 MB a minute in software; 6.90, 7.38 and 8.11 with NVENC.

**The real screen** (`cargo run -p overmark-win --release --example record_check -- --seconds
60`: Desktop Duplication with the pointer and the hardware encoder, with whatever was on the
screen while it ran):

| Box | Recorded | MB a minute |
|---|---|---|
| 800 x 600 at (100, 100) | 60.07 s, 901 frames, NVENC | 1.76 |
| 1280 x 1440 (the left half) | 60.07 s, 901 frames, NVENC | 8.19 |
| 2560 x 1440 (the whole screen) | 60.07 s, 901 frames, NVENC | 17.44 |

The screen was in use while it ran (a terminal and a progress panel updating as builds ran), so
the half and whole screen were busy, not still: the known scene is the still case. The small box
is inside the target of 1 to 3 MB a minute for a small, mostly still box. A run with a 2 s pause
recorded 4.07 s in 6.42 s.

### Checked

- **Unit tests** in core: the box's placement and the two-monitor refusal, even sizes, frame
  times and counts at every rate, the pause offset, the bitrate mapping, NV12, the pointer
  shapes, and every error's message.
- **The engine** (`record/tests.rs`): the real thread, clock and software encoder with a test
  pattern for a screen. A recording's frame count, duration and size from the file read back
  with Media Foundation; its colours decoded back; a pause leaving no gap; discard leaving
  nothing; a lost screen ending the recording and keeping a playable file; a size the encoder
  refuses; a box on no monitor; a failed start or a discard leaving another recording's partial
  file alone; pause and resume doing nothing once a recording has ended.
- **In a browser:** a clip of the test pattern through the engine and NVENC
  (`write_a_sample_clip`, ignored: it writes to `OVERMARK_SAMPLE_CLIP`) in a `<video>` in
  headless Edge (no window), seeked to 1.5 s and drawn to a canvas: duration 3.067 s, 640x360,
  and the pattern's colour (200, 100, 50) exactly.
- **The real screen** (`record_check`): each box records and reads back with Media
  Foundation's own reader and decoder (what Windows' players use), the pause is left out, and
  the pointer lands where `GetCursorPos` says. It makes no window and sends no input.
- **Not checked here:** two monitors (one 2560x1440 at 100 % here; the box's placement is
  tested with two), a turned monitor, a lock or display change while recording (the path is
  tested with a source that fails), Windows' Media Player app itself (it plays through Media
  Foundation, whose reader and decoder are checked), Windows 10 and an N edition.
  The engine uses nothing that Windows 10 2004 lacks.
