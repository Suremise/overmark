# Overmark's icons

Original art for Overmark, under Overmark's MIT licence. No third-party icon sets, fonts or
images are used in any of the icons.

**Every icon file here is generated** by `export.py`. To change an icon, edit the
shapes in `export.py` and run it again; don't edit the SVG, ICO or PNG files by hand.

```
python assets/export.py
```

It needs Python 3.10 or later and [Pillow](https://python-pillow.org/) (`pip install pillow`;
made with Pillow 12.3.0). Both are build-only tools and don't ship in Overmark. With the same
Pillow version, it writes the same bytes every time. The script is Replica's
`assets/export.py`, copied and adapted (the shapes, SVG writer, renderer and contrast check are
Replica's; the drawings, palette and preview are Overmark's).

The preview sheet can show other apps' icons beside Overmark's, for comparison only:

```
python assets/export.py --family "Dialect=<path>/dialect/branding/out/icon.ico" --family "Replica=<path>/replica/assets/icon/replica.ico" --family "Legible=<path>/legible/assets/icon/legible.ico"
```

Their icons appear only in `preview.png`, which is for review and doesn't ship.

## The demo clip

`overmark-demo.gif`, the README's clip of Live and Frozen, is rendered by
`crates/overmark/examples/demo_clip.rs`, under Overmark's MIT licence: the desktop (its
wallpaper, windows and their content) is drawn in code and invented, and the toolbar, strokes,
veil and notice are Overmark's own code drawing them. No screen recording, photo or third-party
image is in it. [docs/RELEASING.md](../docs/RELEASING.md) says how to regenerate it.

## The files

| File | What |
|---|---|
| `icon/overmark.svg` | The app icon's master, used for 40 px and up |
| `icon/overmark-small.svg` | The small variant, used at 16 to 32 px: a flat disc, the nib and its stroke bolder |
| `icon/overmark.ico` | The exe's, the installer's and the window's icon: 16, 20, 24, 32, 40, 48, 64 and 256 px |
| `icon/overmark-64.png` … `overmark-1024.png` | 64, 128, 256, 512 and 1024 px, for the README, the docs and surmise.it/tools |
| `tray/<state>-on-light.svg`, `.ico` | The tray icon for a **light** taskbar: 16, 20, 24 and 32 px |
| `tray/<state>-on-dark.svg`, `.ico` | The tray icon for a **dark** taskbar (the same drawing: see below) |
| `preview.png` | Every icon at its real sizes on light and dark, for review |

The tray states: `idle`, the nib as an outline; `live` (the layer open, Live), the
nib filled with an amber dot; `frozen` (the layer open, Frozen), the nib filled with a
frost-blue pause badge.

## The icon

**A pen nib marking a short stroke.** You mark over whatever is on screen: the nib has just
drawn a short amber curve, and the stroke is the hero. No scissors, no selection rectangle,
nothing like the Snipping Tool or another Microsoft icon.

It's built the way the family's icons are (Hive, Dialect, Replica, Legible), so they sit
together on surmise.it/tools: one bold circle with a vertical gradient, a soft gloss ring, a
dark face, and the glyph in the brand's light tones. The shapes are on Replica's 16-unit grid
(the circle 7.375, the gloss ring 6.6875, the face 5.625 with a 0.5 ring).

- **The large icon** (40 px and up): the amber stroke curving under the nib, and the nib in a
  pale copper with its slit and breather hole cut in the face colour.
- **The small icon** (16 to 32 px), as Dialect's `icon-small.svg` is: no gradient, gloss or
  face. A flat copper disc with the nib and its stroke in the deep face colour, bolder.
- **The tray icon**, as Dialect's `tray.svg` is: the small icon filling its square with heavier
  lines, and a badge at the bottom right in a cut-out ring, so it stands apart from the disc.
  The disc reads on either taskbar, so `on-light` and `on-dark` are drawn the same; both names
  stay for the tray code, which picks one by the taskbar's theme.
- Each pixel size is drawn on its own, supersampled and then reduced, rather than shrunk from
  the biggest image.

**Contrast** (WCAG 2, printed by `export.py`):

| Pair | Ratio |
|---|---|
| The small icon's and tray's disc on a light taskbar (`#F3F3F3`) | 4.35:1 |
| The disc on a dark taskbar (`#202020`) | 3.38:1 |
| The glyph (`--brand-face-deep`) on the disc | 3.99:1 |
| The large icon's stroke (`--brand-amber`) on the face | 9.41:1 |
| The large icon's nib (`--brand-copper-200`) on the face | 12.92:1 |

## Palette

The icon's colours, fixed and the same in both themes. **Amber to copper**, leaning copper so it
stays clear of Hive's honey amber.

| Token | Hex | Used for |
|---|---|---|
| `--brand-copper-200` | `#FFDCC2` | The large icon's nib |
| `--brand-copper-300` | `#F7A66B` | Gradient top |
| `--brand-copper-400` | `#E57B43` | Gradient middle |
| `--brand-copper-500` | `#B9541F` | The small icon's and the tray's flat disc |
| `--brand-copper-600` | `#9A4316` | Gradient bottom |
| `--brand-amber` | `#FFB43D` | The stroke the nib marks |
| `--brand-face` | `#2B1A12` | The face, top |
| `--brand-face-deep` | `#160C07` | The face, bottom, and its ring; the small icon's glyph |
| `--tray-live` | `#FFC233` | The tray's Live dot |
| `--tray-frost` | `#8FD3F7` | The tray's Frozen badge |
