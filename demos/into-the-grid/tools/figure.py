"""Render the demo's runner: a chrome android sprinter in an 8-frame run
cycle, for the MEGA65's full-colour screen.

    python3 figure.py [out dir]

The runner is a skeleton of capsules (head, torso, upper and lower arms
and legs) posed per frame from a run cycle, drawn far limbs first. Each
capsule is shaded as a chrome tube: its surface normal (from the distance
across it) picks a colour from an environment map, a bright sky above
the horizon and dark ground below, so the body reads as polished metal;
limbs on the far side use a darker ramp. The visor is a cyan slit.

Writes, in the out dir (default target/into-the-grid):
- runner.bin: the frames, each WIDTH x HEIGHT bytes of palette indices
  (0 transparent), stored column by column (a column is HEIGHT bytes),
  as the demo blits them;
- runner-palette.bin: RGB triples for entries 0-63;
- runner.png: the frames side by side, to look at.
"""

import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
WIDTH, HEIGHT = 64, 100
FRAMES = 8
SUPER = 4

# Palette: 0 transparent; NEAR and FAR chrome ramps (dark to bright);
# VISOR glow.
NEAR, NEAR_STEPS = 1, 32
FAR, FAR_STEPS = 33, 16
VISOR, VISOR_STEPS = 49, 4
COLOURS = 64


def env(t):
    """The chrome environment, t from 0 (straight down) to 1 (straight up):
    dark warm ground, a black horizon line, then sky to white."""
    stops = [(0.00, (16, 10, 14)), (0.28, (64, 44, 40)), (0.44, (150, 118, 96)),
             (0.48, (200, 176, 150)), (0.52, (10, 10, 22)), (0.60, (60, 78, 120)),
             (0.78, (150, 176, 214)), (0.92, (226, 234, 248)), (1.00, (255, 255, 255))]
    for (a, ca), (b, cb) in zip(stops, stops[1:]):
        if t <= b:
            f = (t - a) / (b - a) if b > a else 0
            return tuple(round(x + (y - x) * f) for x, y in zip(ca, cb))
    return stops[-1][1]


def palette():
    rgb = [(0, 0, 0)] * COLOURS
    for i in range(NEAR_STEPS):
        rgb[NEAR + i] = env(i / (NEAR_STEPS - 1))
    for i in range(FAR_STEPS):
        r, g, b = env(i / (FAR_STEPS - 1))
        rgb[FAR + i] = (r * 55 // 100, g * 55 // 100, b * 65 // 100)
    for i in range(VISOR_STEPS):
        f = (i + 1) / VISOR_STEPS
        rgb[VISOR + i] = (round(40 * f), round(220 * f + 20), round(255 * f))
    return rgb


def capsule(img, depth, a, b, r0, r1, ramp, steps, z):
    """Draws a capsule from a to b (radius r0 to r1) into img where it is
    nearer than depth, chrome-shaded on `ramp`."""
    h, w = img.shape
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    length2 = dx * dx + dy * dy or 1e-6
    ys, xs = np.mgrid[0:h, 0:w]
    px, py = xs + 0.5, ys + 0.5
    t = np.clip(((px - ax) * dx + (py - ay) * dy) / length2, 0, 1)
    cx, cy = ax + t * dx, ay + t * dy
    radius = r0 + (r1 - r0) * t
    ox, oy = px - cx, py - cy
    dist = np.sqrt(ox * ox + oy * oy)
    inside = (dist < radius) & (z > depth)
    # The surface normal: out from the axis, tilted towards the viewer.
    across = np.clip(dist / np.maximum(radius, 1e-6), 0, 1)
    nz = np.sqrt(1 - across ** 2)
    ny = np.where(dist > 0, -oy / np.maximum(dist, 1e-6), 0) * across
    # Reflect the view direction and look it up: up the env map as the
    # normal faces up, with a little of the view-facing part for a sheen.
    up = np.clip(0.5 + 0.5 * (2 * nz * ny) + 0.18 * (nz - 0.5), 0, 1)
    shade = np.clip(np.round(up * (steps - 1)), 0, steps - 1).astype(np.int16)
    img[inside] = ramp + shade[inside]
    depth[inside] = z


def pose(phase):
    """Joint positions (in frame pixels) for a run-cycle phase in [0, 1)."""
    s = math.sin(2 * math.pi * phase)
    c = math.cos(2 * math.pi * phase)
    bob = -2.5 * abs(c)
    lean = math.radians(14)
    hip = (30.0, 52.0 + bob)
    neck = (hip[0] + 22 * math.sin(lean), hip[1] - 22 * math.cos(lean))
    head = (neck[0] + 4.5, neck[1] - 8)

    def limb(root, angle, length):
        return (root[0] + length * math.sin(angle), root[1] + length * math.cos(angle))

    chest = (hip[0] + 15 * math.sin(lean), hip[1] - 15 * math.cos(lean))
    joints = {"hip": hip, "neck": neck, "head": head, "chest": chest,
              "pelvis": (hip[0] + 1.5, hip[1] - 2)}
    for side, sign in (("near", 1), ("far", -1)):
        thigh = 0.85 * s * sign + 0.15
        knee = limb(hip, thigh, 21)
        # The knee folds most as the leg swings forward.
        bend = 0.45 + 1.2 * max(0.0, -s * sign) + 0.3 * max(0.0, c * sign)
        joints[f"{side}_knee"] = knee
        joints[f"{side}_foot"] = limb(knee, thigh - bend, 21)
        shoulder = (neck[0] - 1, neck[1] + 3)
        upper = -0.9 * s * sign + 0.1
        elbow = limb(shoulder, upper, 14)
        joints[f"{side}_shoulder"] = shoulder
        joints[f"{side}_elbow"] = elbow
        joints[f"{side}_hand"] = limb(elbow, upper + 1.6, 13)
    return joints


def frame(phase):
    w, h = WIDTH * SUPER, HEIGHT * SUPER
    img = np.zeros((h, w), dtype=np.int16)
    depth = np.full((h, w), -1e9)
    j = {k: (x * SUPER, y * SUPER) for k, (x, y) in pose(phase).items()}
    S = SUPER

    def draw(a, b, r0, r1, near, z):
        ramp, steps = (NEAR, NEAR_STEPS) if near else (FAR, FAR_STEPS)
        capsule(img, depth, j[a], j[b], r0 * S, r1 * S, ramp, steps, z)

    draw("far_shoulder", "far_elbow", 3.2, 2.6, False, 0)
    draw("far_elbow", "far_hand", 2.6, 2.2, False, 1)
    draw("hip", "far_knee", 4.6, 3.6, False, 2)
    draw("far_knee", "far_foot", 3.6, 2.6, False, 3)
    draw("pelvis", "chest", 6.2, 8.8, True, 10)
    draw("chest", "neck", 8.8, 5.0, True, 10.5)
    draw("hip", "near_knee", 4.8, 3.8, True, 11)
    draw("near_knee", "near_foot", 3.8, 2.8, True, 12)
    draw("neck", "head", 5.6, 6.4, True, 13)
    draw("near_shoulder", "near_elbow", 3.4, 2.8, True, 14)
    draw("near_elbow", "near_hand", 2.8, 2.3, True, 15)
    # The visor: a glowing slit across the front of the head.
    hx, hy = j["head"]
    ys, xs = np.mgrid[0:h, 0:w]
    slit = (np.abs(ys + 0.5 - (hy - 0.5 * S)) < 1.1 * S) & (xs + 0.5 > hx - 0.5 * S) & (xs + 0.5 < hx + 6.4 * S) & (img > 0)
    glow = np.clip(((xs + 0.5 - hx) / (6.4 * S) * VISOR_STEPS), 0, VISOR_STEPS - 1).astype(np.int16)
    img[slit] = VISOR + glow[slit]
    # Down to frame size: a pixel takes its block's most common colour when
    # at least half the block is covered, else stays transparent.
    blocks = img.reshape(HEIGHT, SUPER, WIDTH, SUPER).transpose(0, 2, 1, 3).reshape(HEIGHT, WIDTH, -1)
    out = np.zeros((HEIGHT, WIDTH), dtype=np.uint8)
    for y in range(HEIGHT):
        for x in range(WIDTH):
            cell = blocks[y, x]
            covered = cell[cell > 0]
            if len(covered) * 2 >= len(cell):
                out[y, x] = np.bincount(covered).argmax()
    return out


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid"
    out_dir.mkdir(parents=True, exist_ok=True)
    frames = [frame(i / FRAMES) for i in range(FRAMES)]
    (out_dir / "runner.bin").write_bytes(b"".join(f.T.tobytes() for f in frames))
    rgb = palette()
    (out_dir / "runner-palette.bin").write_bytes(bytes(v for c in rgb for v in c))
    sheet = np.concatenate(frames, axis=1)
    lut = np.array(rgb, dtype=np.uint8)
    Image.fromarray(lut[sheet]).resize((sheet.shape[1] * 3, sheet.shape[0] * 3), Image.NEAREST).save(out_dir / "runner.png")
    print(f"runner: {FRAMES} frames of {WIDTH}x{HEIGHT} -> {out_dir}")


if __name__ == "__main__":
    main()
