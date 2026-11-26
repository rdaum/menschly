"""Generate the demo's data: the palette, tables and the runner's frames.

    python3 data.py [out dir]

Writes, in the out dir (default target/into-the-grid):
- grid-data.mensch, module GridData: the palettes (DEMO_PALETTE, 256 RGB
  triples, and the rotozoomer's ROTO_PALETTE), a sine table, the
  rotozoomer's texture, the glass slab's tables, and where the runner's
  frames lie;
- grid-bank.bin: the blob for the ROM image's upper bank (inject_bank.py
  puts it at linear $30000): the runner's frames (figure.py), then the
  music's stream (music.py, at MUSIC_AT).

The music is the demo's own tune (tune.py) arranged by music.DEFAULT, or
the MIDI file MUSIC_MIDI names in the environment (arranged by the JSON
file MUSIC_ARRANGEMENT names, if set).

The palette:
-   0- 63  the runner's chrome (figure.py);
-  64- 79  the data dots, dark teal to white;
-  80- 95  the floor's depth bands (the demo cycles them to move it);
-  96-103  the floor's lengthwise lines, near (bright) to far;
- 104-111  the glass slab's thickness (its end face), dark to light;
- 112-127  the glass's sheen, deep blue to pale cyan;
- 128-255  entry n - 128 seen through the glass: tinted cold and lighter.
"""

import math
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "tools"))

import json  # noqa: E402

import figure  # noqa: E402
import music  # noqa: E402
import tune  # noqa: E402
import inject_bank  # noqa: E402
import texture  # noqa: E402

DOTS, DOT_STEPS = 64, 16
FLOOR, FLOOR_STEPS = 80, 16
FLOOR_LINES, FLOOR_LINE_STEPS = 96, 8
SHEEN_STRIP = 512
SHEEN, SHEEN_STEPS = 112, 16
SIDE, SIDE_STEPS = 104, 8

# The camera, as the demo's floor has it: a floor point at depth z and
# across X shows at x = 160 + FOCAL * X / z, y = HORIZON + FOCAL * (EYE - Y) / z
# (Y up from the floor). FOCAL * EYE is the demo's DEPTH (1024).
HORIZON, FOCAL, EYE = 120, 32, 32
# The glass slab: standing on the floor, centred at (X, z) PANE_CENTRE,
# PANE_HALF long each way, turned PANE_REST (in 256ths of a turn) from
# facing the viewer, TALL high, THICK through; the runner runs across at
# depth RUNNER_Z. The demo projects it each frame (it turns and falls), in
# eighths of these units.
PANE_CENTRE = (4.0, 28.0)
PANE_HALF = 22.0
PANE_REST = 19
PANE_TALL = 72.0
PANE_THICK = 5.0
RUNNER_Z = 20.5
GLASS = 128
GLASS_TINT = (110, 170, 220)


def ramp(a, b, steps, curve=1.0):
    return [tuple(round(x + (y - x) * (i / (steps - 1)) ** curve) for x, y in zip(a, b))
            for i in range(steps)]


def demo_palette():
    rgb = list(figure.palette()) + [(0, 0, 0)] * (256 - figure.COLOURS)
    for i, c in enumerate(ramp((0, 40, 48), (40, 230, 220), 10) + ramp((80, 240, 230), (255, 255, 255), 6)):
        rgb[DOTS + i] = c
    for i in range(FLOOR_STEPS):
        rgb[FLOOR + i] = (14, 4, 30)
    for i, c in enumerate(ramp((230, 120, 255), (30, 10, 60), FLOOR_LINE_STEPS, 0.7)):
        rgb[FLOOR_LINES + i] = c
    for i, c in enumerate(ramp((10, 24, 60), (150, 220, 255), SHEEN_STEPS, 1.8)):
        rgb[SHEEN + i] = c
    for i, c in enumerate(ramp((20, 50, 70), (170, 240, 250), SIDE_STEPS, 1.4)):
        rgb[SIDE + i] = c
    for i in range(128):
        r, g, b = rgb[i]
        rgb[GLASS + i] = tuple(min(255, round(v * 0.62 + t * 0.38 + 10)) for v, t in zip((r, g, b), GLASS_TINT))
    return rgb


def project(x, y, z):
    return 160 + FOCAL * x / z, HORIZON + FOCAL * (EYE - y) / z


def pane_cross(sine):
    """The column where the runner's path (depth RUNNER_Z) meets the glass
    at rest, with the demo's own sine table."""
    cos, sin = sine[(PANE_REST + 64) & 255] / 127, sine[PANE_REST] / 127
    along = (RUNNER_Z - PANE_CENTRE[1]) / sin
    return round(project(PANE_CENTRE[0] + along * cos, 0, RUNNER_Z)[0])


def array(name, kind, values, per_line=16):
    lines = [f"export const {name}: [{len(values)}]{kind} = ["]
    for i in range(0, len(values), per_line):
        lines.append("    " + ", ".join(str(v) for v in values[i:i + per_line]) + ",")
    lines.append("]")
    return "\n".join(lines)


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid"
    out_dir.mkdir(parents=True, exist_ok=True)
    frames = b"".join(figure.frame(i / figure.FRAMES).T.tobytes() for i in range(figure.FRAMES))
    blob = bytearray(frames)
    midi_path = os.environ.get("MUSIC_MIDI")
    if not midi_path:
        midi_path = out_dir / "tune.mid"
        tune.write(midi_path, tune.tune(), tune.BPM)
    arrangement = music.DEFAULT
    if os.environ.get("MUSIC_ARRANGEMENT"):
        arrangement = json.loads(Path(os.environ["MUSIC_ARRANGEMENT"]).read_text())
    song, song_text, song_frames = music.build(midi_path, arrangement)
    music_at = inject_bank.BANK + len(blob)
    blob += song
    assert len(blob) <= inject_bank.SPACE, f"the bank is {len(blob)} bytes, over {inject_bank.SPACE}"
    (out_dir / "grid-bank.bin").write_bytes(blob)
    palette = demo_palette()
    sine = [round(127 * math.sin(2 * math.pi * i / 256)) for i in range(256)]
    # The glass's sheen down a column: soft bright bands on deep blue, 64
    # rows a period, so any 256 rows from any start look alike.
    sheen = []
    for i in range(SHEEN_STRIP):
        a = 0.5 + 0.5 * math.sin(2 * math.pi * i / 64)
        b = 0.5 + 0.5 * math.sin(2 * math.pi * i / 32 + 1.3)
        level = min(SHEEN_STEPS - 1, round((0.65 * a ** 3 + 0.35 * b ** 6) * (SHEEN_STEPS - 1)))
        sheen.append(SHEEN + level)
    cross = pane_cross(sine)
    # The slab's end face down a column: light bands, faint.
    side_strip = [SIDE + min(SIDE_STEPS - 1, 2 + (i * 5 // 7) % SIDE_STEPS // 2) for i in range(SHEEN_STRIP)]
    text = "\n".join([
        "module GridData",
        "",
        "# Generated by demos/into-the-grid/tools/data.py; see there.",
        "",
        "# The frame the demo starts at (START_FRAME in the environment, for",
        "# looking at a moment of it).",
        f"export const START_FRAME: u16 = {int(os.environ.get('START_FRAME', '0'))}",
        f"export const RUNNER_AT: u32 = ${inject_bank.BANK:x}",
        f"export const RUNNER_WIDTH = {figure.WIDTH}",
        f"export const RUNNER_HEIGHT = {figure.HEIGHT}",
        f"export const RUNNER_FRAMES = {figure.FRAMES}",
        f"export const RUNNER_FRAME_BYTES: u16 = {figure.WIDTH * figure.HEIGHT}",
        f"export const DOTS = {DOTS}",
        f"export const FLOOR = {FLOOR}",
        f"export const FLOOR_LINES = {FLOOR_LINES}",
        f"export const SHEEN = {SHEEN}",
        f"export const GLASS = {GLASS}",
        array("DEMO_PALETTE", "u8", [v for c in palette for v in c], 24),
        f"export const MUSIC_AT: u32 = ${music_at:x}",
        song_text,
        array("SINE", "i8", sine),
        "# The sine at four amplitudes, (k + 1) * 16 at most for row k: the dots'",
        "# wave, picked by how far past the glass they are, without a multiply.",
        "export const WOBBLE: [1024]i8 = [",
        *["    " + ", ".join(str(round(s * (k + 1) * 16 / 127)) for s in sine[i:i + 16]) + ","
          for k in range(4) for i in range(0, 256, 16)],
        "]",
        "# The rotozoomer's texture (texture.py): 128x128, row by row, and its",
        "# palette.",
        f"export const TEXTURE_SIZE = {texture.SIZE}",
        array("ROTO_PALETTE", "u8", [v for c in texture.palette() for v in c], 24),
        array("TEXTURE", "u8", [int(v) for v in texture.texture().flatten()], 32),
        array("SHEEN_STRIP", "u8", sheen),
        array("SIDE_STRIP", "u8", side_strip),
        "# The glass slab, in eighths of the floor's units (see data.py), and the",
        "# column where the runner goes through it at rest.",
        f"export const PANE_CX8 = {round(PANE_CENTRE[0] * 8)}",
        f"export const PANE_CZ8 = {round(PANE_CENTRE[1] * 8)}",
        f"export const PANE_HALF8 = {round(PANE_HALF * 8)}",
        f"export const PANE_TALL8 = {round(PANE_TALL * 8)}",
        f"export const PANE_THICK8 = {round(PANE_THICK * 8)}",
        f"export const EYE8 = {EYE * 8}",
        f"export const PANE_REST = {PANE_REST}",
        f"export const PANE_CROSS = {cross}",
        f"export const SIDE = {SIDE}",
        "",
    ])
    (out_dir / "grid-data.mensch").write_text(text)
    print(f"grid data: {len(blob)} bytes for the ROM bank, music {song_frames / music.FRAME_HZ:.1f} s "
          f"from {Path(midi_path).name} -> {out_dir}")


if __name__ == "__main__":
    main()
