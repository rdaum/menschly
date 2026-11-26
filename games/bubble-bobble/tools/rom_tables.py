"""Write the arcade's tables the game copies, as a Mensch module, from the
main CPU's ROM dumps (re/main-fixed.bin, re/main-bank0.bin).

    python3 rom_tables.py [assets dir]   -> bubble-rom-tables.mensch

The tables:

- DEATH_STEPS: the step patterns of the 32 compass directions a popped
  enemy flies in ($10A2 in the fixed ROM), each ending in $88. A step byte's
  bit 0 moves 1 vertically (bit 3 down, else up), bit 4 horizontally (bit 7
  left, else right).
- DEATH_SCRIPT and BOUNCE_SCRIPT: per enemy slot, the flight a popped enemy
  makes ($80C8 in bank 0), and the one it makes after bouncing off a side
  wall ($815F): (direction, repeats) pairs ending in $99. Direction 0 is up,
  8 right, 16 down, mirrored (32 - d) when flying left.
- CHAIN_FOOD, CHAIN_POINTS, CHAIN_SCORE: the food a popped enemy lands as,
  by its place among the enemies one chain killed (bank 0 $8008), its points
  ($8031) and the floating score shown for them ($8026).
- ITEM_FOOD, ITEM_POINTS, ITEM_SCORE: the 60 items that appear on the map
  each round (bank 2 $B389, five bytes each: sprite, colour, score sprite,
  BCD points / 10).

Sprites are named from frames.tsv by tile (food in bank $F00, scores in
$E00; colour variants of a tile are not told apart; a tile frames.tsv lacks
is NO_SPRITE); rom-sprites.txt lists them for demo_assets.py. The demo data
lives in bank 0, which has room for only some of them: the food and scores
of the first CHAIN_KEPT enemies of a chain and of the items in ITEMS_KEPT;
the rest are NO_SPRITE (still scoring, not drawn) until the sprite sheet
moves out of bank 0.
"""

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
GAME = HERE.parent
ROOT = GAME.parent.parent
SLOTS = 7


def word(buf, at):
    return buf[at] | buf[at + 1] << 8


def table(name, data):
    rows = ",\n    ".join(", ".join(f"${v:02x}" for v in data[i:i + 16]) for i in range(0, len(data), 16))
    return f"export const {name}: [{len(data)}]u8 = [\n    {rows}\n]\n"


def offsets(name, values):
    return f"export const {name}: [{len(values)}]u16 = [{', '.join(str(v) for v in values)}]\n"


CHAIN_FOODS = 11
ITEMS = 60
CHAIN_KEPT = 7
ITEMS_KEPT = {30}


def tile_names():
    names = {}
    for line in (HERE / "frames.tsv").read_text().splitlines()[1:]:
        cols = line.split("\t")
        try:
            names.setdefault(int(cols[3], 16), cols[0])
        except ValueError:
            pass
    return names


def bcd(lo, hi):
    return int(f"{hi:02x}{lo:02x}") * 10


def cells(name, values):
    return f"export const {name}: [{len(values)}]u16 = [\n    " + ",\n    ".join(
        ", ".join(values[i:i + 4]) for i in range(0, len(values), 4)) + "\n]\n"


def words(name, values):
    return f"export const {name}: [{len(values)}]u16 = [{', '.join(str(v) for v in values)}]\n"


def main():
    assets = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "bubble-bobble"
    re_dir = assets / "re"
    fixed = (re_dir / "main-fixed.bin").read_bytes()
    bank0 = (re_dir / "main-bank0.bin").read_bytes()

    steps, step_at = [], []
    for direction in range(32):
        at = word(fixed, 0x10A2 + 2 * direction)
        step_at.append(len(steps))
        while fixed[at] != 0x88:
            steps.append(fixed[at])
            at += 1
        steps.append(0x88)

    def scripts(table_at):
        data, starts = [], []
        for slot in range(SLOTS):
            at = word(bank0, table_at - 0x8000 + 2 * slot) - 0x8000
            starts.append(len(data))
            while bank0[at] != 0x99:
                data += [bank0[at], bank0[at + 1]]
                at += 2
            data.append(0x99)
        return data, starts

    death, death_at = scripts(0x80C8)
    bounce, bounce_at = scripts(0x815F)

    bank2 = (re_dir / "main-bank2.bin").read_bytes()
    known = tile_names()

    class Names(dict):
        def __missing__(self, tile):
            return "NO_SPRITE"
    names = Names(known)
    chain_food = [names[0xF00 + bank0[0x8008 - 0x8000 + 2 * i]] for i in range(CHAIN_FOODS)]
    chain_points = [bcd(bank0[0x8031 - 0x8000 + 2 * i], bank0[0x8032 - 0x8000 + 2 * i])
                    for i in range(CHAIN_FOODS)]
    chain_score = [names[0xE00 + bank0[0x8026 - 0x8000 + i]] for i in range(CHAIN_FOODS)]
    item = [bank2[0xB389 - 0x8000 + 5 * i:][:5] for i in range(ITEMS)]
    item_food = [names[0xF00 + r[0]] for r in item]
    item_points = [bcd(r[3], r[4]) for r in item]
    item_score = [names[0xE00 + r[2]] for r in item]
    chain_food = [n if i < CHAIN_KEPT else "NO_SPRITE" for i, n in enumerate(chain_food)]
    chain_score = [n if i < CHAIN_KEPT else "NO_SPRITE" for i, n in enumerate(chain_score)]
    item_food = [n if i in ITEMS_KEPT else "NO_SPRITE" for i, n in enumerate(item_food)]
    item_score = [n if i in ITEMS_KEPT else "NO_SPRITE" for i, n in enumerate(item_score)]
    sprites = sorted(set(chain_food + chain_score + item_food + item_score) - {"NO_SPRITE"})
    (assets / "rom-sprites.txt").write_text("\n".join(sprites) + "\n")
    out = assets / "bubble-rom-tables.mensch"
    out.write_text(
        "module BubbleRomTables\n\n# Generated by rom_tables.py from the arcade's ROM; see there.\n\n"
        + "import BubbleDemoData\n\nexport const NO_SPRITE = $ffff\n\n"
        + cells("CHAIN_FOOD", chain_food) + words("CHAIN_POINTS", chain_points)
        + cells("CHAIN_SCORE", chain_score)
        + cells("ITEM_FOOD", item_food) + words("ITEM_POINTS", item_points)
        + cells("ITEM_SCORE", item_score)
        + table("DEATH_STEPS", steps) + offsets("DEATH_STEP_AT", step_at)
        + table("DEATH_SCRIPT", death) + offsets("DEATH_SCRIPT_AT", death_at)
        + table("BOUNCE_SCRIPT", bounce) + offsets("BOUNCE_SCRIPT_AT", bounce_at))
    print(out)


if __name__ == "__main__":
    main()
