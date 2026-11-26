#!/usr/bin/env python3
"""Make human-viewable previews of the extracted Bubble Bobble assets.

Writes to target/bubble-bobble/preview/:
  sprites-sheet.png   the raw atlas on a checkerboard, 3x nearest-neighbour
  sprites-labelled.png the atlas with a cell grid and index numbers, 2x
  characters.png       the 12 walk cells at 8x with their column-0/15 edge marked
  level-tiles.png      the terrain atlas, 4x, on a grey backdrop
  room-102.png         room 102 assembled from its map + tiles, 2x
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
D = ROOT / "target/bubble-bobble"
OUT = D / "preview"
CELL = 16


def checkerboard(size, square=8):
    """A grey checkerboard so transparent pixels are obvious."""
    board = Image.new("RGBA", size, (200, 200, 200, 255))
    draw = ImageDraw.Draw(board)
    for y in range(0, size[1], square):
        for x in range(0, size[0], square):
            if (x // square + y // square) % 2:
                draw.rectangle([x, y, x + square - 1, y + square - 1], fill=(150, 150, 150, 255))
    return board


def over(image, background):
    return Image.alpha_composite(background.convert("RGBA"), image.convert("RGBA"))


def read_names():
    cells = {}
    for line in (D / "sprites.tsv").read_text().splitlines()[1:]:
        cell, name = line.split("\t")
        cells.setdefault(int(cell), name)
    return cells


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    atlas = Image.open(D / "sprites.png").convert("RGBA")
    names = read_names()
    columns = atlas.width // CELL

    # 1. Whole sheet, enlarged, on a checkerboard.
    big = atlas.resize((atlas.width * 3, atlas.height * 3), Image.Resampling.NEAREST)
    over(big, checkerboard(big.size, 24)).save(OUT / "sprites-sheet.png")

    # 2. The same with a grid and cell numbers.
    labelled = over(atlas, checkerboard(atlas.size, 4)).resize(
        (atlas.width * 2, atlas.height * 2), Image.Resampling.NEAREST)
    draw = ImageDraw.Draw(labelled)
    for cell in sorted(names):
        x, y = (cell % columns) * CELL * 2, (cell // columns) * CELL * 2
        draw.rectangle([x, y, x + CELL * 2 - 1, y + CELL * 2 - 1], outline=(60, 60, 60, 255))
        if cell % 8 == 0:
            draw.text((x + 1, y + 16), str(cell), fill=(255, 0, 255, 255))
    labelled.save(OUT / "sprites-labelled.png")

    # 3. The walk cycles at 8x, showing horizontal clipping plainly.
    scale = 8
    shown = [n for n in names.values() if n.endswith(tuple(f"_0{i}" for i in range(1, 7)))
             and ("BUBBLUN_WALK" in n or "BOBBLUN_WALK" in n)]
    shown.sort()
    shown = [n for n in shown if "_WALK_" in n][:12]
    pad = 4
    w = len(shown) * (CELL * scale + pad) + pad
    strip = Image.new("RGBA", (w, CELL * scale + pad * 2 + 14), (30, 30, 36, 255))
    for i, name in enumerate(shown):
        cell = next(c for c, n in names.items() if n == name)
        cx, cy = (cell % columns) * CELL, (cell // columns) * CELL
        art = atlas.crop((cx, cy, cx + CELL, cy + CELL))
        art = over(art, checkerboard(art.size, 4)).resize(
            (CELL * scale, CELL * scale), Image.Resampling.NEAREST)
        draw = ImageDraw.Draw(art)
        # Mark the leftmost and rightmost pixel columns: art touching these is clipped.
        draw.rectangle([0, 0, scale - 1, CELL * scale - 1], outline=(255, 0, 0, 255))
        draw.rectangle([CELL * scale - scale, 0, CELL * scale - 1, CELL * scale - 1],
                       outline=(255, 0, 0, 255))
        strip.paste(art, (pad + i * (CELL * scale + pad), pad))
        ImageDraw.Draw(strip).text((pad + i * (CELL * scale + pad), CELL * scale + pad + 2),
                                   name.replace("BUBBLUN_", "B_").replace("BOBBLUN_", "K_")[-12:],
                                   fill=(220, 220, 220, 255))
    strip.save(OUT / "characters.png")

    # 4. Terrain tiles, 4x, on grey so index 0 (backdrop) is visible as such.
    tiles = Image.open(D / "level_tiles.png").convert("RGBA")
    t = over(tiles.resize((tiles.width * 4, tiles.height * 4), Image.Resampling.NEAREST),
             Image.new("RGBA", (tiles.width * 4, tiles.height * 4), (70, 70, 80, 255)))
    t.save(OUT / "level-tiles.png")

    # 5. Room 102 assembled from map + tiles.
    raw = (D / "rooms.bin").read_bytes()
    room = raw[101 * 832:102 * 832]
    # level_tiles.bin holds 768 bytes per room; take room 102's characters.
    bin_tiles = (D / "level_tiles.bin").read_bytes()[101 * 768:102 * 768]
    palettes = (D / "level_palettes.rgb").read_bytes()[101 * 96:101 * 96 + 96]
    pal = [tuple(palettes[i * 3:i * 3 + 3]) for i in range(32)]
    room_img = Image.new("RGB", (32 * 8, 26 * 8), pal[0])
    pixels = room_img.load()
    for row in range(26):
        for col in range(32):
            tile = room[row * 32 + col]
            for ty in range(8):
                for tx in range(8):
                    b = bin_tiles[tile * 64 + ty * 8 + tx]
                    pixels[col * 8 + tx, row * 8 + ty] = pal[b & 15]
    room_img.resize((room_img.width * 2, room_img.height * 2),
                    Image.Resampling.NEAREST).save(OUT / "room-102.png")

    print(f"previews in {OUT}")
    for f in sorted(OUT.iterdir()):
        print(" ", f.name, f.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
