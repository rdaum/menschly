"""Render the rotozoomer's texture: an original tiling design.

    python3 texture.py [out dir]

A 128x128 texture that tiles seamlessly: a 4x4 grid of bevelled chrome
tiles, each in one of eight colour families (so the rotation reads), with
glowing circuit traces running between them and a bright stud at each
tile's centre. Colours are palette entries family * 16 + shade (shade 0
dark to 15 bright), entries 0-127; the palette is ROTO_PALETTE.

Writes texture.png in the out dir to look at.
"""

import colorsys
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SIZE = 128
TILE = 32
FAMILIES = 8
SHADES = 16
# Each family's hue (0-1), picked to sit well together: cold blues and
# violets, a hot magenta and orange, a sharp cyan and acid green.
HUES = [0.58, 0.66, 0.76, 0.86, 0.95, 0.07, 0.50, 0.30]


def palette():
    rgb = [(0, 0, 0)] * 256
    for f, hue in enumerate(HUES):
        for s in range(SHADES):
            v = s / (SHADES - 1)
            # Dark shades saturated, the brightest washing out to white.
            r, g, b = colorsys.hsv_to_rgb(hue, 0.95 - 0.75 * v ** 3, 0.08 + 0.92 * v ** 0.9)
            rgb[f * SHADES + s] = (round(r * 255), round(g * 255), round(b * 255))
    return rgb


def texture():
    ys, xs = np.mgrid[0:SIZE, 0:SIZE] + 0.5
    tx, ty = (xs // TILE).astype(int), (ys // TILE).astype(int)
    lx, ly = xs % TILE - TILE / 2, ys % TILE - TILE / 2
    family = (tx + ty * 3) % FAMILIES
    # The tile: a raised square with a bevel lit from the top left.
    edge = TILE / 2 - np.maximum(np.abs(lx), np.abs(ly))
    bevel = np.clip(edge / 4, 0, 1)
    light = np.where(np.abs(lx) > np.abs(ly), -np.sign(lx), -np.sign(ly)) * (1 - bevel)
    face = 0.42 + 0.12 * np.sin((lx + ly) * 0.35)
    shade = face + 0.45 * light
    # A groove between tiles.
    shade = np.where(edge < 1.2, 0.05, shade)
    # Circuit traces: thin lines along each tile's middle, glowing.
    trace = (np.abs(lx) < 1.0) & (np.abs(ly) < TILE / 2 - 5) | (np.abs(ly) < 1.0) & (np.abs(lx) < TILE / 2 - 5)
    shade = np.where(trace & (edge > 3), 0.95, shade)
    # A ring and a stud at the centre.
    r = np.sqrt(lx ** 2 + ly ** 2)
    shade = np.where(np.abs(r - 8) < 1.0, 0.85, shade)
    shade = np.where(r < 3.0, 1.0 - r / 6, shade)
    level = np.clip(np.round(shade * (SHADES - 1)), 0, SHADES - 1).astype(np.uint8)
    return (family * SHADES + level).astype(np.uint8)


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid"
    out_dir.mkdir(parents=True, exist_ok=True)
    tex = texture()
    lut = np.array(palette(), dtype=np.uint8)
    Image.fromarray(lut[np.tile(tex, (2, 2))]).resize((SIZE * 6, SIZE * 6), Image.NEAREST).save(out_dir / "texture.png")
    print(f"texture: {SIZE}x{SIZE} -> {out_dir / 'texture.png'}")


if __name__ == "__main__":
    main()
