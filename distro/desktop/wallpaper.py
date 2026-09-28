#!/usr/bin/env python3
"""Generate the SlateOS wallpaper: a dark slate gradient with two soft glows.

Pure Python (no PIL) so it runs anywhere; dithered to avoid banding.
Usage: wallpaper.py [out.png] [width height]
"""
import math
import random
import struct
import sys
import zlib


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "wallpaper.png"
    w = int(sys.argv[2]) if len(sys.argv) > 3 else 2560
    h = int(sys.argv[3]) if len(sys.argv) > 3 else 1600
    rnd = random.Random(7)
    top = (23, 26, 33)  # #171a21
    bottom = (12, 13, 18)  # #0c0d12
    glows = (  # (cx, cy, radius, colour, strength), all relative to the frame
        (0.68, 0.22, 0.75, (96, 118, 196), 0.34),
        (0.12, 0.95, 0.60, (150, 96, 140), 0.16),
    )
    rows = []
    for y in range(h):
        fy = y / (h - 1)
        row = bytearray()
        for x in range(w):
            fx = x / (w - 1)
            t = 0.55 * fy + 0.45 * fx
            r = top[0] + (bottom[0] - top[0]) * t
            g = top[1] + (bottom[1] - top[1]) * t
            b = top[2] + (bottom[2] - top[2]) * t
            for cx, cy, rad, col, k in glows:
                d = math.hypot((fx - cx) * (w / h), fy - cy) / rad
                if d < 1.0:
                    a = k * (1 - d) ** 2
                    r += (col[0] - r) * a
                    g += (col[1] - g) * a
                    b += (col[2] - b) * a
            n = rnd.random() - 0.5
            row += bytes((clamp(r + n), clamp(g + n), clamp(b + n)))
        rows.append(b"\x00" + bytes(row))
    raw = b"".join(rows)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(out, "wb") as f:
        f.write(png)


def clamp(v):
    return max(0, min(255, int(round(v))))


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


if __name__ == "__main__":
    main()
