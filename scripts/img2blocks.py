#!/usr/bin/env python3
"""
Convert a PNG into block-ASCII using Unicode quadrant blocks (▘▝▖▗▀▄▌▐▙▟▛▜█).

Each output character represents a 2×2 pixel block - so a target of W cols ×
H rows expands to a 2W × 2H pixel sampling grid. This is roughly what `chafa`
does in its block-only mode.

Usage:
    img2blocks.py INPUT.png --width 18 --height 6
"""

import argparse
from PIL import Image

# 16 quadrant block characters indexed by a 4-bit mask:
#   bit 0 (1) = top-left, bit 1 (2) = top-right,
#   bit 2 (4) = bottom-left, bit 3 (8) = bottom-right.
QUADRANTS = [
    " ", "▘", "▝", "▀",
    "▖", "▌", "▞", "▛",
    "▗", "▚", "▐", "▜",
    "▄", "▙", "▟", "█",
]


def load_alpha_mask(path: str, trim: bool = True) -> Image.Image:
    """Return a 1-bit ink/no-ink image, treating opaque dark pixels as ink.

    If `trim` is true, the bounding box of ink is cropped first so the icon
    fills the full output frame instead of being squeezed by surrounding
    transparent margin.
    """
    img = Image.open(path).convert("RGBA")
    r, g, b, a = img.split()
    if max(a.getextrema()) > 0 and min(a.getextrema()) < 255:
        mask = a.point(lambda v: 255 if v > 64 else 0).convert("1")
    else:
        gray = img.convert("L")
        mask = gray.point(lambda v: 255 if v < 128 else 0).convert("1")
    if trim:
        bbox = mask.getbbox()
        if bbox is not None:
            mask = mask.crop(bbox)
    return mask


def render(mask: Image.Image, cols: int, rows: int) -> str:
    px_w, px_h = cols * 2, rows * 2
    resized = mask.resize((px_w, px_h), Image.LANCZOS).convert("1")
    pixels = resized.load()
    out_lines = []
    for cy in range(rows):
        line = []
        for cx in range(cols):
            tl = 1 if pixels[cx * 2, cy * 2] else 0
            tr = 2 if pixels[cx * 2 + 1, cy * 2] else 0
            bl = 4 if pixels[cx * 2, cy * 2 + 1] else 0
            br = 8 if pixels[cx * 2 + 1, cy * 2 + 1] else 0
            line.append(QUADRANTS[tl + tr + bl + br])
        # Pad to full width (don't rstrip) - keeps rows aligned when the logo
        # is composited next to other text.
        out_lines.append("".join(line))
    return "\n".join(out_lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("input")
    ap.add_argument("--width", type=int, default=18)
    ap.add_argument("--height", type=int, default=6)
    args = ap.parse_args()
    mask = load_alpha_mask(args.input)
    print(render(mask, args.width, args.height))


if __name__ == "__main__":
    main()
