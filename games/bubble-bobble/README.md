# Bubble Bobble assets for MEGA65

From `tools/`, run the two offline tools with Python 3 and Pillow 9.1
or newer. The extractor accepts an unpacked US/Romstar MAME set or a complete
`bublboblr.zip`. The converter reads the generated atlas and name index.

```sh
python3 extract.py ../assets/bubs
python3 convert.py
```

Both default to `target/bubble-bobble/` at the repository root. To use another
directory, run `python3 extract.py /path/to/bublboblr.zip --output-dir assets`
and then `python3 convert.py assets`. All ROM-derived outputs are local.

## Sprites

`sprites.png` has 1,417 distinct 16×16 cells on a 32 × 45 grid, packed
in row-major order. The last 23 cells are empty. `sprites.tsv` maps all
1,447 animation names to cells. Shared effects and repeated ROM poses alias
the same cell. Look up each animation step by name. Cell numbers have changed
since the first atlas and consecutive steps can share an index.

The extractor preserves the tile origin for native 16×16 frames. Short poses
keep their vertical padding and full-height poses can have feet on row 15.
Larger assemblies include the full extent of every 8×8 tile before
nearest-neighbour reduction. Small symbols spanning a larger tile canvas are
centred at their native resolution. In [frames.tsv](tools/frames.tsv), adjacent
rows with the same name assemble one frame. A tile code of `@NAME` reuses an
earlier assembly. Other rows give the tile code in hex, dimensions in tiles,
pixel offsets, tile order, flips, palette bank and optional `pen=entry`
colour overrides. The [MAME graphics driver](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl.cpp)
and [video renderer](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl_v.cpp)
describe the ROM layout and transparent pen.

The converter stores each distinct image and palette once: the 1,417 cells
use 885 images and 592 palettes, 147,364 bytes in all. Many cells share
pixels and differ only in colour (most of Bob's frames are Bub's), and a cell
that is another's mirror image reuses it.

| Converter output | Layout |
| --- | --- |
| `sprite_images.bin` | 128 bytes per image, 8 bytes per row, left pixel in the high nibble |
| `sprite_palettes.rgb` | 48 bytes per palette, 16 RGB triples. Nibble 0 is transparent. Opaque black gets its own index. |
| `sprite_cells.bin` | 4 bytes per cell: image number (bit 15 set when the cell is that image mirrored) and palette number, little-endian |
| `bubble-frames.mensch` | Module `BubbleFrames` exports all frame names as `u16` cell numbers. |

## Terrain and room maps

`level_tiles.png` is a 12-column × 102-row atlas of 8×8 tiles in normal-game
colours. Each row contains a blank, four side-wall quadrants (TL, TR, BL, BR),
a platform, and six shadow tiles. `level_tiles.tsv` gives their names, source
tile codes and normal/super palette banks. Row 102 uses the secret-room art.
Round numbers in names and in the Mensch API start at 1.

| Extractor output | Record per room |
| --- | --- |
| `level_tiles.bin` | 768 bytes, twelve 64-byte FCM characters in atlas order. Each pixel is a local palette index. |
| `level_palettes.rgb` | 96 bytes, normal then super. Each is 16 RGB triples with index 0 reserved for the backdrop. |
| `rooms.bin` | 832 bytes, a 32-column × 26-row display map in row-major order. Values 0–11 select that room's terrain tiles. |

The extractor reads terrain from the graphics chips, palette selectors and
openings from the banked program ROM, and room interiors from `a78-08.37`.
The map includes wall quadrants and neighbour-dependent shadows. The arcade
layout is documented in [Patch-A-Bobble's terrain code](https://github.com/kstenerud/patch-a-bobble/blob/master/src/Gfxset.cpp)
and [room decoder](https://github.com/kstenerud/patch-a-bobble/blob/master/src/Lvlset.cpp).

## Animation demo

`run-demo.sh` builds and boots a demo in Xemu: six characters standing in
room 1, each cycling its animation, under two rows of 22 bobbing bubbles.

```sh
games/bubble-bobble/run-demo.sh             # build and run
games/bubble-bobble/run-demo.sh --no-run    # build the ROM only
```

The 147 KB sprite pack does not fit the kernel's 24 KB image segment, so
`demo_assets.py` extracts the 33 cells the demo's six cycles use, in the
converter's format, and room 1, and writes them as module `BubbleDemoData`
(`target/bubble-bobble/bubble-demo-data.mensch`) with the frame names
renumbered as demo cells.

Everything on screen is an actor ([actors.mensch](../../lib/actors.mensch)).
The demo's `KIND` table declares which of the cast are hardware sprites and
which are drawn in software by the VIC-IV's raster rewrite buffer; Maita and
the bubbles are software, since up to 22 of them share a text row.
`bubble-anim.mensch` holds the demo's cycles: `anim_tick` steps them once a
screen frame and `anim_frame` gives each one's current frame.

## Calling from Mensch

Compile [bubble-video.mensch](src/bubble-video.mensch) (room drawing), the
generated `bubble-frames.mensch`, [actors.mensch](../../lib/actors.mensch)
and [vic4.mensch](../../lib/vic4.mensch) with your game. The sprite files
can be anywhere DMA reaches, inside or outside chip RAM; the actors copy
each cell they show into chip RAM. The six files total 320,356 bytes; keep
the sprite files resident and stage one room at a time. For room N, read
768 terrain bytes at offset `(N-1)*768`, 96 palette bytes at `(N-1)*96`,
and 832 map bytes at `(N-1)*832`. Pass those three buffers as record 1 to
`bb_draw_room`. With complete files in addressable memory, pass N directly.

In chip RAM, reserve 768 bytes at a 64-byte aligned full-colour tile address
(character number at least $100) for the room's terrain, 16 aligned bytes
for the sprite pointer table, eight 128-byte buffers for the hardware
sprites, and a 128-byte entry per distinct cell software actors show at once.

```text
import BubbleFrames
import BubbleVideo
import Actors
import Vic4

# Inside initialization, with room 1 staged:
vic4_init()
actors_sheet(SPRITE_IMAGES, SPRITE_PALETTES, SPRITE_CELLS)
actors_software(SCREEN, 40, 26, 24, ACTOR_CHARACTERS, 32)   # also sets up the screen
actors_hardware(SPRITE_TABLE, SPRITE_BUFFERS, 1, 24, 46)
bb_draw_room(1, 0, LEVEL_TILES, LEVEL_PALETTES, ROOM_MAPS, TILE_RAM, 4)
hero = actor_new(ACTOR_HARDWARE)
bubble = actor_new(ACTOR_SOFTWARE)

# Every frame, below the text area:
actor_draw(hero, 120, 100, BUBBLUN_WALK_01, 1)  # mirrored: facing right (the sprites face left)
actor_draw(bubble, 60, 20, BUBBLE_BLUE_01, 0)
actors_frame()
```

The example names in capitals other than the generated frame names are
application-supplied addresses. `actors_software` sets up a 26-row screen
with 40 background columns and 24 software-actor slots per row; the room
wrapper draws its 26 rows there, its second argument selecting normal (0)
or super (nonzero) colours and its last the screen column of the room's
left edge. Hardware sprites use sprite palette bank 1 here; software actors
use character palette bank 0's groups 1-15, leaving group 0 for the room.
