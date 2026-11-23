# Bubble Bobble sprite extractor

With Python 3 and Pillow 9.1 or newer, run the extractor on a local MAME
`bublboblr` (US/Romstar) ROM set. Supply a complete ZIP or a directory of
unpacked chips. The extractor checks the twelve graphics chips and the colour
tables in the two program ROMs by checksum. It runs offline. Run these commands from this directory.

```sh
python3 extract.py ../../examples/bubs
# Or supply an archive
python3 extract.py /path/to/bublboblr.zip
```

The default output directory is `target/bubble-bobble/` at the repository root. Use `--output-dir`
to choose another. `sprites.png` contains 1,447 named 16×16 cells in a
32-column, 46-row grid. `sprites.tsv` maps zero-based cell numbers to Mensch
constant names, starting with `BUBBLUN_STAND` and `BUBBLUN_WALK_01`. The
last 25 cells are empty. Wide and tall graphics are centred and reduced
with nearest-neighbour sampling.

Use a per-sprite hardware palette. The PNG reserves index 0 for transparency
and each cell uses at most 15 opaque colours. For each cell, remap its used
opaque PNG indices to local indices 1–15, including opaque black.

The frame catalog in [frames.tsv](frames.tsv) contains assembly
instructions. Consecutive rows with the same name draw parts of one frame.
Each part gives a hexadecimal ROM tile code, dimensions in 8×8 tiles, a
pixel offset, tile order, flips and a palette bank. Optional `pen=entry`
pairs select alternate colours from the ROM tables. Composite layouts follow
the supplied visual reference. Future level-map extraction can reuse
`read_rom` and `decode_tiles` to address the original 8×8 tiles directly.
The arcade graphics layout and transparent pen are documented in the
[MAME driver](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl.cpp)
and [video renderer](https://github.com/mamedev/mame/blob/master/src/mame/taito/bublbobl_v.cpp).
