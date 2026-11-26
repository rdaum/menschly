#!/usr/bin/env python3
"""Pack sprites.png and sprites.tsv for MEGA65 16-colour hardware sprites.

Each distinct image and palette is stored once:

  sprite_images.bin    128 bytes per image, 8 per row, left pixel in the high nibble
  sprite_palettes.rgb  48 bytes per palette, 16 RGB triples, entry 0 transparent
  sprite_cells.bin     4 bytes per cell: image number (bit 15 set when the
                       stored image is the cell mirrored left to right) and
                       palette number, both little-endian u16
  bubble-frames.mensch module BubbleFrames, every frame name as its cell
"""

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


MIRRORED = 0x8000


def mirror_image(image):
    """`image` (a packed 16x16 cell) mirrored left to right."""
    out = bytearray()
    for row in range(0, len(image), 8):
        out.extend((byte << 4 & 0xF0) | byte >> 4 for byte in reversed(image[row:row + 8]))
    return bytes(out)


def pack_set(cells):
    """Store each distinct image and palette of `cells`, (image, palette)
    pairs, once. Returns the images, the palettes and each cell's
    (image number, palette number), the image number with MIRRORED set when
    the cell is a stored image mirrored."""
    images, palettes, records = {}, {}, []
    for image, palette in cells:
        if image in images:
            number = images[image]
        elif mirror_image(image) in images:
            number = images[mirror_image(image)] | MIRRORED
        else:
            number = images.setdefault(image, len(images))
        records.append((number, palettes.setdefault(palette, len(palettes))))
    if len(images) >= MIRRORED or len(palettes) > 0xFFFF:
        raise ValueError("Too many distinct images or palettes")
    return list(images), list(palettes), records


def cell_table(records):
    return b"".join(image.to_bytes(2, "little") + palette.to_bytes(2, "little")
                    for image, palette in records)


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
    packed = []
    with Image.open(directory / "sprites.png") as atlas:
        columns = atlas.width // 16
        capacity = columns * (atlas.height // 16)
        if atlas.width % 16 or atlas.height % 16:
            raise ValueError("Sprite atlas dimensions must be multiples of 16")
        if not cells or len(cells) > min(capacity, 65536) or set(cells) != set(range(len(cells))):
            raise ValueError("Expected consecutive, zero-based cell numbers within the atlas")
        for i in range(len(cells)):
            x, y = (i % columns) * 16, (i // columns) * 16
            packed.append(pack_cell(atlas.crop((x, y, x + 16, y + 16)), cells[i]))
    images, palettes, records = pack_set(packed)
    constants = ["module BubbleFrames\n\n"]
    constants.extend(f"export const {name}: u16 = {cell}\n" for name, cell in names.items())
    (directory / "sprite_images.bin").write_bytes(b"".join(images))
    (directory / "sprite_palettes.rgb").write_bytes(b"".join(palettes))
    (directory / "sprite_cells.bin").write_bytes(cell_table(records))
    (directory / "bubble-frames.mensch").write_text("".join(constants), encoding="ascii")
    total = len(images) * 128 + len(palettes) * 48 + len(records) * 4
    print(f"{len(records)} cells: {len(images)} images, {len(palettes)} palettes, "
          f"{total:,} bytes; {len(names)} Mensch constants")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, nargs="?",
                        default=Path(__file__).resolve().parents[2] / "target/bubble-bobble",
                        help="directory containing the atlas and its index (also receives the binaries)")
    try:
        convert(parser.parse_args().directory)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
