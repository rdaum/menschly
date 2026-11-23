# Bubble Bobble assets for MEGA65

From this directory, run the two offline tools with Python 3 and Pillow 9.1
or newer. The extractor accepts an unpacked US/Romstar MAME set or a complete
`bublboblr.zip`. The converter reads the generated atlas and name index.

```sh
python3 extract.py ../../examples/bubs
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
centred at their native resolution. In [frames.tsv](frames.tsv), adjacent
rows with the same name assemble one frame. A tile code of `@NAME` reuses an
earlier assembly. Other rows give the tile code in hex, dimensions in tiles,
pixel offsets, tile order, flips, palette bank and optional `pen=entry`
colour overrides. The [MAME graphics driver](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl.cpp)
and [video renderer](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl_v.cpp)
describe the ROM layout and transparent pen.

| Converter output | Layout |
| --- | --- |
| `sprites.bin` | 128 bytes per cell, 8 bytes per row, left pixel in the high nibble |
| `sprite_palettes.rgb` | 48 bytes per cell, 16 RGB triples. Nibble 0 is transparent. Opaque black gets its own index. |
| `bubble-frames.mensch` | Module `BubbleFrames` exports all frame names as `u16` cell indices. |

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

`run-demo.sh` builds and boots a demo of six characters cycling their
animations in Xemu:

```sh
scripts/bubble_bobble/run-demo.sh             # build and run
scripts/bubble_bobble/run-demo.sh --no-run    # build the ROM only
```

The 249 KB asset pack does not fit the kernel's 24 KB image segment, so
`demo_assets.py` extracts the 36 cells the demo's six cycles use and writes
them as module `BubbleDemoData` (`target/bubble-bobble/bubble-demo-data.mensch`):
exported image and palette tables and the frame names, renumbered as demo
cells. At startup the demo copies the images by DMA to `$46000` and puts the
sprite pointer table at `$45000`, because the VIC-IV reads the table from a
16-byte boundary and each image from a 64-byte one.

`bubble-anim.mensch` is the animation library: `anim_init`, `anim_show` (which
skips work when a slot already shows that frame), `anim_place`, and `anim_run`,
which steps every cycle in parallel one frame per `hold` screen frames.

## Calling from Mensch

Compile [bubble-video.mensch](bubble-video.mensch), the generated
`bubble-frames.mensch`, and [vic4.mensch](../../lib/vic4.mensch) with your
game. The sprite images, pointer table and active FCM characters belong
in chip RAM. The full five files total 422,384 bytes. On a 384 KiB machine,
keep the sprite images and palettes resident and stage one room at a time.
For room N, read 768 terrain bytes at offset `(N-1)*768`, 96 palette bytes
at `(N-1)*96`, and 832 map bytes at `(N-1)*832`. Pass those three buffers
as record 1 to `bb_draw_room`. With complete files in addressable memory,
pass N directly. The sprite image base and FCM tile destination must be 64-byte
aligned. Reserve 16 aligned bytes for the sprite pointer table and 768 bytes
at a full-colour tile address (character number at least $100).

```text
import BubbleFrames
import BubbleVideo
import Vic4

# Inside initialization, with room 1 staged:
vic4_init()
vic4_text16(SCREEN, 32, 0)
vic4_fill_colour(0, 0, 0, 32 * 26)
bb_draw_room(1, 0, LEVEL_TILES, LEVEL_PALETTES, ROOM_MAPS, TILE_RAM)
bb_sprites_init(SPRITE_TABLE, $01, 1)
bb_sprite_frame(0, BUBBLUN_WALK_01, SPRITE_IMAGES, SPRITE_PALETTES)
vic4_sprite_place(0, 120, 100)
vic4_sprites_on($01)
```

The example names in capitals other than the generated frame name are
application-supplied addresses. The room wrapper writes 26 screen rows;
reserve that screen memory and choose a display height or viewport accordingly.
Its second argument selects normal (0) or super (nonzero) colours. Sprites
use palette bank 1 in this example, leaving bank 0 for the room.
