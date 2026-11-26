"""Read and write standard MIDI files, just the parts the demo's music uses:
notes and tempo.

    notes, tempo_map = read("song.mid")
    write("song.mid", notes, bpm=125)

A note is a Note(start, end, channel, pitch, velocity), times in seconds
(read follows the file's tempo changes), channel 0-15 (9 is drums).
"""

import struct
from collections import namedtuple

Note = namedtuple("Note", "start end channel pitch velocity")

DIVISION = 480


def _varlen(data, i):
    value = 0
    while True:
        byte = data[i]
        i += 1
        value = (value << 7) | (byte & 0x7F)
        if not byte & 0x80:
            return value, i


def _ticks_to_seconds(tempos, division):
    """A function from a tick to seconds, given (tick, microseconds per beat)
    tempo changes."""
    tempos = sorted(tempos) or [(0, 500000)]
    if tempos[0][0] != 0:
        tempos.insert(0, (0, 500000))
    marks = []
    seconds = 0.0
    for i, (tick, us) in enumerate(tempos):
        if i:
            prev_tick, prev_us = tempos[i - 1]
            seconds += (tick - prev_tick) * prev_us / 1e6 / division
        marks.append((tick, seconds, us))

    def convert(tick):
        for mark_tick, mark_seconds, us in reversed(marks):
            if tick >= mark_tick:
                return mark_seconds + (tick - mark_tick) * us / 1e6 / division
        return 0.0
    return convert


def read(path):
    """The file's notes (sorted by start) and its tempo changes."""
    data = open(path, "rb").read()
    if data[:4] != b"MThd":
        raise ValueError(f"{path}: not a standard MIDI file")
    _, tracks, division = struct.unpack(">HHH", data[8:14])
    if division & 0x8000:
        raise ValueError(f"{path}: SMPTE time division is not supported")
    pos = 8 + struct.unpack(">I", data[4:8])[0]
    raw = []
    tempos = []
    for _ in range(tracks):
        if data[pos:pos + 4] != b"MTrk":
            raise ValueError(f"{path}: bad track header")
        length = struct.unpack(">I", data[pos + 4:pos + 8])[0]
        track = data[pos + 8:pos + 8 + length]
        pos += 8 + length
        i = tick = status = 0
        sounding = {}
        while i < len(track):
            delta, i = _varlen(track, i)
            tick += delta
            byte = track[i]
            if byte == 0xFF:
                kind = track[i + 1]
                length, j = _varlen(track, i + 2)
                if kind == 0x51:
                    tempos.append((tick, int.from_bytes(track[j:j + length], "big")))
                i = j + length
                continue
            if byte in (0xF0, 0xF7):
                length, j = _varlen(track, i + 1)
                i = j + length
                continue
            if byte & 0x80:
                status = byte
                i += 1
            kind, channel = status & 0xF0, status & 0x0F
            if kind in (0xC0, 0xD0):
                i += 1
                continue
            a, b = track[i], track[i + 1]
            i += 2
            if kind == 0x90 and b:
                sounding.setdefault((channel, a), []).append((tick, b))
            elif kind in (0x80, 0x90) and sounding.get((channel, a)):
                start, velocity = sounding[(channel, a)].pop(0)
                raw.append((start, tick, channel, a, velocity))
    seconds = _ticks_to_seconds(tempos, division)
    notes = sorted(Note(seconds(s), seconds(e), c, p, v) for s, e, c, p, v in raw)
    return notes, tempos


def _event(delta, payload):
    out = bytearray()
    value = delta
    stack = [value & 0x7F]
    value >>= 7
    while value:
        stack.append((value & 0x7F) | 0x80)
        value >>= 7
    out += bytes(reversed(stack))
    return bytes(out) + bytes(payload)


def write(path, notes, bpm=120):
    """Writes `notes` (times in beats, not seconds) as a one-track file at
    `bpm`."""
    events = []
    for n in notes:
        on = round(n.start * DIVISION)
        off = max(on + 1, round(n.end * DIVISION))
        events.append((on, 1, bytes([0x90 | n.channel, n.pitch, n.velocity])))
        events.append((off, 0, bytes([0x80 | n.channel, n.pitch, 0])))
    events.sort(key=lambda e: (e[0], e[1]))
    body = bytearray(_event(0, b"\xff\x51\x03" + round(60e6 / bpm).to_bytes(3, "big")))
    tick = 0
    for at, _, payload in events:
        body += _event(at - tick, payload)
        tick = at
    body += _event(0, b"\xff\x2f\x00")
    with open(path, "wb") as f:
        f.write(b"MThd" + struct.pack(">IHHH", 6, 0, 1, DIVISION))
        f.write(b"MTrk" + struct.pack(">I", len(body)) + body)
