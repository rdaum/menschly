"""Write the demo's own test tune, an original electro loop, as a MIDI file.

    python3 tune.py [out.mid]

16 bars in F minor at BPM over Fm, Fm, Db, Eb (two bars each, twice). It
builds from a rolling 16th-note arpeggio alone, with the drums from bar 3,
a deep bass from bar 5, sparse chord stabs from bar 9 and a slow lead over
the last four. Channels: 0 bass, 1 stabs, 2 lead, 3 arpeggio, 9 drums (36
kick, 38 snare, 42 closed hat, 49 crash).
"""

import sys
from pathlib import Path

from midi import Note, write  # noqa: F401 (write is used by data.py)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BPM = 125
BARS = 16
BEATS = 4

# The chords, two bars each: the bass's root, and the chord's tones above
# the arpeggio's root (root, third, fifth, octave).
CHORDS = [
    (41, 65, (0, 3, 7, 12)),   # Fm
    (41, 65, (0, 3, 7, 12)),   # Fm
    (37, 61, (0, 4, 7, 12)),   # Db
    (39, 63, (0, 4, 7, 12)),   # Eb
]
# The arpeggio rolls up and down the chord's tones, a 16th each; its second
# bar of a chord goes an octave up.
ROLL = [0, 1, 2, 3, 2, 1, 0, 1, 2, 3, 3, 2, 1, 2, 3, 2]
# A bar of bass: (beat, semitones over the root, length): long notes with
# an octave drop and a push into the next bar.
BASS = [(0, 0, 1.25), (1.5, -12, 0.5), (2.25, 0, 0.75), (3.25, 12, 0.25), (3.5, 0, 0.4)]
# Stabs: one chord, just off the third beat.
STABS = [2.5]
# The lead: a slow phrase over four bars.
LEAD = [(0, 77, 1.5), (1.5, 75, 0.5), (2, 72, 2.0), (4, 70, 1.0), (5, 72, 1.0),
        (6, 68, 2.0), (8, 75, 1.5), (9.5, 77, 0.5), (10, 80, 2.0), (12, 79, 1.0),
        (13, 75, 1.0), (14, 77, 2.0)]
KICK, SNARE, HAT, CRASH = 36, 38, 42, 49


def tune():
    notes = []
    for bar in range(BARS):
        at = bar * BEATS
        bass_root, arp_root, tones = CHORDS[(bar // 2) % len(CHORDS)]
        octave = 12 if bar % 2 else 0
        for i, step in enumerate(ROLL):
            pitch = arp_root + tones[step] + octave
            notes.append(Note(at + i / 4, at + i / 4 + 0.2, 3, pitch, 110 if i % 4 == 0 else 85))
        if bar >= 2:
            # Kick on 1, the "and" of 2 and a push before 4; snare on 2 and
            # 4; 16th hats, the off-beats louder; a crash at each part.
            for beat in (0, 1.5, 2.75):
                notes.append(Note(at + beat, at + beat + 0.25, 9, KICK, 127))
            for beat in (1, 3):
                notes.append(Note(at + beat, at + beat + 0.25, 9, SNARE, 115))
            for i in range(16):
                notes.append(Note(at + i / 4, at + i / 4 + 0.1, 9, HAT, 100 if i % 2 else 60))
        if bar in (2, 4, 8, 12):
            notes.append(Note(at, at + 2, 9, CRASH, 100))
        if bar >= 4:
            for beat, step, length in BASS:
                notes.append(Note(at + beat, at + beat + length, 0, bass_root + step, 115))
        if bar >= 8:
            for beat in STABS:
                for tone in tones[1:]:
                    notes.append(Note(at + beat, at + beat + 0.3, 1, arp_root - 12 + tone, 96))
        if bar == 12:
            for beat, pitch, length in LEAD:
                notes.append(Note(at + beat, at + beat + length, 2, pitch, 100))
    return notes


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid" / "tune.mid"
    out.parent.mkdir(parents=True, exist_ok=True)
    write(out, tune(), BPM)
    print(f"tune: {BARS} bars at {BPM} bpm -> {out}")


if __name__ == "__main__":
    main()
