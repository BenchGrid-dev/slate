#!/usr/bin/env python3
"""Generate the SlateOS wallpapers: soft ribbons of light over a gradient.

    wallpaper.py dark|light OUT.jpg [WIDTH HEIGHT]
    wallpaper.py dark splash.png 1024 768      the boot menu background

Needs numpy and Pillow (a development tool; the images are committed). Grain is
added before quantising so the gradients do not band.
"""
import sys

import numpy as np
from PIL import Image

THEMES = {
    "dark": {
        "base": ((10, 12, 24), (22, 28, 58), (12, 12, 22)),   # bottom-left, top-right, bottom-right
        "ribbons": [
            # (colour, centre y, amplitude, frequency, phase, width, strength)
            ((58, 74, 178), 0.56, 0.15, 0.50, 0.10, 0.13, 0.55),
            ((112, 82, 186), 0.70, 0.11, 0.70, 1.70, 0.07, 0.42),
            ((48, 124, 210), 0.43, 0.09, 0.90, 3.10, 0.045, 0.32),
        ],
        "highlight": (200, 212, 255),
        "vignette": 0.50,
    },
    "light": {
        "base": ((226, 233, 248), (246, 244, 250), (250, 240, 236)),
        "ribbons": [
            ((118, 146, 238), 0.56, 0.15, 0.50, 0.10, 0.13, 0.62),
            ((232, 150, 192), 0.70, 0.11, 0.70, 1.70, 0.07, 0.50),
            ((110, 188, 232), 0.43, 0.09, 0.90, 3.10, 0.045, 0.45),
        ],
        "highlight": (255, 255, 255),
        "vignette": 0.12,
    },
}


def srgb_to_linear(c):
    c = np.asarray(c, dtype=np.float64) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(c, 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def render(theme, w, h, seed=3):
    t = THEMES[theme]
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w].astype(np.float64)
    fx, fy = x / (w - 1), y / (h - 1)
    aspect = w / h

    # Base: a three-corner gradient in linear light.
    bl, tr, br = (srgb_to_linear(c) for c in t["base"])
    diag = np.clip((fx + (1 - fy)) / 2, 0, 1)[..., None]
    img = bl * (1 - diag) + tr * diag
    img = img * (1 - (fx * fy)[..., None] * 0.6) + br * (fx * fy)[..., None] * 0.6

    # Ribbons: soft bands that drift across the frame, each with a brighter edge.
    for colour, cy, amp, freq, phase, width, strength in t["ribbons"]:
        col = srgb_to_linear(colour)
        u = fx * aspect
        centre = (1 - cy) + amp * np.sin(2 * np.pi * freq * u / aspect + phase) \
            + amp * 0.35 * np.sin(2 * np.pi * freq * 2.3 * u / aspect + phase * 1.7)
        thick = width * (0.75 + 0.5 * (0.5 + 0.5 * np.sin(2 * np.pi * 0.4 * fx + phase)))
        d = (fy - centre) / thick
        body = np.exp(-d ** 2) * strength
        edge = np.exp(-((d + 0.6) / 0.38) ** 2) * strength * 0.35
        a = body[..., None]
        img = img * (1 - a * 0.55) + col * a  # glow over the base
        img = img + srgb_to_linear(t["highlight"]) * edge[..., None] * 0.06

    # Vignette towards the corners.
    r = np.sqrt(((fx - 0.5) * aspect) ** 2 + (fy - 0.45) ** 2) / np.sqrt((0.5 * aspect) ** 2 + 0.55 ** 2)
    img = img * (1 - t["vignette"] * r[..., None] ** 2.2)

    out = linear_to_srgb(img) * 255
    out += rng.normal(0, 1.1, out.shape)  # grain against banding
    return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "RGB")


def main():
    theme, out = sys.argv[1], sys.argv[2]
    w = int(sys.argv[3]) if len(sys.argv) > 4 else 3840
    h = int(sys.argv[4]) if len(sys.argv) > 4 else 2400
    img = render(theme, w, h)
    if out.endswith(".png"):  # the boot menu's background (GRUB and syslinux read PNG)
        img.save(out, optimize=True)
    else:
        img.save(out, quality=94, subsampling=0, optimize=True)


if __name__ == "__main__":
    main()
