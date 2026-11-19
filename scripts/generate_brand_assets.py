#!/usr/bin/env python3
"""Generate every Auralis brand asset from the geometry in src/brand.rs.

Outputs (all committed; rerunning on the same inputs reproduces them byte for byte):
  assets/auralis-mark.svg, assets/auralis-logo.svg (light bg), assets/auralis-logo-dark.svg
  assets/auralis.ico                                 Windows app icon, 16-256 px
  assets/macos/AppIcon.iconset/icon_*.png            macOS app icon sources for iconutil
  assets/macos/dmg-background.png (+ @2x)            DMG Finder window background
  assets/installer/wizard-image*.bmp                 Inno Setup modern wizard art
  assets/installer/wizard-small*.bmp

Requirements: Python 3.10+, Pillow, numpy. The DMG hint is set in Segoe UI, so
regenerating needs C:\\Windows\\Fonts\\segoeui.ttf (or --font PATH). The Auralis wordmark
itself is drawn from strokes defined below, not from a font. See docs/BRAND.md.

This is the only asset generator; it supersedes the earlier icon-only scripts/generate_icon.py.

Usage:  python scripts/generate_brand_assets.py [--preview DIR] [--font PATH]
"""

from __future__ import annotations

import argparse
import io
import math
import re
import struct
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
BRAND_RS = ROOT / "src" / "brand.rs"

# Neutral graphite palette (see docs/BRAND.md). Only the mark color is ever bright.
FIELD = (0x11, 0x11, 0x12)  # installer and DMG fields


# --------------------------------------------------------------------------- geometry


def _floats(text: str) -> list[float]:
    return [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", text)]


def load_brand() -> dict:
    src = BRAND_RS.read_text(encoding="utf-8")

    def const(name: str) -> str:
        match = re.search(rf"pub const {name}: [^=]+=\s*(.+?);", src, re.S)
        if not match:
            raise SystemExit(f"{name} not found in {BRAND_RS}")
        return match.group(1)

    def points(name: str) -> list[tuple[float, float]]:
        values = _floats(const(name))
        return list(zip(values[0::2], values[1::2]))

    def color(name: str) -> tuple[int, int, int]:
        return tuple(int(v, 16) for v in re.findall(r"0x([0-9a-fA-F]{2})", const(name)))

    return {
        "grid": _floats(const("GRID"))[0],
        "stroke": _floats(const("STROKE_WIDTH"))[0],
        "a": points("A_STROKE"),
        "wave": points("WAVE_STROKE"),
        "inset": _floats(const("TILE_INSET"))[0],
        "radius": _floats(const("TILE_RADIUS"))[0],
        "tile": color("TILE_COLOR"),
        "ink": color("MARK_COLOR"),
        "min_stroke_px": _floats(const("MIN_STROKE_PX"))[0],
    }


B = load_brand()
TILE = B["tile"]
INK = B["ink"]


def arc(cx, cy, rx, ry, t0, t1, steps=None):
    """Points on an ellipse, angles in degrees, counterclockwise as seen on screen."""
    if steps is None:
        steps = max(8, int(abs(t1 - t0) / 6))
    out = []
    for i in range(steps + 1):
        t = math.radians(t0 + (t1 - t0) * i / steps)
        out.append((cx + rx * math.cos(t), cy - ry * math.sin(t)))
    return out


def wordmark_glyphs():
    """Monoline "Auralis" in the mark's stroke, baseline y=0, cap height 32, x-height 22.

    Returns (polylines, dots, advance_width). Coordinates are in the same units as the
    64-unit mark grid so the stroke weight matches the mark at equal scale.
    """
    x_height, cap = 22.0, 32.0
    gap = 9.0  # centerline-to-centerline space between neighbouring glyphs
    lines: list[list[tuple[float, float]]] = []
    dots: list[tuple[float, float]] = []
    x = 0.0

    # A: same leg slope as the mark (dx/dy = 1/2), plain crossbar.
    a_width = cap  # legs fall 1 unit across for every 2 units down
    lines.append([(x, 0.0), (x + a_width / 2, -cap), (x + a_width, 0.0)])
    cross_y = -cap * 0.34
    inset = -cross_y / 2.0
    lines.append([(x + inset, cross_y), (x + a_width - inset, cross_y)])
    x += a_width + gap

    # u
    w = 18.0
    lines.append([(x, -x_height), (x, -w / 2)] + arc(x + w / 2, -w / 2, w / 2, w / 2, 180, 360)[1:])
    lines.append([(x + w, -x_height), (x + w, 0.0)])
    x += w + gap

    # r
    lines.append([(x, -x_height), (x, 0.0)])
    lines.append(arc(x + 11.0, -11.0, 11.0, 11.0, 180, 90))
    x += 11.0 + gap - 1.0

    # a (single story)
    r = x_height / 2
    lines.append(arc(x + r, -r, r, r, 0, 360, steps=72))
    lines.append([(x + 2 * r, -x_height), (x + 2 * r, 0.0)])
    x += 2 * r + gap

    # l
    lines.append([(x, -cap), (x, 0.0)])
    x += gap

    # i
    lines.append([(x, -x_height), (x, 0.0)])
    dots.append((x, -cap + 2.0))
    x += gap

    # s: two stacked ellipses joined at the spine, point-symmetric.
    sw, ry = 15.0, x_height / 4
    cx = x + sw / 2
    upper = arc(cx, -3 * ry, sw / 2, ry, 30, 270)
    lower = arc(cx, -ry, sw / 2, ry, 90, -150)
    lines.append(upper + lower[1:])
    x += sw

    return lines, dots, x


# --------------------------------------------------------------------------- raster


class Canvas:
    """Float RGB canvas with straight alpha, composited with analytic coverage."""

    def __init__(self, w, h, color=None):
        self.w, self.h = w, h
        self.rgb = np.zeros((h, w, 3), np.float64)
        self.alpha = np.zeros((h, w), np.float64)
        if color is not None:
            self.rgb[:] = color
            self.alpha[:] = 1.0
        ys, xs = np.mgrid[0:h, 0:w]
        self.px = xs + 0.5
        self.py = ys + 0.5

    def paint(self, coverage, color):
        a = np.clip(coverage, 0.0, 1.0)
        out_a = a + self.alpha * (1 - a)
        safe = np.where(out_a > 0, out_a, 1)
        for c in range(3):
            self.rgb[..., c] = (
                color[c] * a + self.rgb[..., c] * self.alpha * (1 - a)
            ) / safe
        self.alpha = out_a

    def image(self, mode="RGBA"):
        rgb = np.clip(np.round(self.rgb), 0, 255).astype(np.uint8)
        a = np.clip(np.round(self.alpha * 255), 0, 255).astype(np.uint8)
        img = Image.fromarray(np.dstack([rgb, a]), "RGBA")
        return img if mode == "RGBA" else img.convert(mode)


def polyline_distance(px, py, polylines):
    d = np.full(px.shape, np.inf)
    for line in polylines:
        for (sx, sy), (ex, ey) in zip(line, line[1:] or line):
            dx, dy = ex - sx, ey - sy
            ll = dx * dx + dy * dy
            if ll <= 1e-12:
                t = 0.0
            else:
                t = np.clip(((px - sx) * dx + (py - sy) * dy) / ll, 0, 1)
            d = np.minimum(d, np.hypot(px - (sx + t * dx), py - (sy + t * dy)))
    return d


def stroke_coverage(canvas, polylines, scale, ox, oy, stroke_px):
    """Coverage of round-capped strokes; geometry units map to px via scale and offset."""
    gx = (canvas.px - ox) / scale
    gy = (canvas.py - oy) / scale
    d = polyline_distance(gx, gy, polylines) * scale
    return np.clip(0.5 - (d - stroke_px / 2), 0, 1)


def rounded_rect_coverage(canvas, left, top, size, radius, squircle=False):
    cx = left + size / 2
    cy = top + size / 2
    qx = np.abs(canvas.px - cx)
    qy = np.abs(canvas.py - cy)
    if squircle:
        # Superellipse (n = 5) approximating Apple's continuous-corner icon shape.
        n = 5.0
        h = size / 2
        r = (np.abs(qx / h) ** n + np.abs(qy / h) ** n) ** (1 / n)
        # Scale the normalized radius error back to an approximate pixel distance.
        sd = (r - 1) * h
        return np.clip(0.5 - sd, 0, 1)
    half = size / 2 - radius
    ax = qx - half
    ay = qy - half
    outside = np.hypot(np.maximum(ax, 0), np.maximum(ay, 0))
    sd = outside + np.minimum(np.maximum(ax, ay), 0) - radius
    return np.clip(0.5 - sd, 0, 1)


def mark_lines():
    return [B["a"], B["wave"]]


def mark_inked_width():
    return (B["a"][2][0] - B["a"][0][0]) + B["stroke"]


def draw_mark(canvas, cx, cy, glyph_width, color=INK, stroke_px=None):
    """Draw the bare mark so its inked width is glyph_width px, centered on (cx, cy)."""
    grid = B["grid"]
    scale = glyph_width / mark_inked_width()
    if stroke_px is None:
        stroke_px = B["stroke"] * scale
    canvas.paint(
        stroke_coverage(canvas, mark_lines(), scale, cx - grid / 2 * scale, cy - grid / 2 * scale, stroke_px),
        color,
    )


def draw_wordmark(canvas, left, baseline, cap_px, color=INK, stroke_px=None):
    lines, dots, width = wordmark_glyphs()
    scale = cap_px / 32.0
    sp = stroke_px if stroke_px is not None else B["stroke"] * scale
    cov = stroke_coverage(canvas, lines, scale, left, baseline, sp)
    dot = stroke_coverage(canvas, [[d, d] for d in dots], scale, left, baseline, sp * 1.15)
    canvas.paint(np.maximum(cov, dot), color)
    return width * scale


def wordmark_width(cap_px):
    return wordmark_glyphs()[2] * cap_px / 32.0


# --------------------------------------------------------------------------- icons


def app_tile(size):
    """The Windows/app tile at an exact pixel size, mirroring brand::render_mark(Tile).

    Sizes up to 24 px are hand-tuned: the tile goes full-bleed and the crossbar and stroke
    snap to the pixel grid so the A and its wave stay distinct instead of blurring.
    """
    c = Canvas(size, size)
    grid = B["grid"]
    scale = size / grid
    inset, radius = B["inset"], B["radius"]
    stroke = max(B["stroke"] * scale, B["min_stroke_px"])
    a, wave = B["a"], B["wave"]
    if size <= 24:
        inset = 0.0
        radius = B["radius"] * grid / (grid - 2 * B["inset"]) * 0.9
        stroke = {16: 1.5, 20: 1.75, 24: 2.0}.get(size, stroke)
        # Snap the crossbar baseline to a pixel row center and deepen the wave so its
        # crest and trough still separate from the bar at this size.
        base = wave[0][1]
        bar = (round(base * scale - 0.5) + 0.5) / scale
        depth = {16: 1.25, 20: 1.4, 24: 1.4}[size]
        wave = [(x, bar + (y - base) * depth) for (x, y) in wave]
        wave = [(x, y) for (x, y) in wave]
    c.paint(rounded_rect_coverage(c, inset * scale, inset * scale, (grid - 2 * inset) * scale, radius * scale), TILE)
    ink = stroke_coverage(c, [a, wave], scale, 0, 0, stroke)
    c.paint(np.minimum(ink, c.alpha), INK)
    return c.image()


def ico_bytes(images):
    """Assemble an ICO: 32-bit BMP entries below 256 px, PNG for 256 px."""
    entries, payloads = [], []
    for img in images:
        w, h = img.size
        if w >= 256:
            buf = io.BytesIO()
            img.save(buf, "PNG", optimize=False)
            data = buf.getvalue()
        else:
            bgra = np.array(img)[::-1, :, [2, 1, 0, 3]].tobytes()
            mask_stride = ((w + 31) // 32) * 4
            mask = bytes(mask_stride * h)  # alpha carries transparency; AND mask stays clear
            header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(bgra) + len(mask), 0, 0, 0, 0)
            data = header + bgra + mask
        entries.append((w, h, data))
    out = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    for w, h, data in entries:
        out += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for _, _, data in entries:
        out += data
    return out


def macos_icon(px):
    """macOS app icon on Apple's 1024 grid: 824 pt squircle at 100 pt margins."""
    c = Canvas(px, px)
    u = px / 1024.0
    left, size = 100 * u, 824 * u
    # Apple's template shadow: soft, directly below the tile.
    if px >= 64:
        shadow = rounded_rect_coverage(c, left, left + 10 * u, size, 0, squircle=True)
        img = Image.fromarray(np.uint8(shadow * 255), "L")
        from PIL import ImageFilter

        img = img.filter(ImageFilter.GaussianBlur(14 * u))
        c.paint(np.asarray(img, np.float64) / 255 * 0.28, (0, 0, 0))
    tile = rounded_rect_coverage(c, left, left, size, 0, squircle=True)
    c.paint(tile, TILE)
    # A hairline keeps the dark tile's edge visible on a dark Dock.
    if px >= 64:
        edge = np.clip(tile - rounded_rect_coverage(c, left + max(u * 3, 1), left + max(u * 3, 1), size - 2 * max(u * 3, 1), 0, squircle=True), 0, 1)
        c.paint(edge * 0.10, (255, 255, 255))
    # The inked mark spans 60% of the tile width, a little under the Windows tile's 70%,
    # matching the optical weight of Apple's glyph grid.
    glyph = size * 0.60
    stroke = max(B["stroke"] * glyph / mark_inked_width(), 1.25)
    draw_mark(c, px / 2, px / 2, glyph, stroke_px=stroke)
    return c.image()


# --------------------------------------------------------------------------- installer


def wizard_image(scale):
    """Welcome-page panel: graphite field, the mark, the wordmark beneath it. Nothing else."""
    w, h = round(164 * scale), round(314 * scale)
    c = Canvas(w, h, FIELD)
    s = scale
    glyph = 48 * s
    mark_cy = 136 * s
    draw_mark(c, w / 2, mark_cy, glyph)
    cap = 13 * s
    baseline = mark_cy + glyph / 2 + 24 * s + cap
    draw_wordmark(c, (w - wordmark_width(cap)) / 2, baseline, cap, stroke_px=1.75 * s)
    return c.image("RGB")


def wizard_small(px):
    c = Canvas(px, px, (255, 255, 255))
    tile = app_tile(px)
    base = c.image("RGBA")
    base.alpha_composite(tile)
    return base.convert("RGB")


# --------------------------------------------------------------------------- DMG


DMG_FIELD = (0xf4, 0xf4, 0xf5)
DMG_ARROW = (0xb4, 0xb4, 0xb8)
DMG_HINT = (0x6e, 0x6e, 0x73)
# Finder icon centers in window points; package.yml passes the same values to create-dmg.
DMG_APP_X, DMG_LINK_X, DMG_ICON_Y = 160, 440, 168


def dmg_background(scale, font_path):
    """Light field so Finder's default (dark) icon labels stay readable; quiet arrow + hint."""
    w, h = 600 * scale, 400 * scale
    c = Canvas(w, h, DMG_FIELD)
    y = DMG_ICON_Y
    left, right = DMG_APP_X + 96, DMG_LINK_X - 96
    line = [[(left, y), (right, y)], [(right - 10, y - 8), (right, y), (right - 10, y + 8)]]
    c.paint(stroke_coverage(c, line, scale, 0, 0, 2 * scale), DMG_ARROW)
    img = c.image("RGB")
    draw = ImageDraw.Draw(img)
    font = ImageFont.truetype(str(font_path), 13 * scale)
    text = "Drag Auralis to Applications"
    tw = draw.textlength(text, font=font)
    draw.text(((w - tw) / 2, 288 * scale), text, font=font, fill=DMG_HINT)
    return img


# --------------------------------------------------------------------------- SVG


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def path_d(lines, ox=0.0, oy=0.0, scale=1.0):
    parts = []
    for line in lines:
        pts = [(ox + x * scale, oy + y * scale) for x, y in line]
        parts.append("M" + " ".join(f"{fmt(x)} {fmt(y)}" for x, y in pts))
    return "".join(parts)


def mark_svg():
    g, i, r = B["grid"], B["inset"], B["radius"]
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(g)} {fmt(g)}" role="img" aria-labelledby="title description">
  <title id="title">Auralis</title>
  <desc id="description">The Auralis mark: an A whose crossbar is a soundwave, on a graphite tile.</desc>
  <rect x="{fmt(i)}" y="{fmt(i)}" width="{fmt(g - 2 * i)}" height="{fmt(g - 2 * i)}" rx="{fmt(r)}" fill="{hexcolor(TILE)}"/>
  <path d="{path_d(mark_lines())}" fill="none" stroke="{hexcolor(INK)}" stroke-linecap="round" stroke-linejoin="round" stroke-width="{fmt(B['stroke'])}"/>
</svg>
"""


def hexcolor(rgb):
    return "#" + "".join(f"{v:02x}" for v in rgb)


def logo_svg(word_color, description):
    """Horizontal lockup: tile, 24-unit gap, wordmark with cap height 32 (half the tile)."""
    g = B["grid"]
    lines, dots, width = wordmark_glyphs()
    word_left = g + 24.0
    baseline = g / 2 + 16.0
    total = word_left + width + B["stroke"] / 2 + 2.0
    dot_r = B["stroke"] * 1.15 / 2
    circles = "".join(
        f'\n    <circle cx="{fmt(word_left + x)}" cy="{fmt(baseline + y)}" r="{fmt(dot_r)}"/>' for x, y in dots
    )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(total)} {fmt(g)}" role="img" aria-labelledby="title description">
  <title id="title">Auralis</title>
  <desc id="description">{description}</desc>
  <rect x="{fmt(B['inset'])}" y="{fmt(B['inset'])}" width="{fmt(g - 2 * B['inset'])}" height="{fmt(g - 2 * B['inset'])}" rx="{fmt(B['radius'])}" fill="{hexcolor(TILE)}"/>
  <g fill="none" stroke-linecap="round" stroke-linejoin="round" stroke-width="{fmt(B['stroke'])}">
    <path d="{path_d(mark_lines())}" stroke="{hexcolor(INK)}"/>
    <path d="{path_d(lines, word_left, baseline)}" stroke="{word_color}"/>
  </g>
  <g fill="{word_color}">{circles}
  </g>
</svg>
"""


# --------------------------------------------------------------------------- main


ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
ICONSET = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]


def save_png(img, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, "PNG", optimize=False)


def save_bmp(img, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.convert("RGB").save(path, "BMP")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--preview", type=Path, help="also write enlarged preview PNGs here")
    parser.add_argument("--font", type=Path, default=Path(r"C:\Windows\Fonts\segoeui.ttf"))
    args = parser.parse_args()
    if not args.font.exists():
        raise SystemExit(f"font not found: {args.font} (pass --font)")

    assets = ROOT / "assets"
    (assets / "auralis-mark.svg").write_text(mark_svg(), encoding="utf-8", newline="\n")
    for name, color, where in (
        ("auralis-logo.svg", TILE, "light"),
        ("auralis-logo-dark.svg", INK, "dark"),
    ):
        svg = logo_svg(hexcolor(color), f"The Auralis mark beside the Auralis wordmark, for {where} backgrounds.")
        (assets / name).write_text(svg, encoding="utf-8", newline="\n")

    icons = {s: app_tile(s) for s in ICO_SIZES}
    (assets / "auralis.ico").write_bytes(ico_bytes([icons[s] for s in ICO_SIZES]))

    mac = {}
    for name, px in ICONSET:
        mac.setdefault(px, macos_icon(px))
        save_png(mac[px], assets / "macos" / "AppIcon.iconset" / name)

    for scale in (1, 2):
        suffix = "" if scale == 1 else "@2x"
        save_png(dmg_background(scale, args.font), assets / "macos" / f"dmg-background{suffix}.png")

    for suffix, scale in (("", 1.0), ("-125", 1.25), ("-150", 1.5), ("-200", 2.0)):
        save_bmp(wizard_image(scale), assets / "installer" / f"wizard-image{suffix}.bmp")
    for suffix, px in (("", 55), ("-125", 69), ("-150", 83), ("-200", 110)):
        save_bmp(wizard_small(px), assets / "installer" / f"wizard-small{suffix}.bmp")

    if args.preview:
        p = args.preview
        p.mkdir(parents=True, exist_ok=True)
        # Icon strip at 1x on light and dark, plus an 8x nearest-neighbour zoom of small sizes.
        for name, bg in (("icons-dark.png", (32, 32, 34)), ("icons-light.png", (243, 243, 243))):
            width = sum(ICO_SIZES[:-1]) + 16 * len(ICO_SIZES)
            strip = Image.new("RGBA", (width, 160), bg + (255,))
            x = 8
            for s in ICO_SIZES[:-1]:
                strip.alpha_composite(icons[s], (x, (160 - s) // 2))
                x += s + 16
            strip.convert("RGB").save(p / name)
        zoom = Image.new("RGB", ((16 + 20 + 24 + 32) * 8 + 5 * 16, 32 * 8 + 32), (243, 243, 243))
        x = 16
        for s in (16, 20, 24, 32):
            tile = Image.new("RGBA", (s, s), (243, 243, 243, 255))
            tile.alpha_composite(icons[s])
            zoom.paste(tile.convert("RGB").resize((s * 8, s * 8), Image.NEAREST), (x, 16))
            x += s * 8 + 16
        zoom.save(p / "icons-zoom.png")
        mac[1024].resize((512, 512), Image.LANCZOS).save(p / "macos-icon-512.png")
        dock = Image.new("RGBA", (128 * 3 + 64, 192), (30, 30, 32, 255))
        for i, s in enumerate((128, 64, 32)):
            dock.alpha_composite(mac[s * 1 if s != 64 else 64], (16 + i * 144, (192 - s) // 2))
        dock.convert("RGB").save(p / "macos-icon-dock.png")
        print(f"previews in {p}")
    print("brand assets written")


if __name__ == "__main__":
    main()
