"""Capture the arcade's sound commands as the chip register writes they make.

    python3 capture_sound.py            # every command, $00-$34
    python3 capture_sound.py 0f 10      # some

For each command MAME (headless, mame/sound.sh with reglog.lua) boots
bublboblr, parks the main CPU so it sends nothing of its own, queues the
command and logs every write to the YM3526 (OPL) and the YM2203 (OPN) until
3 s pass with no music writes (or SECONDS run out). Each command becomes
target/bubble-bobble/sound/<cmd>.jsonl:

- a header line: the command, its two interrupt rates (the YM2203's timers
  A and B, as the command left them: the sound CPU runs on both) and, for
  music that repeats, the timer A tick its repeat starts from and the
  repeat's length in timer A ticks;
- then one line per event in order: {"irq": "a"} or {"irq": "b"} where the
  sound CPU's interrupt for that timer began (its flag being cleared), and
  {"chip": "opl" | "opn", "reg": r, "value": v} for each write (the timer
  flag clears themselves are not listed).

The OPL writes can be replayed on an OPL2-compatible chip (the MEGA65's FM)
as they are, at the same interrupt rates; the YM2203's FM part needs its own
mapping. NOTES lists each command's notes as frequencies, as a check.
"""

import json
import math
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
GAME = HERE.parent
ROOT = GAME.parent.parent
OUT = ROOT / "target" / "bubble-bobble" / "sound"
CLOCK = 3_000_000
# Ticks of identical key-ons that make a repeat.
REPEAT_RUN = 600
SECONDS = int(os.environ.get("SECONDS_PER_COMMAND", "50"))
AT = 150


def capture(cmd):
    log = OUT / f"{cmd:02x}.log"
    env = dict(os.environ, CMD=f"{cmd:02X}", AT=str(AT), FRAMES=str(AT + int(SECONDS * 59)), OUT=str(log))
    subprocess.run([str(GAME / "mame" / "sound.sh"), str(SECONDS + 4)], env=env, check=True)
    return log.read_text().split("\n")


def events(lines):
    """The command's events from its send on, and the timer values it set."""
    out, timers, started = [], {}, False
    for line in lines:
        cols = line.split()
        if not cols:
            continue
        if cols[1] == "send":
            started = True
            continue
        if not started or cols[1] == "queue":
            continue
        chip, reg, value = cols[1], int(cols[2], 16), int(cols[3], 16)
        if chip == "opn" and reg == 0x27:
            # The interrupt handler clears timer A's flag ($1F) or B's ($2F).
            if value & 0x10 and not value & 0x20:
                out.append({"irq": "a"})
            elif value & 0x20 and not value & 0x10:
                out.append({"irq": "b"})
            continue
        if chip == "opn" and reg in (0x24, 0x25, 0x26):
            timers[reg] = value
        out.append({"chip": chip, "reg": reg, "value": value})
    return out, timers


def rates(timers):
    ta = (timers.get(0x24, 0) << 2) | (timers.get(0x25, 0) & 3)
    tb = timers.get(0x26, 0)
    return CLOCK / (72 * (1024 - ta)), CLOCK / (1152 * (256 - tb))


def notes(evs):
    """Key-ons as (timer A tick, chip, channel, Hz)."""
    tick, found = -1, []
    flow, on = [0] * 9, [0] * 9
    opn_f = {}
    for ev in evs:
        if ev.get("irq") == "a":
            tick += 1
            continue
        if "chip" not in ev:
            continue
        reg, value = ev["reg"], ev["value"]
        if ev["chip"] == "opl":
            if 0xA0 <= reg <= 0xA8:
                flow[reg - 0xA0] = value
            elif 0xB0 <= reg <= 0xB8:
                ch, key = reg - 0xB0, (value >> 5) & 1
                if key and not on[ch]:
                    fnum, block = ((value & 3) << 8) | flow[ch], (value >> 2) & 7
                    found.append((tick, "opl", ch, fnum * CLOCK / (72 * 2 ** (20 - block))))
                on[ch] = key
        else:
            if 0xA0 <= reg <= 0xA2:
                opn_f[reg - 0xA0] = (opn_f.get(reg - 0xA0, 0) & 0x3F00) | value
            elif 0xA4 <= reg <= 0xA6:
                opn_f[reg - 0xA4] = ((value & 0x3F) << 8) | (opn_f.get(reg - 0xA4, 0) & 0xFF)
            elif reg == 0x28 and value & 0xF0:
                ch = value & 3
                word = opn_f.get(ch, 0)
                fnum, block = word & 0x7FF, (word >> 11) & 7
                # YM2203 FM: prescaler 6, f = fnum * clock / (144 * 2^(21 - block)).
                found.append((tick, "opn", ch, fnum * CLOCK / (144 * 2 ** (21 - block))))
    return found


def repeat(found, ticks):
    """(start tick, length) of the music's loop: the shortest length such
    that, from some tick on, every tick's key-ons are those `length` ticks
    later right to the end of the capture, for at least REPEAT_RUN ticks
    (a section repeated within the music does not reach the end); else
    None. An intro before the loop is allowed."""
    import numpy as np
    sig = np.zeros(ticks + 1, dtype=np.int64)
    by_tick = {}
    for tick, chip, ch, hz in found:
        by_tick.setdefault(tick, []).append((chip, ch, round(hz, 1)))
    for tick, keys in by_tick.items():
        if 0 <= tick <= ticks:
            sig[tick] = hash(tuple(sorted(keys))) or 1
    # The capture's last few ticks may be cut short.
    last = ticks - 2
    for length in range(60, last - REPEAT_RUN):
        same = sig[:last + 1 - length] == sig[length:last + 1]
        bad = np.flatnonzero(~same)
        start = int(bad[-1]) + 1 if len(bad) else 0
        if len(same) - start >= REPEAT_RUN and sig[start:start + length].any():
            return start, length
    return None


def midi(hz):
    return round(69 + 12 * math.log2(hz / 440), 1) if hz > 0 else None


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cmds = [int(a, 16) for a in sys.argv[1:]] or list(range(0x35))
    summary = []
    for cmd in cmds:
        evs, timers = events(capture(cmd))
        rate_a, rate_b = rates(timers)
        ticks = sum(1 for ev in evs if ev.get("irq") == "a")
        found = notes(evs)
        loop = repeat(found, ticks)
        header = {"command": cmd, "timer_a_hz": round(rate_a, 3), "timer_b_hz": round(rate_b, 3),
                  "ticks": ticks, "writes": sum(1 for ev in evs if "chip" in ev),
                  "repeat": {"from_tick": loop[0], "ticks": loop[1]} if loop else None}
        with open(OUT / f"{cmd:02x}.jsonl", "w") as f:
            f.write(json.dumps(header) + "\n")
            for ev in evs:
                f.write(json.dumps(ev) + "\n")
        (OUT / f"{cmd:02x}.log").unlink()
        summary.append(f"${cmd:02X}: {ticks} ticks at {rate_a:.2f} Hz, {header['writes']} writes, "
                       f"{len(found)} notes" + (f", repeats every {loop[1]} ticks ({loop[1] / rate_a:.2f} s)"
                                                if loop else ""))
        print(summary[-1], flush=True)
        notes_out = OUT / f"{cmd:02x}.notes"
        notes_out.write_text("".join(f"{t}\t{chip}\t{ch}\t{hz:.1f}\t{midi(hz)}\n" for t, chip, ch, hz in found))


if __name__ == "__main__":
    main()
