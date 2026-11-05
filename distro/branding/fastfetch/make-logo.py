#!/usr/bin/env python3
"""Regenerate raze.txt, the fastfetch logo, from the Raze mark (art/logo.svg).

The mark is split into the hexagon frame and the letter (both cut by the blade, which
is drawn thinner than in the SVG so it stays visible at this size), rasterised with
resvg and drawn with half blocks: every character cell holds two pixels, one above
the other. $1-$3 colour the frame and $4-$6 the letter, as a diagonal gradient
(colours are set in config.jsonc).

    nix shell nixpkgs#resvg 'nixpkgs#python3Packages.pillow' -c python3 make-logo.py > raze.txt
"""
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

COLS, PIXEL_ROWS = 50, 44  # terminal lines are taller than two square pixels
BLADE = '<path d="M60 206 L206 52" stroke="#000" stroke-width="6" stroke-linecap="round"/>'
HEXAGON = '<path d="M128 8 L232 68 L232 188 L128 248 L24 188 L24 68 Z" fill="none" stroke="#fff" stroke-width="14" stroke-linejoin="round"/>'
LETTER = ('<path d="M82 70 H146 C176 70 192 88 192 112 C192 132 180 146 160 151 L194 196 H162 L131 154 H108 '
          'V196 H82 Z M108 92 V132 H144 C158 132 166 124 166 112 C166 100 158 92 144 92 Z" fill="#fff"/>')


def raster(shape, tmp):
    svg = Path(tmp) / "shape.svg"
    png = Path(tmp) / "shape.png"
    svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">'
                   f'<rect width="256" height="256" fill="#000"/>{shape}{BLADE}</svg>')
    subprocess.run(["resvg", "--width", "1024", "--height", "1024", str(svg), str(png)], check=True)
    return Image.open(png).convert("L").resize((COLS, PIXEL_ROWS), Image.BOX)


def main():
    with tempfile.TemporaryDirectory() as tmp:
        frame, letter = raster(HEXAGON, tmp), raster(LETTER, tmp)

    def kind(x, y):  # 0 empty, 1 frame, 2 letter
        if letter.getpixel((x, y)) > 110:
            return 2
        return 1 if frame.getpixel((x, y)) > 110 else 0

    def colour(x, y, k):
        t = (x / COLS + y / PIXEL_ROWS) / 2
        return (0 if t < 0.40 else 1 if t < 0.62 else 2) + (1 if k == 1 else 4)

    for row in range(PIXEL_ROWS // 2):
        line, current = "", None
        for x in range(COLS):
            top, bottom = kind(x, 2 * row), kind(x, 2 * row + 1)
            if not top and not bottom:
                ch, col = " ", None
            elif top and top == bottom:
                ch, col = "█", colour(x, 2 * row, top)
            elif top and not bottom:
                ch, col = "▀", colour(x, 2 * row, top)
            elif bottom and not top:
                ch, col = "▄", colour(x, 2 * row + 1, bottom)
            else:  # frame and letter share the cell: the letter wins
                ch, col = ("▀" if top == 2 else "▄"), colour(x, 2 * row, 2)
            if col is not None and col != current:
                line += f"${col}"
                current = col
            line += ch
        print(line.rstrip())


if __name__ == "__main__":
    sys.exit(main())
