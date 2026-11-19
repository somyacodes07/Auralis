#!/usr/bin/env python3
"""Regenerate assets/auralis.ico from the geometry in assets/auralis-mark.svg.

Standard library only. Usage (from the repository root):

    python scripts/generate_icon.py

Writes a multi-size icon (16, 24, 32, 48, 64, 128, 256 px) with PNG-compressed 32-bit
RGBA entries, which Windows Vista and later read natively. The mark is rasterized with
4x4 supersampling: a light A/wave stroke on a near-black rounded square, transparent
outside the square, matching the SVG mark.
"""

import math
import struct
import sys
import zlib
from pathlib import Path

SIZES = (16, 24, 32, 48, 64, 128, 256)
SAMPLES = 4

# Geometry in the SVG's 64x64 viewBox.
TILE = (2.0, 2.0, 62.0, 62.0)
TILE_RADIUS = 16.0
STROKE_RADIUS = 3.5 / 2.0
POLYLINES = (
    ((18, 48), (32, 15), (46, 48)),
    ((24, 35), (30, 35), (33, 28), (38, 43), (42, 35), (46, 35)),
)
BACKGROUND = (0x18, 0x18, 0x19)
FOREGROUND = (0xF2, 0xF2, 0xF3)


def segment_distance(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    length_squared = dx * dx + dy * dy
    t = 0.0 if length_squared == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length_squared))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def inside_tile(x, y):
    left, top, right, bottom = TILE
    cx = min(max(x, left + TILE_RADIUS), right - TILE_RADIUS)
    cy = min(max(y, top + TILE_RADIUS), bottom - TILE_RADIUS)
    return (x - cx) ** 2 + (y - cy) ** 2 <= TILE_RADIUS**2


def on_stroke(x, y):
    for line in POLYLINES:
        for (ax, ay), (bx, by) in zip(line, line[1:]):
            if segment_distance(x, y, ax, ay, bx, by) <= STROKE_RADIUS:
                return True
    return False


def render(size):
    scale = 64.0 / size
    rows = []
    for py in range(size):
        row = bytearray([0])  # PNG filter type: none
        for px in range(size):
            coverage = stroke = 0
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    x = (px + (sx + 0.5) / SAMPLES) * scale
                    y = (py + (sy + 0.5) / SAMPLES) * scale
                    if inside_tile(x, y):
                        coverage += 1
                        stroke += on_stroke(x, y)
            total = SAMPLES * SAMPLES
            if coverage == 0:
                row += bytes(4)
                continue
            mix = stroke / coverage
            color = (round(b + (f - b) * mix) for b, f in zip(BACKGROUND, FOREGROUND))
            row += bytes(color) + bytes([round(255 * coverage / total)])
        rows.append(bytes(row))
    return png(size, b"".join(rows))


def png(size, raw):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(images):
    directory = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, payload = b"", b""
    for size, data in images:
        dimension = 0 if size >= 256 else size  # 0 means 256 in ICONDIRENTRY
        entries += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        payload += data
    return directory + entries + payload


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "assets" / "auralis.ico"
    out.write_bytes(ico([(size, render(size)) for size in SIZES]))
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
