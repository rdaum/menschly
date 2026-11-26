"""Generate the demo's data: the palette, tables and the runner's frames.

    python3 data.py [out dir]

Writes, in the out dir (default target/into-the-grid):
- grid-data.mensch, module GridData: the palette (DEMO_PALETTE, 256 RGB
  triples), a sine table, and where the runner's frames lie;
- grid-bank.bin: the blob for the ROM image's upper bank (inject_bank.py
  puts it at linear $30000): the runner's frames (figure.py).

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

import figure  # noqa: E402
import inject_bank  # noqa: E402

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
# The glass slab: standing on the floor from (X, z) NEAR to FAR, TALL high,
# THICK through; the runner runs across at depth RUNNER_Z.
PANE_NEAR = (-20.0, 13.0)
PANE_FAR = (30.0, 38.0)
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


def pane():
    """The glass slab projected: per screen column, the rows its front face
    and its near end (the slab's thickness) cover, and where along the front
    face the column is (0-255, for the sheen); and the column where the
    runner's path goes through its plane."""
    (nx, nz), (fx, fz) = PANE_NEAR, PANE_FAR
    # The front face's normal, towards the viewer's side: the end face is
    # the near edge pushed back through the glass by THICK.
    ax, az = fx - nx, fz - nz
    length = math.hypot(ax, az)
    bx, bz = -az / length * PANE_THICK, ax / length * PANE_THICK

    def edge(x, z):
        return project(x, 0, z), project(x, PANE_TALL, z)

    near_bottom, near_top = edge(nx, nz)
    far_bottom, far_top = edge(fx, fz)
    back_bottom, back_top = edge(nx + bx, nz + bz)
    columns = {}
    for x in range(320):
        cx = x + 0.5
        spans = []
        for (b0, t0), (b1, t1) in (((near_bottom, near_top), (far_bottom, far_top)),
                                   ((back_bottom, back_top), (near_bottom, near_top))):
            lo, hi = sorted((b0[0], b1[0]))
            if lo <= cx < hi and hi - lo > 0:
                f = (cx - b0[0]) / (b1[0] - b0[0])
                top = t0[1] + (t1[1] - t0[1]) * f
                bottom = b0[1] + (b1[1] - b0[1]) * f
                spans.append((max(0, round(top)), min(200, round(bottom)), f))
            else:
                spans.append(None)
        if any(spans):
            columns[x] = spans
    first, last = min(columns), max(columns)
    front_top, front_bottom, front_u, side_top, side_bottom = [], [], [], [], []
    for x in range(first, last + 1):
        front, side = columns.get(x, (None, None))
        front_top.append(front[0] if front else 0)
        front_bottom.append(front[1] if front else 0)
        # Along the face in depth, not on screen: the sheen recedes.
        if front:
            z = 1 / ((1 - front[2]) / nz + front[2] / fz)
            front_u.append(round((z - nz) / (fz - nz) * 255))
        else:
            front_u.append(0)
        side_top.append(side[0] if side else 0)
        side_bottom.append(side[1] if side else 0)
    # Where the runner's path (depth RUNNER_Z) meets the glass's plane.
    f = (RUNNER_Z - nz) / (fz - nz)
    cross = round(project(nx + ax * f, 0, RUNNER_Z)[0])
    return first, front_top, front_bottom, front_u, side_top, side_bottom, cross


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
    blob = frames
    assert len(blob) <= inject_bank.SPACE
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
    first, front_top, front_bottom, front_u, side_top, side_bottom, cross = pane()
    # The slab's end face down a column: light at its outer edge.
    side_strip = [SIDE + min(SIDE_STEPS - 1, 2 + (i * 5 // 7) % SIDE_STEPS // 2) for i in range(256)]
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
        array("SINE", "i8", sine),
        array("SHEEN_STRIP", "u8", sheen),
        array("SIDE_STRIP", "u8", side_strip),
        "# The glass slab, per screen column from PANE_FIRST: the rows its front",
        "# face covers (top, then bottom, exclusive; both 0 where it does not),",
        "# how far along the face in depth the column is (0-255), the rows its",
        "# near end covers; and the column where the runner goes through it.",
        f"export const PANE_FIRST = {first}",
        f"export const PANE_COLUMNS = {len(front_top)}",
        f"export const PANE_CROSS = {cross}",
        f"export const SIDE = {SIDE}",
        array("FRONT_TOP", "u8", front_top),
        array("FRONT_BOTTOM", "u8", front_bottom),
        array("FRONT_U", "u8", front_u),
        array("SIDE_TOP", "u8", side_top),
        array("SIDE_BOTTOM", "u8", side_bottom),
        "",
    ])
    (out_dir / "grid-data.mensch").write_text(text)
    print(f"grid data: palette, sine, {len(blob)} bytes for the ROM bank -> {out_dir}")


if __name__ == "__main__":
    main()
