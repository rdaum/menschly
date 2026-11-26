#!/usr/bin/env python3
"""Build a named 16×16 indexed atlas from a local Bubble Bobble arcade ROM set."""

import argparse
import csv
from dataclasses import dataclass
from itertools import groupby
from pathlib import Path
from zipfile import BadZipFile, ZipFile
from zlib import crc32

from PIL import Image

from levels import write_levels

HERE = Path(__file__).resolve().parent
CELL = 16
COLUMNS = 32
# File order in MAME's gfx1 region. Each half supplies two of the four bitplanes.
GRAPHICS = {
    0x20358C22: "a78-09.12", 0x930168A9: "a78-10.13", 0x9773E512: "a78-11.14",
    0xD045549B: "a78-12.15", 0xD0AF35C5: "a78-13.16", 0x7B5369A8: "a78-14.17",
    0x6B61A413: "a78-15.30", 0xB5492D97: "a78-16.31", 0xD69762D5: "a78-17.32",
    0x9F243B68: "a78-18.33", 0x66E9438C: "a78-19.34", 0x9EF863AD: "a78-20.35",
}
PALETTE_CRC = 0x71191B1F
ENDING_PALETTE_CRC = 0x88BFB15B
RASCAL_PALETTE_CRC = 0x799DCA46
OPAQUE = [0] + [255] * 255


def rom_files(source):
    sizes = {0x8000, 0x10000}
    if source.is_dir():
        for path in sorted(source.rglob("*")):
            if path.is_file() and path.stat().st_size in sizes:
                yield path.read_bytes()
    else:
        with ZipFile(source) as archive:
            for member in archive.infolist():
                if member.file_size in sizes:
                    yield archive.read(member)


def read_rom(source, *, with_rooms=False):
    chips = {}
    palette = None
    rascal = None
    screen_rom = None
    program_rom = None
    for data in rom_files(source):
        if len(data) == 0x8000:
            checksum = crc32(data)
            if checksum in GRAPHICS:
                chips[checksum] = data
            elif checksum == 0xAE11A07B:
                screen_rom = data
            elif crc32(data[0x555a:0x55ba]) == RASCAL_PALETTE_CRC:
                rascal = data[0x555a:0x55ba]
        else:
            table = data[0x4000:0x4300]
            ending = data[0x2886:0x28c6]
            if crc32(table) == PALETTE_CRC and crc32(ending) == ENDING_PALETTE_CRC:
                palette = table + ending
                program_rom = data
    missing = [name for checksum, name in GRAPHICS.items() if checksum not in chips]
    if palette is None:
        missing.append("64 KiB banked program ROM with the supported palette table (e.g. a78-24.52)")
    if rascal is None:
        missing.append("32 KiB program ROM with the Rascal palettes (e.g. a78-25.51)")
    if with_rooms and screen_rom is None:
        missing.append("a78-08.37 (room layouts)")
    if missing:
        raise ValueError("Missing or unsupported ROM data: " + ", ".join(missing))
    graphics = [chips[checksum] for checksum in GRAPHICS]
    if with_rooms:
        return graphics, palette + rascal, program_rom, screen_rom
    return graphics, palette + rascal


def decode_tiles(chips):
    """Return the populated 8×8 ROM tiles in MAME tile-code order as an 8px strip."""
    lo = b"".join(chips[:6])
    hi = b"".join(chips[6:])
    pixels = bytearray()
    for a, b in zip(lo, hi):
        for bit in range(4):
            pen = (((a >> (4 + bit)) & 1) << 3
                   | ((a >> bit) & 1) << 2
                   | ((b >> (4 + bit)) & 1) << 1
                   | ((b >> bit) & 1))
            pixels.append(pen ^ 15)  # MAME marks gfx1 ROMREGION_INVERT
    return Image.frombytes("L", (8, len(pixels) // 8), bytes(pixels))


def decode_palettes(table):
    # RGBx444, big endian. PNG index 0 is reserved even when a ROM pen is opaque black.
    colors = []
    indices = []
    for high, low in zip(table[::2], table[1::2]):
        rgb = (17 * (high >> 4), 17 * (high & 15), 17 * (low >> 4))
        if rgb not in colors:
            colors.append(rgb)
        indices.append(1 + colors.index(rgb))
    return [(0, 0, 0), *colors], indices


@dataclass(frozen=True)
class Part:
    x: int
    y: int
    tile: int
    width: int
    height: int
    order: str
    flip: str
    palette: int
    overrides: str


def read_frames():
    recipes = {}
    with (HERE / "frames.tsv").open(encoding="ascii", newline="") as stream:
        for name, rows in groupby(csv.DictReader(stream, delimiter="\t"), key=lambda r: r["name"]):
            rows = list(rows)
            tile = rows[0]["tile"]
            if tile.startswith("@"):
                if len(rows) != 1 or tile[1:] not in recipes:
                    raise ValueError(f"Invalid frame alias {name}: {tile}")
                parts = recipes[tile[1:]]
            else:
                parts = [
                    Part(int(r["x"]), int(r["y"]), int(r["tile"], 16), int(r["width"]),
                         int(r["height"]), r["order"], r["flip"], int(r["bank"]), r["overrides"])
                    for r in rows
                ]
            recipes[name] = parts
            yield name, parts


def tile_position(i, part):
    if part.order == "rows":
        return i % part.width, i // part.width
    if part.order == "cols":
        return i // part.height, i % part.height
    # Four sequential tiles form a 16×16 block within a larger graphic.
    block = i // 4
    if part.order == "blocks":
        bx, by = block % (part.width // 2), block // (part.width // 2)
    elif part.order == "vertical_blocks":
        bx, by = block // (part.height // 2), block % (part.height // 2)
    else:
        raise ValueError(f"Unknown tile order: {part.order}")
    return bx * 2 + i % 2, by * 2 + (i % 4) // 2


def render_part(part, tiles, colors):
    image = Image.new("L", (part.width * 8, part.height * 8))
    for i in range(part.width * part.height):
        y = (part.tile + i) * 8
        tile = tiles.crop((0, y, 8, y + 8))
        tx, ty = tile_position(i, part)
        image.paste(tile, (tx * 8, ty * 8))
    if "x" in part.flip:
        image = image.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    if "y" in part.flip:
        image = image.transpose(Image.Transpose.FLIP_TOP_BOTTOM)
    pens = colors[part.palette * 16:part.palette * 16 + 16]
    for override in part.overrides.split():
        pen, entry = override.split("=")
        pens[int(pen)] = colors[int(entry)]
    pens[15] = 0
    return image.point(pens + [0] * 240)


def render_cell(name, parts, tiles, colors):
    left = min(part.x for part in parts)
    top = min(part.y for part in parts)
    right = max(part.x + 8 * part.width for part in parts)
    bottom = max(part.y + 8 * part.height for part in parts)
    frame = Image.new("L", (right - left, bottom - top))
    for part in parts:
        image = render_part(part, tiles, colors)
        frame.paste(image, (part.x - left, part.y - top), image.point(OPAQUE))
    bounds = frame.getbbox()
    if bounds is None:
        raise ValueError(f"Empty frame: {name}")
    # Retain the ROM origin, including the transparent rows above crouching
    # characters. Some small glyphs straddle tiles on a larger assembly canvas.
    if max(frame.size) > CELL and max(bounds[2] - bounds[0], bounds[3] - bounds[1]) <= CELL:
        frame = frame.crop(bounds)
    frame.thumbnail((CELL, CELL), Image.Resampling.NEAREST)
    cell = Image.new("L", (CELL, CELL))
    cell.paste(frame, ((CELL - frame.width) // 2, (CELL - frame.height) // 2))
    used = set(cell.tobytes()) - {0}
    if not 1 <= len(used) <= 15:
        raise ValueError(f"{name} has {len(used)} visible colours after scaling")
    return cell


def generate(source, destination):
    chips, table, program_rom, screen_rom = read_rom(source, with_rooms=True)
    tiles = decode_tiles(chips)
    palette, colors = decode_palettes(table)
    frames = list(read_frames())
    names = [name for name, _ in frames]
    if len(names) != len(set(names)):
        raise ValueError("Duplicate frame name in catalog")
    cells = {}
    index = ["cell\tname\n"]
    for name, parts in frames:
        pixels = render_cell(name, parts, tiles, colors).tobytes()
        cell = cells.setdefault(pixels, len(cells))
        index.append(f"{cell}\t{name}\n")
    rows = (len(cells) + COLUMNS - 1) // COLUMNS
    atlas = Image.new("P", (COLUMNS * CELL, rows * CELL))
    atlas.putpalette(bytes(channel for rgb in palette for channel in rgb))
    for pixels, i in cells.items():
        atlas.paste(Image.frombytes("P", (CELL, CELL), pixels),
                    (i % COLUMNS * CELL, i // COLUMNS * CELL))
    destination.mkdir(parents=True, exist_ok=True)
    atlas.save(destination / "sprites.png", transparency=0)
    (destination / "sprites.tsv").write_text("".join(index), encoding="ascii")
    write_levels(destination, tiles, palette, colors, program_rom, screen_rom)
    print(f"{len(frames)} names for {len(cells)} unique sprites, {CELL}×{CELL}px, {COLUMNS} columns × {rows} rows")
    print(f"{destination / 'sprites.png'}\n{destination / 'sprites.tsv'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rom", type=Path, help="MAME ZIP or directory of unpacked ROM chips")
    parser.add_argument("--output-dir", type=Path,
                        default=HERE.parents[2] / "target/bubble-bobble", help="output directory")
    args = parser.parse_args()
    try:
        generate(args.rom, args.output_dir)
    except (OSError, ValueError, BadZipFile) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")


if __name__ == "__main__":
    main()
