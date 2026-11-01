"""Draws Overmark's icons: the SVG masters, the .ico and .png files, and the preview sheet.

    python assets/export.py [--family NAME=PATH.ico ...]

Build-only (Python 3.10+ and Pillow; see assets/README.md). Every icon is described once, below,
as a list of shapes on a 16-unit grid; the same list is written as SVG and drawn as pixels. Each
pixel size is drawn on its own (supersampled, then reduced), not shrunk from a big image, and
32 px and below use the small drawing, flat and bolder, so they read on a taskbar.

The icons are in the family style of Hive, Dialect, Replica and Legible: one bold circle with a
vertical gradient, a soft gloss ring, a dark face and a simple glyph in the brand's light tones.
Overmark's glyph is its own: a pen nib marking a short amber stroke. The machinery (shapes, SVG,
pixels, contrast) is Replica's assets/export.py, copied and adapted.

`--family` adds other apps' icons to the preview sheet for comparison (their .ico files); the
sheet is for review only, and nothing of theirs is written anywhere else.
"""

from __future__ import annotations

import argparse
import math
import sys
from dataclasses import dataclass, field
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ASSETS = Path(__file__).resolve().parent
GRID = 16.0

APP_ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
APP_PNG_SIZES = [64, 128, 256, 512, 1024]
TRAY_ICO_SIZES = [16, 20, 24, 32]
# Up to this size, the small drawing (flat, bolder).
SMALL_UP_TO = 32

# The palette (assets/README.md lists it as tokens). Fixed: the same in both themes. Amber to
# copper, leaning copper so it stays clear of Hive's honey amber.
COPPER_200 = "#FFDCC2"  # The nib on the large icon.
COPPER_300 = "#F7A66B"  # Gradient top.
COPPER_400 = "#E57B43"  # Gradient middle.
COPPER_500 = "#B9541F"  # The small icon's and the tray's flat disc.
COPPER_600 = "#9A4316"  # Gradient bottom.
AMBER = "#FFB43D"  # The stroke the nib marks: the hero.
FACE = "#2B1A12"  # The face, top.
FACE_DEEP = "#160C07"  # The face, bottom, and its ring; the small icon's glyph.
WHITE = "#FFFFFF"
# The tray's state badges.
LIVE_DOT = "#FFC233"
FROST = "#8FD3F7"


# --- Shapes ------------------------------------------------------------------------------------


@dataclass
class Gradient:
    """A vertical gradient over the shape's box: (offset 0 to 1, colour, opacity) stops."""

    stops: list[tuple[float, str, float]]


Paint = str | Gradient | None


@dataclass
class RoundRect:
    x0: float
    y0: float
    x1: float
    y1: float
    radius: float
    fill: Paint = None
    stroke: Paint = None
    width: float = 0.0  # A stroke lies inside the rectangle.


@dataclass
class Circle:
    cx: float
    cy: float
    r: float
    fill: Paint = None
    stroke: Paint = None
    width: float = 0.0  # A stroke is centred on the circle, as in SVG.
    opacity: float = 1.0


@dataclass
class Arc:
    """A stroke along a circle, from `start` to `end` degrees, clockwise on screen (0 = 3 o'clock)."""

    cx: float
    cy: float
    r: float
    start: float
    end: float
    width: float
    stroke: Paint = None


@dataclass
class Polygon:
    points: list[tuple[float, float]]
    fill: Paint = None


Shape = RoundRect | Circle | Arc | Polygon


@dataclass
class Erase:
    """Cuts these shapes out of everything drawn before it (their colours are ignored)."""

    shapes: list[Shape]


@dataclass
class Group:
    """Steps drawn on a layer of their own, so an [`Erase`] in it cuts only what's in it."""

    steps: list["Step"]


Step = Shape | Erase | Group


@dataclass
class Icon:
    steps: list[Step] = field(default_factory=list)

    def add(self, *steps: Step) -> "Icon":
        self.steps.extend(steps)
        return self


# --- The drawings ------------------------------------------------------------------------------


def rotate(points: list[tuple[float, float]], degrees: float, at: tuple[float, float]) -> list[tuple[float, float]]:
    """`points` turned by `degrees` (clockwise on screen) round the origin, then moved to `at`."""
    a = math.radians(degrees)
    c, s = math.cos(a), math.sin(a)
    return [(at[0] + x * c - y * s, at[1] + x * s + y * c) for x, y in points]


def nib(tip: tuple[float, float], length: float, half_width: float, ink: str, slit: str | None = None) -> list[Step]:
    """A pen nib pointing down and to the left, its tip at `tip`: a pointed shoulder and a
    straight barrel, with a slit and a breather hole cut in `slit`'s colour (or none)."""
    w, l = half_width, length
    # Pointing straight down in its own coordinates: the tip at (0, 0), the barrel above it.
    body = [(0.0, 0.0), (w, -l * 0.42), (w, -l), (-w, -l), (-w, -l * 0.42)]
    turn = 45.0  # down-left
    steps: list[Step] = [Polygon(rotate(body, turn, tip), fill=ink)]
    if slit:
        hole = rotate([(0.0, -l * 0.46)], turn, tip)[0]
        slit_line = [(-0.09 * w * 2, -l * 0.08), (0.09 * w * 2, -l * 0.08), (0.09 * w * 2, -l * 0.46), (-0.09 * w * 2, -l * 0.46)]
        steps += [Polygon(rotate(slit_line, turn, tip), fill=slit), Circle(hole[0], hole[1], w * 0.32, fill=slit)]
    return steps


def stroke_mark(width: float, ink: str, start: float = 112.0, end: float = 68.0) -> list[Shape]:
    """The short stroke the nib has just made: a gentle curve under the nib's tip, from its tip
    away to the right, with round ends."""
    cx, cy, r = 8.6, 1.4, 9.4
    # Clockwise on screen from `end` to `start` runs along the bottom, right to left.
    ends = []
    for deg in (start, end):
        a = math.radians(deg)
        ends.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return [Arc(cx, cy, r, end, start, width, stroke=ink)] + [Circle(x, y, width / 2, fill=ink) for x, y in ends]


def tip_of_stroke(start: float = 112.0) -> tuple[float, float]:
    a = math.radians(start)
    return 8.6 + 9.4 * math.cos(a), 1.4 + 9.4 * math.sin(a)


def app_icon_large() -> Icon:
    tip = tip_of_stroke()
    return Icon().add(
        Circle(8, 8, 7.375, fill=Gradient([(0, COPPER_300, 1), (0.55, COPPER_400, 1), (1, COPPER_600, 1)])),
        Circle(8, 8, 6.6875, stroke=Gradient([(0, WHITE, 0.35), (1, WHITE, 0)]), width=0.1875, opacity=0.6),
        Circle(8, 8, 5.625, fill=Gradient([(0, FACE, 1), (1, FACE_DEEP, 1)]), stroke=FACE_DEEP, width=0.5),
        *stroke_mark(1.0, AMBER),
        *nib((tip[0] + 0.25, tip[1] - 0.25), 7.0, 1.25, COPPER_200, slit=FACE_DEEP),
    )


def small_glyph(heavy: bool = False, outline: bool = False) -> list[Step]:
    """The small glyph: the nib and its stroke in the deep face colour, bolder. `outline` draws
    the nib as an outline (the tray's idle state)."""
    extra = 0.2 if heavy else 0.0
    tip = tip_of_stroke(118.0)
    steps: list[Step] = [*stroke_mark(1.5 + extra, FACE_DEEP, start=118.0, end=66.0)]
    steps += nib((tip[0] + 0.35, tip[1] - 0.35), 8.6, 1.95 + extra / 2, FACE_DEEP)
    if outline:
        # The inside of the nib in the disc's colour, leaving an outline.
        inner_tip = (tip[0] + 0.35 + 1.25, tip[1] - 0.35 - 1.25)
        steps += nib(inner_tip, 8.6 - 3.3, 1.95 + extra / 2 - 1.05, COPPER_500)
    return steps


def app_icon_small(size: int) -> Icon:
    return Icon().add(Circle(8, 8, 7.5, fill=COPPER_500), *small_glyph())


def app_icon(size: int) -> Icon:
    return app_icon_small(size) if size <= SMALL_UP_TO else app_icon_large()


TRAY_STATES = ["idle", "live", "frozen"]
# The tray icon is a copper disc that reads on either taskbar, so both of a state's files are
# drawn the same; the names stay for the code that picks one by the taskbar's theme.
TASKBARS = ["on-light", "on-dark"]


def tray_icon(state: str, size: int) -> Icon:
    """The small icon filling its square, with heavier strokes. Idle: the nib as an outline.
    Live: the nib filled, with an amber dot. Frozen: the nib filled, with a frost-blue pause
    badge. A badge sits at the bottom right in a cut-out ring, so it stands apart from the disc
    on either taskbar."""
    icon = Icon().add(Circle(8, 8, 7.75, fill=COPPER_500), *small_glyph(heavy=True, outline=state == "idle"))
    cx, cy, r = 12.2, 12.2, 3.6
    if state == "live":
        icon.add(Erase([Circle(cx, cy, r + 1.0)]), Circle(cx, cy, r, fill=LIVE_DOT, stroke=FACE_DEEP, width=0.9))
    elif state == "frozen":
        icon.add(
            Erase([Circle(cx, cy, r + 1.0)]),
            Circle(cx, cy, r, fill=FROST, stroke=FACE_DEEP, width=0.9),
            RoundRect(cx - 1.55, cy - 1.7, cx - 0.45, cy + 1.7, 0.2, fill=FACE_DEEP),
            RoundRect(cx + 0.45, cy - 1.7, cx + 1.55, cy + 1.7, 0.2, fill=FACE_DEEP),
        )
    return icon


# --- SVG ---------------------------------------------------------------------------------------


def fmt(v: float) -> str:
    return f"{v:.3f}".rstrip("0").rstrip(".")


class Svg:
    def __init__(self) -> None:
        self.defs: list[str] = []

    def paint(self, p: Paint) -> str:
        if p is None:
            return "none"
        if isinstance(p, str):
            return p
        n = len(self.defs) + 1
        stops = "".join(
            f'<stop offset="{fmt(o)}" stop-color="{c}"' + (f' stop-opacity="{fmt(a)}"' if a != 1 else "") + "/>"
            for o, c, a in p.stops
        )
        self.defs.append(f'<linearGradient id="g{n}" x1="0" y1="0" x2="0" y2="1">{stops}</linearGradient>')
        return f"url(#g{n})"

    def attrs(self, fill: Paint, stroke: Paint = None, width: float = 0.0, erase: bool = False) -> str:
        if erase:
            fill = "#000" if fill or not stroke else None
            stroke = "#000" if stroke else None
        out = f'fill="{self.paint(fill)}"'
        if stroke:
            out += f' stroke="{self.paint(stroke)}" stroke-width="{fmt(width)}"'
        return out

    def shape(self, shape: Shape, erase: bool = False) -> str:
        if isinstance(shape, RoundRect):
            x0, y0, x1, y1, r = shape.x0, shape.y0, shape.x1, shape.y1, shape.radius
            parts = []
            if shape.fill or erase and not shape.stroke:
                parts.append(
                    f'<rect x="{fmt(x0)}" y="{fmt(y0)}" width="{fmt(x1 - x0)}" height="{fmt(y1 - y0)}"'
                    f' rx="{fmt(r)}" {self.attrs(shape.fill, erase=erase)}/>'
                )
            if shape.stroke:
                h = shape.width / 2
                parts.append(
                    f'<rect x="{fmt(x0 + h)}" y="{fmt(y0 + h)}" width="{fmt(x1 - x0 - shape.width)}"'
                    f' height="{fmt(y1 - y0 - shape.width)}" rx="{fmt(max(r - h, 0))}"'
                    f" {self.attrs(None, shape.stroke, shape.width, erase)}/>"
                )
            return "".join(parts)
        if isinstance(shape, Circle):
            opacity = f' opacity="{fmt(shape.opacity)}"' if shape.opacity != 1 else ""
            return (
                f'<circle cx="{fmt(shape.cx)}" cy="{fmt(shape.cy)}" r="{fmt(shape.r)}"'
                f" {self.attrs(shape.fill, shape.stroke, shape.width, erase)}{opacity}/>"
            )
        if isinstance(shape, Arc):
            a0, a1 = math.radians(shape.start), math.radians(shape.end)
            x0, y0 = shape.cx + shape.r * math.cos(a0), shape.cy + shape.r * math.sin(a0)
            x1, y1 = shape.cx + shape.r * math.cos(a1), shape.cy + shape.r * math.sin(a1)
            large = 1 if (shape.end - shape.start) % 360 > 180 else 0
            d = f"M{fmt(x0)} {fmt(y0)}A{fmt(shape.r)} {fmt(shape.r)} 0 {large} 1 {fmt(x1)} {fmt(y1)}"
            return f'<path d="{d}" {self.attrs(None, shape.stroke, shape.width, erase)}/>'
        if isinstance(shape, Polygon):
            points = " ".join(f"{fmt(x)},{fmt(y)}" for x, y in shape.points)
            return f'<polygon points="{points}" {self.attrs(shape.fill, erase=erase)}/>'
        raise TypeError(shape)


def svg_body(svg: Svg, steps: list[Step], masks: list[str]) -> str:
    body = ""
    for step in steps:
        if isinstance(step, Group):
            body += f"<g>{svg_body(svg, step.steps, masks)}</g>"
        elif isinstance(step, Erase):
            n = len(masks) + 1
            cut = "".join(svg.shape(s, erase=True) for s in step.shapes)
            masks.append(
                f'<mask id="cut{n}" maskUnits="userSpaceOnUse" x="0" y="0" width="16" height="16">'
                f'<rect width="16" height="16" fill="#fff"/>{cut}</mask>'
            )
            body = f'<g mask="url(#cut{n})">{body}</g>'
        else:
            body += svg.shape(step)
    return body


def to_svg(icon: Icon) -> str:
    svg = Svg()
    masks: list[str] = []
    body = svg_body(svg, icon.steps, masks)
    defs = svg.defs + masks
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 16 16">\n'
        "<!-- Overmark icon (MIT). Generated by assets/export.py: edit that, not this. -->\n"
        f"{'<defs>' + ''.join(defs) + '</defs>' if defs else ''}{body}\n</svg>\n"
    )


# --- Pixels ------------------------------------------------------------------------------------


def rgb(colour: str) -> tuple[int, int, int]:
    c = colour.lstrip("#")
    return int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)


def supersample(size: int) -> int:
    """How many times bigger a size is drawn before it's reduced: less for the big ones, which
    have pixels enough."""
    return 8 if size <= 64 else 4 if size <= 256 else 2


def fill_image(paint: str | Gradient, top: float, bottom: float, big: int) -> Image.Image:
    """`paint` over the whole canvas; a gradient runs from `top` to `bottom` (pixels)."""
    if isinstance(paint, str):
        return Image.new("RGBA", (big, big), (*rgb(paint), 255))
    column = Image.new("RGBA", (1, big))
    stops = paint.stops
    for y in range(big):
        t = min(max((y + 0.5 - top) / max(bottom - top, 1e-6), 0.0), 1.0)
        for (o0, c0, a0), (o1, c1, a1) in zip(stops, stops[1:]):
            if t <= o1 or (o1, c1, a1) == stops[-1]:
                f = 0.0 if o1 == o0 else min(max((t - o0) / (o1 - o0), 0.0), 1.0)
                r0, r1 = rgb(c0), rgb(c1)
                colour = tuple(round(r0[i] + (r1[i] - r0[i]) * f) for i in range(3))
                column.putpixel((0, y), (*colour, round(255 * (a0 + (a1 - a0) * f))))
                break
    return column.resize((big, big), Image.Resampling.NEAREST)


def parts(shape: Shape, k: float, big: int) -> list[tuple[Image.Image, Paint, float, float]]:
    """Each painted part of `shape` as (its coverage mask, its paint, and the top and bottom of
    the box a gradient spans), scaled by `k` pixels per grid unit."""

    def mask() -> tuple[Image.Image, ImageDraw.ImageDraw]:
        m = Image.new("L", (big, big), 0)
        return m, ImageDraw.Draw(m)

    out = []
    if isinstance(shape, RoundRect):
        box = [shape.x0 * k, shape.y0 * k, shape.x1 * k - 1, shape.y1 * k - 1]
        if shape.fill:
            m, d = mask()
            d.rounded_rectangle(box, radius=shape.radius * k, fill=255)
            out.append((m, shape.fill, box[1], box[3]))
        if shape.stroke:
            m, d = mask()
            d.rounded_rectangle(box, radius=shape.radius * k, outline=255, width=round(shape.width * k))
            out.append((m, shape.stroke, box[1], box[3]))
    elif isinstance(shape, Circle):
        box = [(shape.cx - shape.r) * k, (shape.cy - shape.r) * k, (shape.cx + shape.r) * k, (shape.cy + shape.r) * k]
        if shape.fill:
            m, d = mask()
            d.ellipse([box[0], box[1], box[2] - 1, box[3] - 1], fill=255)
            out.append((m, shape.fill, box[1], box[3]))
        if shape.stroke:
            m, d = mask()
            h = shape.width * k / 2
            d.ellipse([box[0] - h, box[1] - h, box[2] + h - 1, box[3] + h - 1], fill=255)
            d.ellipse([box[0] + h, box[1] + h, box[2] - h - 1, box[3] - h - 1], fill=0)
            out.append((m, shape.stroke, box[1], box[3]))
    elif isinstance(shape, Arc):
        outer = shape.r + shape.width / 2
        box = [(shape.cx - outer) * k, (shape.cy - outer) * k, (shape.cx + outer) * k - 1, (shape.cy + outer) * k - 1]
        m, d = mask()
        d.arc(box, shape.start, shape.end, fill=255, width=round(shape.width * k))
        out.append((m, shape.stroke, box[1], box[3]))
    elif isinstance(shape, Polygon):
        m, d = mask()
        d.polygon([(x * k, y * k) for x, y in shape.points], fill=255)
        ys = [y * k for _, y in shape.points]
        out.append((m, shape.fill, min(ys), max(ys)))
    else:
        raise TypeError(shape)
    return out


def draw_steps(steps: list[Step], big: int, k: float) -> Image.Image:
    canvas = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    for step in steps:
        if isinstance(step, Group):
            canvas = Image.alpha_composite(canvas, draw_steps(step.steps, big, k))
            continue
        if isinstance(step, Erase):
            cut = Image.new("L", (big, big), 0)
            for shape in step.shapes:
                for m, *_ in parts(shape, k, big):
                    cut = Image.composite(Image.new("L", (big, big), 255), cut, m)
            alpha = Image.composite(Image.new("L", (big, big), 0), canvas.getchannel("A"), cut)
            canvas.putalpha(alpha)
            continue
        opacity = step.opacity if isinstance(step, Circle) else 1.0
        for m, paint, top, bottom in parts(step, k, big):
            layer = fill_image(paint, top, bottom, big)
            a = Image.composite(layer.getchannel("A"), Image.new("L", (big, big), 0), m)
            if opacity != 1.0:
                a = a.point(lambda v: round(v * opacity))
            layer.putalpha(a)
            canvas = Image.alpha_composite(canvas, layer)
    return canvas


def render(icon: Icon, size: int) -> Image.Image:
    big = size * supersample(size)
    return draw_steps(icon.steps, big, big / GRID).resize((size, size), Image.Resampling.LANCZOS)


def save_ico(path: Path, draw, sizes: list[int]) -> dict[int, Image.Image]:
    images = {s: draw(s) for s in sizes}
    largest = images[sizes[-1]]
    largest.save(path, format="ICO", sizes=[(s, s) for s in sizes], append_images=[images[s] for s in sizes[:-1]])
    return images


# --- Contrast ----------------------------------------------------------------------------------


def contrast(a: str, b: str) -> float:
    """WCAG 2's contrast ratio between two opaque colours."""

    def lum(c: str) -> float:
        v = [x / 255 for x in rgb(c)]
        v = [x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in v]
        return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]

    la, lb = lum(a), lum(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


LIGHT_TASKBAR = "#F3F3F3"
DARK_TASKBAR = "#202020"


# --- The preview sheet -------------------------------------------------------------------------


def font(size: int) -> ImageFont.ImageFont:
    for name in ["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"]:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


PREVIEW_SIZES = [16, 24, 32, 48, 256]


def family_icon(path: Path, size: int) -> Image.Image:
    """Another app's icon at `size`, from its .ico (the nearest size it has, scaled)."""
    with Image.open(path) as ico:
        sizes = sorted(ico.info.get("sizes", {(size, size)}))
        best = min(sizes, key=lambda s: (abs(s[0] - size), -s[0]))
        ico.size = best
        image = ico.convert("RGBA")
    return image if image.size == (size, size) else image.resize((size, size), Image.Resampling.LANCZOS)


def preview(app: dict[int, Image.Image], tray: dict[tuple[str, str], dict[int, Image.Image]], family: list[tuple[str, Path]]) -> Image.Image:
    rows: list[tuple[str, dict[int, Image.Image]]] = [("Overmark", app)]
    rows += [(name, {s: family_icon(path, s) for s in PREVIEW_SIZES}) for name, path in family]
    row_h = 300
    tray_y = 100 + len(rows) * 2 * row_h + 40
    sheet = Image.new("RGB", (1200, tray_y + 3 * 150), "#FAFAFA")
    d = ImageDraw.Draw(sheet)
    title, label, small = font(26), font(16), font(13)

    d.text((30, 20), "Overmark's icons", fill="#111", font=title)
    note = "Actual pixels at 16, 24, 32, 48 and 256 px"
    if family:
        note += ", beside the family's (" + ", ".join(n for n, _ in family) + ")"
    d.text((30, 58), note, fill="#444", font=label)

    y = 100
    for name, images in rows:
        for bg, fg in [(LIGHT_TASKBAR, "#333"), (DARK_TASKBAR, "#DDD")]:
            d.rectangle([30, y, 1170, y + row_h - 10], fill=bg)
            d.text((40, y + 8), f"{name}, on {'light' if bg == LIGHT_TASKBAR else 'dark'}", fill=fg, font=small)
            x = 50
            for s in PREVIEW_SIZES:
                shown = images[s]
                sheet.paste(shown, (x, y + 30 + (256 - s) // 2 if s < 256 else y + 30), shown)
                d.text((x, y + row_h - 34), f"{s} px", fill=fg, font=small)
                x += max(s, 40) + 40
            y += row_h

    d.text(
        (30, tray_y - 30),
        "Tray: 16, 24 and 32 px (100, 150 and 200% scaling), then 16 px at 400%",
        fill="#444",
        font=label,
    )
    for row, taskbar in enumerate(TASKBARS):
        bg, fg = (LIGHT_TASKBAR, "#333") if taskbar == "on-light" else (DARK_TASKBAR, "#DDD")
        y = tray_y + row * 140
        d.rectangle([30, y, 1170, y + 128], fill=bg)
        d.text((40, y + 6), f"{taskbar.replace('on-', '')} taskbar", fill=fg, font=small)
        x = 40
        for state in TRAY_STATES:
            images = tray[(state, taskbar)]
            cx = x
            for s in [16, 24, 32]:
                sheet.paste(images[s], (cx, y + 44 + (32 - s) // 2), images[s])
                cx += s + 10
            zoomed = images[16].resize((64, 64), Image.Resampling.NEAREST)
            sheet.paste(zoomed, (cx + 4, y + 28), zoomed)
            d.text((x, y + 104), {"idle": "idle", "live": "Live open", "frozen": "Frozen open"}[state], fill=fg, font=small)
            x += 360
    return sheet


# --- Main --------------------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--family", action="append", default=[], metavar="NAME=PATH.ico", help="another app's icon, for the preview sheet only")
    args = parser.parse_args()
    family = []
    for item in args.family:
        name, _, path = item.partition("=")
        family.append((name, Path(path)))

    icon_dir = ASSETS / "icon"
    tray_dir = ASSETS / "tray"
    icon_dir.mkdir(exist_ok=True)
    tray_dir.mkdir(exist_ok=True)

    (icon_dir / "overmark.svg").write_text(to_svg(app_icon_large()), encoding="utf-8", newline="\n")
    (icon_dir / "overmark-small.svg").write_text(to_svg(app_icon_small(32)), encoding="utf-8", newline="\n")
    app = save_ico(icon_dir / "overmark.ico", lambda s: render(app_icon(s), s), APP_ICO_SIZES)
    for s in APP_PNG_SIZES:
        image = app.get(s) or render(app_icon(s), s)
        app[s] = image
        image.save(icon_dir / f"overmark-{s}.png", optimize=True)
    for s in PREVIEW_SIZES:
        app.setdefault(s, render(app_icon(s), s))

    tray = {}
    for state in TRAY_STATES:
        for taskbar in TASKBARS:
            stem = f"{state}-{taskbar}"
            (tray_dir / f"{stem}.svg").write_text(to_svg(tray_icon(state, 32)), encoding="utf-8", newline="\n")
            tray[(state, taskbar)] = save_ico(tray_dir / f"{stem}.ico", lambda s: render(tray_icon(state, s), s), TRAY_ICO_SIZES)

    preview(app, tray, family).save(ASSETS / "preview.png", optimize=True)

    print(f"Wrote the icons to {icon_dir}, {tray_dir} and {ASSETS / 'preview.png'}")
    print("Contrast (WCAG 2):")
    print(f"  the small icon's and tray's disc {COPPER_500} on a light taskbar: {contrast(COPPER_500, LIGHT_TASKBAR):.2f}:1")
    print(f"  on a dark taskbar: {contrast(COPPER_500, DARK_TASKBAR):.2f}:1")
    print(f"  the glyph {FACE_DEEP} on the disc: {contrast(FACE_DEEP, COPPER_500):.2f}:1")
    print(f"  the large icon's stroke {AMBER} on the face {FACE}: {contrast(AMBER, FACE):.2f}:1")
    print(f"  the large icon's nib {COPPER_200} on the face {FACE}: {contrast(COPPER_200, FACE):.2f}:1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
