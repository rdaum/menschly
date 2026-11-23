"""Play the same inputs in the arcade (MAME) and in our game (Xemu), both
headless, and compare Bub and the enemies frame by frame.

    python3 replay.py "RIGHT 120-128, UP 140, FIRE 300" --frames 600

A script lists buttons (UP = jump, FIRE = blow, LEFT, RIGHT) held over
frame ranges counted from the start of round 1 (a single frame holds the
button for JUMP_HOLD frames). It prints, per object, the first frame the two
disagree and how many frames do, with the frames around the first.

The arcade's positions are converted to ours: its x is a body's centre and
its height counts up from the bottom (modulo 256); ours are the top left of
the 16x16 body in room pixels.
"""

import argparse
import os
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
ASSETS = ROOT / "target" / "bubble-bobble"
WORK = ASSETS / "replay"
MAME_DIR = ASSETS / "re" / "mame"
XEMU = os.environ.get("MENSCH_XMEGA65", str(Path.home() / "src/xemu/build/bin/xmega65.native"))

# Our joystick bits (lib/vic4.mensch) and the arcade's input fields.
BUTTONS = {
    "UP": (0x01, "P1 Button 2"),
    "LEFT": (0x04, "P1 Left"),
    "RIGHT": (0x08, "P1 Right"),
    "FIRE": (0x10, "P1 Button 1"),
}
JUMP_HOLD = 5
# The arcade frame (MAME's count, after coin, start and normal mode) that
# matches our frame 0: the first enemy appears on both.
ARCADE_START = 2380
# The arcade acts on input a frame later than our game (its MCU reads the
# controls, the main CPU uses them the next frame), so its inputs go in
# this many frames early (one of them for MAME applying input at the end of
# a frame).
INPUT_LEAD = 2
ENEMIES = 3
LOG = 0x58000
LOG_BYTES = 32


def parse(script, frames):
    inputs = [0] * frames
    for part in filter(None, (p.strip() for p in script.split(","))):
        name, _, span = part.partition(" ")
        bit = BUTTONS[name.upper()][0]
        first, _, last = span.partition("-")
        first = int(first)
        last = int(last) if last else first + JUMP_HOLD - 1
        for t in range(first, min(last, frames - 1) + 1):
            inputs[t] |= bit
    return inputs


def run_ours(inputs):
    WORK.mkdir(parents=True, exist_ok=True)
    module = WORK / "game-replay.mensch"
    rows = ",\n    ".join(", ".join(f"${v:02x}" for v in inputs[i:i + 16])
                          for i in range(0, len(inputs), 16))
    module.write_text(f"module GameReplay\nexport const REPLAY = 1\n"
                      f"export const REPLAY_FRAMES = {len(inputs)}\n"
                      f"export const REPLAY_INPUT: [{len(inputs)}]u8 = [\n    {rows}\n]\n")
    rom = WORK / "game.rom"
    env = dict(os.environ, REPLAY_MODULE=str(module), GAME_ROM=str(rom))
    subprocess.run([str(HERE / "run-game.sh"), "--no-run"], env=env, check=True,
                   stdout=subprocess.DEVNULL)
    mem = WORK / "mem.bin"
    mem.unlink(missing_ok=True)
    (WORK / "sd").mkdir(exist_ok=True)
    # Xemu runs in real time at 50 frames a second; give it boot time too.
    seconds = len(inputs) / 50 + 4
    subprocess.run(["timeout", str(int(seconds)), XEMU, "-headless", "-besure", "-nosound",
                    "-virtsd", "-sdimg", str(WORK / "sd"), "-rom", str(rom),
                    "-dumpmem", str(mem)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    data = mem.read_bytes()
    frames = []
    for t in range(len(inputs)):
        rec = data[LOG + t * LOG_BYTES:][:LOG_BYTES]
        bx, by, bstate = struct.unpack_from("<hhB", rec, 0)
        enemies = []
        for i in range(ENEMIES):
            ex, ey, state, motion = struct.unpack_from("<hhBB", rec, 5 + i * 6)
            # Waiting or gone enemies are not on screen.
            enemies.append(visible((ex, ey)) if state == 1 else None)
        frames.append(((bx, by, bstate), enemies))
    return frames


def run_arcade(inputs):
    events = []
    held = 0
    for t, bits in enumerate(inputs + [0]):
        for name, (bit, field) in BUTTONS.items():
            if (bits ^ held) & bit:
                events.append(f"{ARCADE_START + t - INPUT_LEAD}:{field}:{1 if bits & bit else 0}")
        held = bits
    out = WORK / "arcade.tsv"
    env = dict(os.environ, OUT=str(out), FIRST=str(ARCADE_START), LAST=str(ARCADE_START + len(inputs)),
               RANGES="E690-E693,ED49-ED93", INPUTS=",".join(events),
               SNAPDIR=str(WORK / "snap"))
    seconds = (ARCADE_START + len(inputs)) // 59 + 3
    subprocess.run([str(MAME_DIR / "probe.sh"), str(seconds)], env=env, check=True)
    frames = []
    for line in out.read_text().splitlines():
        cols = line.split("\t")
        bub = bytes.fromhex(cols[1])
        rec = bytes.fromhex(cols[2])
        state, h, x = bub[1], bub[2], bub[3]
        enemies = []
        for i in range(ENEMIES):
            r = rec[i * 0x19:(i + 1) * 0x19]
            enemies.append(visible(convert(r[2], r[1])) if r[0] else None)
        frames.append(((*convert(x, h), state), enemies))
    return frames


def convert(x, h):
    y = 216 - h
    if y > 208:
        y -= 256
    return (x - 8, y)


def visible(position):
    """A position, or None when entirely outside the room."""
    if position is None or position[1] >= 208 or position[1] <= -16:
        return None
    return position


def compare(ours, arcade, context):
    names = ["Bub"] + [f"enemy {i}" for i in range(ENEMIES)]
    count = min(len(ours), len(arcade))
    worst = 0
    for k, name in enumerate(names):
        def at(frames, t):
            return frames[t][0] if k == 0 else frames[t][1][k - 1]
        # The arcade's Bub is state 0 (not there) for a frame on coming back.
        bad = [t for t in range(count) if at(ours, t) != at(arcade, t)
               and not (k == 0 and at(arcade, t)[2] == 0)]
        if not bad:
            print(f"{name}: matches for all {count} frames")
            continue
        worst = max(worst, len(bad))
        first = bad[0]
        print(f"{name}: first differs at frame {first}, {len(bad)} of {count} frames differ")
        for t in range(max(0, first - 2), min(count, first + context)):
            print(f"    {t:4d}  ours {at(ours, t)!s:18}  arcade {at(arcade, t)}")
    return worst


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("script", nargs="?", default="")
    parser.add_argument("--frames", type=int, default=600)
    parser.add_argument("--context", type=int, default=6)
    args = parser.parse_args()
    if args.frames > 1024:
        sys.exit("at most 1024 frames (the game's replay log)")
    inputs = parse(args.script, args.frames)
    arcade = run_arcade(inputs)
    ours = run_ours(inputs)
    compare(ours, arcade, args.context)


if __name__ == "__main__":
    main()
