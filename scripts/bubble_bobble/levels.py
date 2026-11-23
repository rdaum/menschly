"""Extract each room's 8×8 terrain, palettes and 32×26 display map."""

from PIL import Image


ROOMS = 102
WIDTH = 32
ROLES = ("EMPTY", "WALL_TL", "WALL_TR", "WALL_BL", "WALL_BR", "PLATFORM",
         "SHADOW_BOTTOM_LEFT", "SHADOW_LEFT_BEGIN", "SHADOW_BOTTOM",
         "SHADOW_BOTTOM_RIGHT", "SHADOW_LEFT", "SHADOW_BOTTOM_BEGIN")
# Indexed by three solid neighbours: left (4), above (2), upper left (1).
SHADOW = (0, 9, 11, 8, 7, 10, 6, 6)


def border(style):
    row = [True] * WIDTH
    for x, code in [(9, style >> 2), (19, style & 3)]:
        if code < 2:
            row[x:x + 4] = [False] * 4
    return row


def room_map(screen, gates):
    solid = [border(gates >> 4)]
    # Eight MSB-first 3-bit cells per three bytes. Zero denotes solid terrain.
    inside = []
    for pos in range(0, len(screen), 3):
        word = int.from_bytes(screen[pos:pos + 3], "big")
        inside.extend((word >> shift) & 7 == 0 for shift in range(21, -1, -3))
    solid.extend(inside[i:i + WIDTH] for i in range(0, len(inside), WIDTH))
    solid.append(border(gates & 15))
    cells = bytearray()
    for y, row in enumerate(solid):
        for x, wall in enumerate(row):
            if wall:
                side = x < 2 or x >= WIDTH - 2
                cells.append(1 + (x % 2) + (y % 2) * 2 if side else 5)
            else:
                left = x > 0 and row[x - 1]
                above = y > 0 and solid[y - 1][x]
                # At the left border the bottom shadow extends to the edge.
                corner = y > 0 and solid[y - 1][max(0, x - 1)]
                cells.append(SHADOW[(left << 2) | (above << 1) | corner])
    return cells


def write_levels(destination, tiles, palette, colors, program, screens):
    atlas = Image.new("P", (8 * len(ROLES), 8 * ROOMS))
    atlas.putpalette(bytes(c for rgb in palette for c in rgb))
    characters, palettes, maps = bytearray(), bytearray(), bytearray()
    index = ["cell\tname\trom_tile\tnormal_bank\tsuper_bank\n"]
    # Local FCM index zero is the backdrop. Visible ROM pens 0..14 become 1..15.
    local_indices = bytes(range(1, 16)) + bytes(241)
    blank = Image.new("L", (8, 8), 15)
    for room in range(ROOMS):
        record = 0x673A + 43 * room
        normal = 16 + program[record]
        super_bank = 16 + (program[record + 14] + 3) % 8
        if not 16 <= normal < 24:
            raise ValueError(f"Invalid terrain palette for room {room + 1}")
        wall = 0x204 + 5 * room
        codes = [None, *range(wall, wall + 5), *range(0xF0, 0xF6)]
        if room == 101:  # The secret-room tiles live in the fifth graphics chip pair.
            codes = [None, *range(0x2495, 0x2499), 0x2494, *([None] * 6)]
            normal = super_bank = 26
        for bank in (normal, super_bank):
            pens = [0, *colors[16 * bank:16 * bank + 15]]
            palettes.extend(c for pen in pens for c in palette[pen])
        display = colors[normal * 16:normal * 16 + 15] + [0] * 241
        for role, (name, code) in enumerate(zip(ROLES, codes)):
            tile = blank if code is None else tiles.crop((0, 8 * code, 8, 8 * (code + 1)))
            characters.extend(tile.tobytes().translate(local_indices))
            atlas.paste(Image.frombytes("P", (8, 8), tile.point(display).tobytes()),
                        (8 * role, 8 * room))
            address = "-" if code is None else f"{code:04x}"
            index.append(f"{room * len(ROLES) + role}\tROOM_{room+1:03}_{name}\t"
                         f"{address}\t{normal}\t{super_bank}\n")
        start = 0xCFC + room * 288
        maps.extend(room_map(screens[start:start + 288], program[record + 3]))
    atlas.save(destination / "level_tiles.png", transparency=0)
    (destination / "level_tiles.tsv").write_text("".join(index), encoding="ascii")
    (destination / "level_tiles.bin").write_bytes(characters)
    (destination / "level_palettes.rgb").write_bytes(palettes)
    (destination / "rooms.bin").write_bytes(maps)
