#!/usr/bin/env python3
"""Generate the images of the SlateOS boot menu theme (distro/iso/grub-theme/).

    grub-theme.py INTER.ttf

The logo (the gem of icons/slate.svg and the word SlateOS, in Inter) and the pieces
GRUB stretches into the menu box and the highlight of the selected entry. A development
tool, like wallpaper.py: needs Pillow, and the images are committed. The background is
splash.png and the icons are nixpkgs' theme's, recoloured when the image is built.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "grub-theme")
SS = 4  # supersampling


def gem(size):
    """The diamond of slate.svg (its 36..100 square of 128), filled with its gradient."""
    s = size * SS
    grad = Image.new("RGBA", (s, s))
    stops = [(0.0, (0xB9, 0xC9, 0xFF)), (0.55, (0x7C, 0x9C, 0xF5)), (1.0, (0x8A, 0x6C, 0xF0))]
    px = grad.load()
    for y in range(s):
        for x in range(s):
            t = (x + y) / (2 * (s - 1))
            for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
                if t <= t1:
                    k = (t - t0) / (t1 - t0)
                    px[x, y] = tuple(round(a + (b - a) * k) for a, b in zip(c0, c1)) + (255,)
                    break
    mask = Image.new("L", (s, s))
    d = ImageDraw.Draw(mask)
    d.polygon([(s / 2, 0), (s, s / 2), (s / 2, s), (0, s / 2)], fill=255)
    out = Image.new("RGBA", (s, s))
    out.paste(grad, (0, 0), mask)
    # The lighter top-right facet.
    shine = Image.new("L", (s, s))
    ImageDraw.Draw(shine).polygon([(s / 2, 0), (s, s / 2), (s / 2, s / 2)], fill=round(255 * 0.22))
    out = Image.composite(Image.new("RGBA", (s, s), (255, 255, 255, 255)), out, shine)
    out.putalpha(mask)
    return out.resize((size, size), Image.LANCZOS)


def logo(font_path):
    w, h = 360, 100
    img = Image.new("RGBA", (w * SS, h * SS), (0, 0, 0, 0))
    f = ImageFont.truetype(font_path, 58 * SS)
    try:
        f.set_variation_by_axes([32, 600])  # opsz, wght: Inter SemiBold, as the menu bar
    except Exception:  # noqa: BLE001
        pass
    d = ImageDraw.Draw(img)
    text = "SlateOS"
    tw = d.textlength(text, font=f)
    g = 64
    gap = 18
    total = g + gap + tw / SS
    x0 = (w - total) / 2
    d.text(((x0 + g + gap) * SS, h * SS / 2), text, font=f, fill=(255, 255, 255, 255), anchor="lm")
    img = img.resize((w, h), Image.LANCZOS)
    img.alpha_composite(gem(g), (round(x0), (h - g) // 2))
    img.save(os.path.join(OUT, "logo.png"))


def nine(name, w, h, r, fill, border=None):
    """A rounded rectangle cut into GRUB's nine pieces: name_{nw,n,ne,w,c,e,sw,s,se}.png."""
    big = Image.new("RGBA", (w * SS, h * SS), (0, 0, 0, 0))
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, w * SS - 1, h * SS - 1), r * SS, fill=fill,
        outline=border, width=SS if border else 0,
    )
    img = big.resize((w, h), Image.LANCZOS)
    cuts = {
        "nw": (0, 0, r, r), "n": (r, 0, w - r, r), "ne": (w - r, 0, w, r),
        "w": (0, r, r, h - r), "c": (r, r, w - r, h - r), "e": (w - r, r, w, h - r),
        "sw": (0, h - r, r, h), "s": (r, h - r, w - r, h), "se": (w - r, h - r, w, h),
    }
    for k, box in cuts.items():
        img.crop(box).save(os.path.join(OUT, f"{name}_{k}.png"))


def main():
    os.makedirs(OUT, exist_ok=True)
    logo(sys.argv[1])
    nine("boot_menu", 64, 64, 16, (14, 16, 28, 190), (255, 255, 255, 30))
    # GRUB draws the selected entry's text lower by the highlight's corner size: keep the
    # corners small, or the entries look unevenly spaced.
    nine("select", 48, 40, 4, (90, 120, 232, 255))


main()
