"""Play an input script in the arcade and show what happens in the game when
each sound command is sent, to tell which command is which effect.

    python3 sound_events.py "FIRE 150-151, RIGHT 310-330" --frames 4000

For every command the main CPU sends the sound CPU (mame/soundcmds.lua),
prints its frame (from round 1's start, as replay.py counts) and the game's
changes over the frames just before it: Bub's state, bubbles' and enemies'
states, the score and the lives.
"""

import argparse
import os
import subprocess

import replay

LIVES = 0xE659


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("script", nargs="?", default="")
    parser.add_argument("--frames", type=int, default=3000)
    parser.add_argument("--before", type=int, default=3, help="frames of changes shown")
    args = parser.parse_args()
    inputs = replay.parse(args.script, args.frames)
    events, held = [], 0
    for t, bits in enumerate(inputs + [0]):
        for name, (bit, field) in replay.BUTTONS.items():
            if (bits ^ held) & bit:
                lead = replay.FIRE_LEAD if name == "FIRE" else replay.INPUT_LEAD
                events.append(f"{replay.ARCADE_START + t - lead}:{field}:{1 if bits & bit else 0}")
        held = bits
    replay.WORK.mkdir(parents=True, exist_ok=True)
    out = replay.WORK / "sound-events.tsv"
    base, stride, count = replay.BUBBLE_BASE, replay.BUBBLE_STRIDE, replay.ARCADE_BUBBLES
    env = dict(os.environ, OUT=str(out), FIRST="0", LAST=str(replay.ARCADE_START + args.frames),
               RANGES=f"E690-E693,ED49-ED93,{base:04X}-{base + stride * count - 1:04X},E641-E643,"
                      f"{LIVES:04X}-{LIVES:04X}",
               INPUTS=",".join(events), PROBE_SCRIPT="soundcmds.lua")
    seconds = (replay.ARCADE_START + args.frames) // 59 + 3
    subprocess.run([str(replay.MAME_DIR / "probe.sh"), str(seconds)], env=env, check=True)

    states, sounds = {}, []
    for line in out.read_text().splitlines():
        cols = line.split()
        if cols[0] == "S":
            sounds.append((int(cols[1]), cols[2]))
            continue
        bub = bytes.fromhex(cols[1])
        enemies = bytes.fromhex(cols[2])
        table = bytes.fromhex(cols[3])
        states[int(cols[0])] = {
            "bub": f"state {bub[1]} h {bub[2]}",
            **{f"enemy {i}": f"{enemies[i * 0x19]:02x}/{enemies[i * 0x19 + 0x0E]:02x}"
               for i in range(replay.ENEMIES)},
            **{f"bubble {i}": f"{table[i * stride]:02x}" for i in range(count)},
            "score": int(bytes.fromhex(cols[4])[::-1].hex()) * 10,
            "lives": bytes.fromhex(cols[5])[0],
        }
    for frame, cmd in sounds:
        changes = []
        for t in range(frame - args.before, frame + 1):
            now, then = states.get(t), states.get(t - 1)
            if not now or not then:
                continue
            for key, value in now.items():
                if key != "bub" or value.split()[1] != then[key].split()[1]:
                    if value != then[key]:
                        changes.append(f"{t - replay.ARCADE_START}: {key} {then[key]} -> {value}")
        print(f"${cmd.upper()} at {frame - replay.ARCADE_START}: " + ("; ".join(changes) or "-"))


if __name__ == "__main__":
    main()
