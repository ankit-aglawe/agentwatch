#!/usr/bin/env python3
"""
Render assets/logo.txt as an SVG with a per-block vertical green gradient.

The CLI banner emits per-row ANSI colors; GitHub Markdown strips inline
styles, so the README hero needs an SVG instead. This script produces
assets/banner.svg with the same gradient shape: light at the top of each
letter, deep green at the bottom, gradient restarting per block (logo,
wordmark, tagline).

Usage:
    python3 scripts/banner2svg.py
"""

from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parent.parent
SRC  = ROOT / "assets" / "logo.txt"
DST  = ROOT / "assets" / "banner.svg"

# Brand gradient - must match the constants in
# crates/agentwatch-cli/src/main.rs (GRADIENT_TOP / GRADIENT_BOTTOM).
TOP_COLOR    = "#c4ecc0"   # light green (top of each letter)
BOTTOM_COLOR = "#6fb266"   # deep green  (bottom of each letter)

# Tuned for SF Mono / JetBrains Mono at 16px - GitHub's monospace metrics.
CHAR_W   = 9.6
LINE_H   = 22
PAD_X    = 24
PAD_Y    = 20
FONT_PX  = 16


def split_into_blocks(lines):
    """Return list[list[int]] - groups of line indices separated by blanks."""
    blocks, current = [], []
    for idx, line in enumerate(lines):
        if line.strip():
            current.append(idx)
        else:
            if current:
                blocks.append(current)
                current = []
    if current:
        blocks.append(current)
    return blocks


def main() -> None:
    lines = SRC.read_text().splitlines()
    max_w = max((len(line) for line in lines), default=1)
    svg_w = int(PAD_X * 2 + max_w * CHAR_W)
    svg_h = int(PAD_Y * 2 + len(lines) * LINE_H)

    blocks = split_into_blocks(lines)
    line_to_block = {}
    block_y = {}
    for bi, idxs in enumerate(blocks):
        first, last = idxs[0], idxs[-1]
        block_y[bi] = (PAD_Y + first * LINE_H, PAD_Y + (last + 1) * LINE_H)
        for i in idxs:
            line_to_block[i] = bi

    gradients = []
    for bi, (y_top, y_bottom) in block_y.items():
        gradients.append(
            f'<linearGradient id="brand-b{bi}" gradientUnits="userSpaceOnUse"\n'
            f'                    x1="0" y1="{y_top}" x2="0" y2="{y_bottom}">\n'
            f'      <stop offset="0%"   stop-color="{TOP_COLOR}"/>\n'
            f'      <stop offset="100%" stop-color="{BOTTOM_COLOR}"/>\n'
            f'    </linearGradient>'
        )

    tspans = []
    for idx, line in enumerate(lines):
        text = escape(line) if line else " "
        y = PAD_Y + (idx + 1) * LINE_H - 6
        fill = (
            f'url(#brand-b{line_to_block[idx]})'
            if idx in line_to_block
            else TOP_COLOR
        )
        tspans.append(
            f'<text x="{PAD_X}" y="{y}" fill="{fill}" xml:space="preserve">{text}</text>'
        )

    svg = f'''<svg xmlns="http://www.w3.org/2000/svg"
     viewBox="0 0 {svg_w} {svg_h}" width="{svg_w}" height="{svg_h}"
     role="img" aria-label="agentwatch - htop for AI coding agents">
  <defs>
    {chr(10).join("    " + g for g in gradients).lstrip()}
    <style>
      text {{
        font-family: 'JetBrains Mono','SF Mono','Fira Code',Menlo,Consolas,monospace;
        font-size: {FONT_PX}px;
        font-weight: 600;
        white-space: pre;
      }}
    </style>
  </defs>
  {chr(10).join("  " + t for t in tspans)}
</svg>
'''
    DST.write_text(svg)
    print(f"wrote {DST.relative_to(ROOT)} ({svg_w}x{svg_h}px, {len(lines)} lines, {len(blocks)} blocks)")


if __name__ == "__main__":
    main()
