"""Turn a MIDI file into music for the demo's player (src/music.mensch): an
event stream for the FM chip's 9 channels and the SIDs' 12 voices, with
the instruments and note tables it needs.

    python3 music.py [song.mid [arrangement.json]]

An arrangement says how the MIDI's notes become sounds:
- "instruments": name -> an FM instrument {"fm": [11 register values]} (the
  modulator's $20, $40, $60, $80 and $E0 registers, the carrier's, then
  the channel's $C0) or a SID one {"sid": [waveform, attack/decay,
  sustain/release, pulse width]};
- "parts": each takes the notes of some MIDI channels (0-15) within a pitch
  range, shifted by "transpose", and plays each on one voice from each of
  its "layers" ({"voices": [...], "instrument": name}). Voices 0-8 are FM
  channels, 9-20 SID voices (SID n // 3, voice n % 3, of the 9 on). A
  layer takes a free voice from its list, or else the one that has played
  longest;
- "drums": MIDI pitch (on channel 9) -> {"voice", "instrument", "pitch"}.

DEFAULT is the arrangement for the demo's own tune (tune.py).

Times become 50 Hz frames. The stream (offsets from its start):
- u16 where to loop back to;
- per frame with events: $80 voice note instrument (a note starts), $81
  voice (it stops), ..., then a byte 1-127, the frames to wait for the
  next (a "group": events and a wait);
- or $82, u16 offset, u16 count: play `count` groups from that offset
  (written out earlier, never another $82), then go on after this;
- $FF ends it: the player goes back to the loop point (a song can loop
  past an intro).
"""

import json
import sys
from pathlib import Path

import midi

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FRAME_HZ = 50
FM_CLOCK = 49716
SID_CLOCK = 1_000_000
FM_VOICES = 9
SID_VOICES = 12
NOTE_ON, NOTE_OFF, REPEAT, END = 0x80, 0x81, 0x82, 0xFF
MAX_WAIT = 127
# Groups a repeat must cover to be worth its 5 bytes.
REPEAT_MIN = 3

# FM registers: modulator $20 $40 $60 $80 $E0, carrier the same, $C0.
DEFAULT = {
    "instruments": {
        "fm_bass": {"fm": [0x21, 0x1A, 0xF4, 0x26, 0x00, 0x21, 0x00, 0xF3, 0x47, 0x00, 0x0C]},
        "fm_stab": {"fm": [0x01, 0x16, 0xF2, 0x35, 0x00, 0x01, 0x04, 0xF5, 0x66, 0x00, 0x0A]},
        "fm_lead": {"fm": [0x22, 0x20, 0xE1, 0x16, 0x01, 0x21, 0x02, 0xD2, 0x25, 0x00, 0x08]},
        "fm_kick": {"fm": [0x00, 0x0B, 0xF8, 0xF8, 0x00, 0x00, 0x00, 0xF7, 0xF7, 0x00, 0x0E]},
        "fm_arp": {"fm": [0x02, 0x1C, 0xF6, 0x58, 0x00, 0x01, 0x06, 0xF5, 0x58, 0x00, 0x06]},
        "sid_arp": {"sid": [0x40, 0x04, 0x00, 0x600]},
        "sid_bass": {"sid": [0x20, 0x08, 0x00, 0x800]},
        "sid_stab": {"sid": [0x40, 0x05, 0x00, 0x300]},
        "sid_lead": {"sid": [0x20, 0x26, 0x96, 0x800]},
        "sid_snare": {"sid": [0x80, 0x07, 0x00, 0x800]},
        "sid_hat": {"sid": [0x80, 0x03, 0x00, 0x800]},
        "sid_crash": {"sid": [0x80, 0x0B, 0x00, 0x800]},
    },
    "parts": [
        {"name": "bass", "channels": [0], "layers": [
            {"voices": [0], "instrument": "fm_bass"},
            {"voices": [9], "instrument": "sid_bass"}]},
        {"name": "stabs", "channels": [1], "layers": [
            {"voices": [1, 2, 3], "instrument": "fm_stab"},
            {"voices": [10, 11, 12], "instrument": "sid_stab"}]},
        {"name": "arp", "channels": [3], "layers": [
            {"voices": [6, 7], "instrument": "fm_arp"},
            {"voices": [14, 18], "instrument": "sid_arp"}]},
        {"name": "lead", "channels": [2], "layers": [
            {"voices": [4], "instrument": "fm_lead"},
            {"voices": [13], "instrument": "sid_lead"}]},
    ],
    "drums": {
        "36": {"voice": 5, "instrument": "fm_kick", "pitch": 33},
        "38": {"voice": 15, "instrument": "sid_snare", "pitch": 84},
        "42": {"voice": 16, "instrument": "sid_hat", "pitch": 108},
        "49": {"voice": 17, "instrument": "sid_crash", "pitch": 96},
    },
}


def frequency(note):
    return 440 * 2 ** ((note - 69) / 12)


def fm_note(note):
    """The FM chip's $A0 and $B0 values (no key-on) for MIDI note `note`."""
    f = frequency(note)
    for block in range(8):
        fnum = round(f * (1 << (20 - block)) / FM_CLOCK)
        if fnum < 1024:
            return fnum & 0xFF, (block << 2) | (fnum >> 8)
    return 0xFF, (7 << 2) | 3


def sid_note(note):
    return min(0xFFFF, round(frequency(note) * (1 << 24) / SID_CLOCK))


def schedule(notes, arrangement):
    """(frame, kind, voice, note, instrument) events: the notes on voices."""
    names = list(arrangement["instruments"])
    events = []
    busy = {}          # voice -> frame it is free from
    started = {}       # voice -> frame its note started

    def take(pool, on, off):
        free = [v for v in pool if busy.get(v, 0) <= on]
        voice = min(free or pool, key=lambda v: started.get(v, -1))
        busy[voice] = off
        started[voice] = on
        return voice

    drums = {int(k): v for k, v in arrangement.get("drums", {}).items()}
    # Each note sounding on a voice gets an id, so a stop can tell whether
    # its note still has the voice.
    ident = 0
    for n in notes:
        on = round(n.start * FRAME_HZ)
        off = max(on + 1, round(n.end * FRAME_HZ))
        if n.channel == 9:
            drum = drums.get(n.pitch)
            if drum:
                voice = drum["voice"]
                busy[voice], started[voice] = off, on
                ident += 1
                events.append((on, 1, voice, drum["pitch"], names.index(drum["instrument"]), ident))
                events.append((off, 0, voice, 0, 0, ident))
            continue
        for part in arrangement["parts"]:
            if n.channel not in part["channels"]:
                continue
            if not part.get("low", 0) <= n.pitch <= part.get("high", 127):
                continue
            pitch = max(0, min(127, n.pitch + part.get("transpose", 0)))
            for layer in part["layers"]:
                voice = take(layer["voices"], on, off)
                ident += 1
                events.append((on, 1, voice, pitch, names.index(layer["instrument"]), ident))
                events.append((off, 0, voice, 0, 0, ident))
            break
    # In time order, stops before starts in a frame; a stop whose note has
    # lost its voice to a later one is dropped.
    events.sort(key=lambda e: (e[0], e[1]))
    owner = {}
    kept = []
    for frame, kind, voice, note, instrument, ident in events:
        if kind:
            owner[voice] = ident
        elif owner.get(voice) != ident:
            continue
        kept.append((frame, kind, voice, note, instrument))
    return kept


def groups(events, length, loop):
    """The stream's groups, each a frame's events and then one wait byte,
    with one starting at frame `loop`; and that group's index."""
    out = []
    loop_group = None
    frame = 0
    i = 0
    pending = b""
    stops = sorted({e[0] for e in events} | {loop, length})
    for stop in stops:
        if stop > length:
            break
        while i < len(events) and events[i][0] == frame:
            _, kind, voice, note, instrument = events[i]
            pending += bytes([NOTE_ON, voice, note, instrument]) if kind else bytes([NOTE_OFF, voice])
            i += 1
        while stop - frame > 0:
            step = min(MAX_WAIT, stop - frame)
            if frame == loop:
                loop_group = len(out)
            out.append(pending + bytes([step]))
            pending = b""
            frame += step
    if loop_group is None:
        loop_group = 0
    return out, loop_group


def stream(events, length, loop=0):
    """The stream: earlier runs of groups played again where they repeat
    (never across the loop point)."""
    parts, loop_group = groups(events, length, loop)
    body = bytearray()
    literal = {}
    loop_at = 0
    g = 0
    while g < len(parts):
        if g == loop_group:
            loop_at = len(body)
        best = (0, 0)
        for start in literal:
            run = 0
            while (g + run < len(parts) and start + run in literal and start + run < g
                   and parts[start + run] == parts[g + run]
                   and literal[start + run] == literal[start] + sum(len(parts[k]) for k in range(start, start + run))
                   and not (g < loop_group <= g + run)):
                run += 1
            if run > best[0]:
                best = (run, start)
        if best[0] >= REPEAT_MIN and 5 < sum(len(parts[k]) for k in range(g, g + best[0])):
            body += bytes([REPEAT]) + (2 + literal[best[1]]).to_bytes(2, "little") + best[0].to_bytes(2, "little")
            g += best[0]
        else:
            literal[g] = len(body)
            body += parts[g]
            g += 1
    body.append(END)
    return (2 + loop_at).to_bytes(2, "little") + bytes(body)


def instrument_table(arrangement):
    table = []
    for spec in arrangement["instruments"].values():
        if "fm" in spec:
            table += [0] + list(spec["fm"])
        else:
            wave, ad, sr, width = spec["sid"]
            table += [1, wave, ad, sr, width & 0xFF, width >> 8] + [0] * 6
    return table


def build(midi_path, arrangement=DEFAULT, loop_seconds=0.0, length_seconds=None):
    """The stream (looping back to `loop_seconds`; as long as
    `length_seconds`, or to the last note), and Mensch source describing
    it (MUSIC_AT is filled in by the caller)."""
    notes, _ = midi.read(midi_path)
    events = schedule(notes, arrangement)
    length = max(e[0] for e in events) + 1
    if length_seconds:
        length = round(length_seconds * FRAME_HZ)
    data = stream(events, length, round(loop_seconds * FRAME_HZ))
    fm = [fm_note(n) for n in range(128)]
    sid = [sid_note(n) for n in range(128)]

    def array(name, values):
        return f"export const {name}: [{len(values)}]u8 = [" + ", ".join(str(v) for v in values) + "]"

    lines = [
        "# The music (tools/music.py, from " + Path(midi_path).name + "): its instruments,",
        "# 12 bytes each (0 then the FM registers, or 1 then a SID's waveform,",
        "# attack/decay, sustain/release and pulse width), and per MIDI note the",
        "# FM chip's $A0/$B0 values and a SID's frequency.",
        f"export const INSTRUMENTS = {len(arrangement['instruments'])}",
        array("INSTRUMENT", instrument_table(arrangement)),
        array("FM_LOW", [lo for lo, _ in fm]),
        array("FM_HIGH", [hi for _, hi in fm]),
        array("SID_LOW", [f & 0xFF for f in sid]),
        array("SID_HIGH", [f >> 8 for f in sid]),
    ]
    return data, "\n".join(lines), length


def main():
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid" / "tune.mid"
    arrangement = json.loads(Path(sys.argv[2]).read_text()) if len(sys.argv) > 2 else DEFAULT
    data, _, frames = build(path, arrangement)
    print(f"{path.name}: {frames} frames ({frames / FRAME_HZ:.1f} s), {len(data)} bytes of stream")


if __name__ == "__main__":
    main()
