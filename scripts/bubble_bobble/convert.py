#!/usr/bin/env python3
"""Pack sprites.png and sprites.tsv for MEGA65 16-colour hardware sprites."""

import argparse
import csv
import re
from pathlib import Path

from PIL import Image


def pack_cell(cell, name):
    local = {(0, 0, 0, 0): 0}
    indices = []
    rgba = cell.convert("RGBA").tobytes()
    for offset in range(0, len(rgba), 4):
        r, g, b, a = rgba[offset:offset + 4]
        if a not in (0, 255):
            raise ValueError(f"{name}: partial alpha")
        color = (r, g, b, a) if a else (0, 0, 0, 0)
        indices.append(local.setdefault(color, len(local)))
    if len(local) > 16:
        raise ValueError(f"{name}: more than 15 opaque colours")
    image = bytes(left << 4 | right for left, right in zip(indices[::2], indices[1::2]))
    palette = bytes(channel for color in local for channel in color[:3]).ljust(48, b"\0")
    return image, palette


def convert(directory):
    with (directory / "sprites.tsv").open(encoding="ascii", newline="") as stream:
        records = csv.DictReader(stream, delimiter="\t")
        if records.fieldnames != ["cell", "name"]:
            raise ValueError("Expected cell and name columns in sprites.tsv")
        names = {}
        cells = {}
        for row in records:
            name, cell = row["name"], int(row["cell"])
            if not re.fullmatch(r"[A-Z][A-Z0-9_]*", name) or name in names:
                raise ValueError(f"Invalid or repeated sprite constant: {name}")
            names[name] = cell
            cells.setdefault(cell, name)
    images, palettes = bytearray(), bytearray()
    with Image.open(directory / "sprites.png") as atlas:
        columns = atlas.width // 16
        capacity = columns * (atlas.height // 16)
        if atlas.width % 16 or atlas.height % 16:
            raise ValueError("Sprite atlas dimensions must be multiples of 16")
        if not cells or len(cells) > min(capacity, 65536) or set(cells) != set(range(len(cells))):
            raise ValueError("Expected consecutive, zero-based cell numbers within the atlas")
        for i in range(len(cells)):
            x, y = (i % columns) * 16, (i // columns) * 16
            image, palette = pack_cell(atlas.crop((x, y, x + 16, y + 16)), cells[i])
            images.extend(image)
            palettes.extend(palette)
    constants = ["module BubbleFrames\n\n"]
    constants.extend(f"export const {name}: u16 = {cell}\n" for name, cell in names.items())
    (directory / "sprites.bin").write_bytes(images)
    (directory / "sprite_palettes.rgb").write_bytes(palettes)
    (directory / "bubble-frames.mensch").write_text("".join(constants), encoding="ascii")
    print(f"{len(cells)} sprites × 128 bytes, {len(names)} Mensch constants")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, nargs="?",
                        default=Path(__file__).resolve().parents[2] / "target/bubble-bobble",
                        help="directory containing the atlas and its index (also receives the binaries)")
    try:
        convert(parser.parse_args().directory)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
