"""Turn a captured sound command's OPL writes (capture_sound.py) into a stream
for the MEGA65's FM chip.

    python3 sound_stream.py 0f [out.bin]

The arcade's YM3526 runs at 3 MHz (41667 Hz inside); the MEGA65's FM chip
(as Xemu has it, an OPL3 at 49716 Hz, as an OPL2 at 3.58 MHz) at another
rate, so every F-number is scaled by CHIP_RATIO (the block stays), keeping
the pitches; envelopes run that much faster. Writes that leave a register
as it was are dropped.

The stream (little-endian):
- u16 the tick rate in hundredths of a hertz (the arcade's timer A);
- u16 the offset, from the stream's start, of the tick the music loops to;
- per tick: a count n (0-253) and n (register, value) pairs;
- or $FE, u16 offset, u16 count: play `count` ticks from that offset (ticks
  written out earlier, never another $FE), then go on after this;
- $FF: the end (loop, or stop if the music does not repeat).

One pass is kept: the intro if any, then the loop once.
"""

import json
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
GAME = HERE.parent
ROOT = GAME.parent.parent
SOUND = ROOT / "target" / "bubble-bobble" / "sound"
ARCADE_RATE = 3_000_000 / 72
MEGA65_RATE = 49716
CHIP_RATIO = ARCADE_RATE / MEGA65_RATE
END = 0xFF
REPEAT = 0xFE
MAX_COUNT = 253
# Ticks a repeat must cover to be worth its 5 bytes.
REPEAT_MIN = 8


def ticks_of(events):
    """The OPL writes grouped by timer A tick (writes before the first go in
    tick 0)."""
    ticks = [[]]
    started = False
    for ev in events:
        if ev.get("irq") == "a":
            if started:
                ticks.append([])
            started = True
        elif ev.get("chip") == "opl":
            ticks[-1].append((ev["reg"], ev["value"]))
    return ticks


def rescale(ticks):
    """The writes with F-numbers scaled for the MEGA65's chip: a write to a
    channel's $A0 or $B0 register writes both, from the scaled F-number."""
    low, high = [0] * 9, [0] * 9
    out = []
    for writes in ticks:
        tick = []
        for reg, value in writes:
            if 0xA0 <= reg <= 0xA8 or 0xB0 <= reg <= 0xB8:
                ch = reg & 0x0F
                if reg < 0xB0:
                    low[ch] = value
                else:
                    high[ch] = value
                fnum = round((((high[ch] & 3) << 8) | low[ch]) * CHIP_RATIO)
                tick.append((0xA0 + ch, fnum & 0xFF))
                tick.append((0xB0 + ch, (high[ch] & 0xFC) | (fnum >> 8)))
            else:
                tick.append((reg, value))
        out.append(tick)
    return out


def stream(cmd):
    lines = (SOUND / f"{cmd:02x}.jsonl").read_text().splitlines()
    header = json.loads(lines[0])
    ticks = rescale(ticks_of(json.loads(line) for line in lines[1:]))
    repeat = header["repeat"]
    first = repeat["from_tick"] if repeat else 0
    count = (repeat["from_tick"] + repeat["ticks"]) if repeat else len(ticks)
    # Each tick's bytes, writes that change nothing left out.
    encoded = []
    state = {}
    for writes in ticks[:count]:
        kept = []
        for reg, value in writes:
            if state.get(reg) != value:
                kept.append((reg, value))
                state[reg] = value
        tick = b""
        while len(kept) > MAX_COUNT:
            tick += bytes([MAX_COUNT]) + bytes(v for pair in kept[:MAX_COUNT] for v in pair)
            kept = kept[MAX_COUNT:]
        encoded.append(tick + bytes([len(kept)]) + bytes(v for pair in kept for v in pair))
    body = bytearray()
    # Where each tick written out literally went: repeats point only at runs
    # of these (and never across the loop point, so a repeat's ticks stay
    # in one pass of the music).
    literal = {}
    loop_at = None
    t = 0
    while t < count:
        if t == first:
            loop_at = len(body)
        best = (0, 0)
        for s0, at in literal.items():
            length = 0
            while (t + length < count and s0 + length in literal and s0 + length < t
                   and encoded[s0 + length] == encoded[t + length]
                   and literal[s0 + length] == at + sum(len(encoded[k]) for k in range(s0, s0 + length))
                   and not (t < first <= t + length)):
                length += 1
            if length > best[0]:
                best = (length, s0)
        if best[0] >= REPEAT_MIN:
            body += bytes([REPEAT]) + struct.pack("<HH", 4 + literal[best[1]], best[0])
            t += best[0]
        else:
            literal[t] = len(body)
            body += encoded[t]
            t += 1
    body.append(END)
    head = struct.pack("<HH", round(header["timer_a_hz"] * 100), 4 + (loop_at or 0))
    return head + body, header, count


def main():
    cmd = int(sys.argv[1], 16)
    data, header, count = stream(cmd)
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else SOUND / f"{cmd:02x}.opl"
    out.write_bytes(data)
    print(f"${cmd:02X}: {count} ticks at {header['timer_a_hz']} Hz, {len(data)} bytes -> {out}")


if __name__ == "__main__":
    main()
