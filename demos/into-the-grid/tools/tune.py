"""Write the demo's own test tune, an original electro loop, as a MIDI file.

    python3 tune.py [out.mid]

16 bars in F minor at BPM, over a four-chord turn (Fm, Db, Bbm, C, two
bars each... once through the 8 bars twice). It builds: drums and a
syncopated bass for 4 bars, off-beat chord stabs join for 4, then a lead
line for the last 8. Channels: 0 bass, 1 stabs, 2 lead, 9 drums (36 kick,
38 snare, 42 closed hat, 49 crash).
"""

import sys
from pathlib import Path

from midi import Note, write  # noqa: F401 (write is used by data.py)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BPM = 125
BARS = 16
BEATS = 4

# The chords, two bars each: the bass's root and the stab's voicing.
CHORDS = [
    (41, (65, 68, 72)),   # Fm
    (37, (61, 65, 68)),   # Db
    (34, (58, 61, 65)),   # Bbm
    (36, (60, 64, 67)),   # C
]
# A bar of bass: (beat, semitones over the root, length).
BASS = [(0, 0, 0.5), (0.75, 12, 0.25), (1.5, 0, 0.5), (2.25, 7, 0.25),
        (2.5, 0, 0.5), (3.25, 10, 0.25), (3.5, 0, 0.4)]
# Stabs on the off-beats.
STABS = [0.5, 1.75, 3.0]
# The lead: two bars at a time, its last note following the chord.
LEAD = [(0, 72, 0.5), (0.5, 75, 0.5), (1, 77, 1.0), (2.5, 75, 0.25), (2.75, 72, 0.75),
        (4, 70, 0.5), (4.5, 72, 1.25), (6, 68, 0.5), (6.5, 67, 1.25)]
LEAD_ENDS = [67, 68, 65, 67]
KICK, SNARE, HAT, CRASH = 36, 38, 42, 49


def tune():
    notes = []
    for bar in range(BARS):
        at = bar * BEATS
        root, chord = CHORDS[(bar // 2) % len(CHORDS)]
        # Drums: kick with a push before the third beat, snare on 2 and 4,
        # hats on every eighth (louder off the beat), a crash at each part.
        for beat in (0, 0.75, 2, 2.5):
            notes.append(Note(at + beat, at + beat + 0.25, 9, KICK, 120))
        for beat in (1, 3):
            notes.append(Note(at + beat, at + beat + 0.25, 9, SNARE, 110))
        for i in range(8):
            notes.append(Note(at + i / 2, at + i / 2 + 0.125, 9, HAT, 100 if i % 2 else 70))
        if bar in (0, 4, 8, 12):
            notes.append(Note(at, at + 2, 9, CRASH, 100))
        for beat, step, length in BASS:
            notes.append(Note(at + beat, at + beat + length, 0, root + step, 110))
        if bar >= 4:
            for beat in STABS:
                for pitch in chord:
                    notes.append(Note(at + beat, at + beat + 0.25, 1, pitch, 96))
        if bar >= 8 and bar % 2 == 0:
            phrase = LEAD[:-1] + [(LEAD[-1][0], LEAD_ENDS[(bar // 2) % 4], LEAD[-1][2])]
            for beat, pitch, length in phrase:
                notes.append(Note(at + beat, at + beat + length, 2, pitch, 100))
    return notes


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid" / "tune.mid"
    out.parent.mkdir(parents=True, exist_ok=True)
    write(out, tune(), BPM)
    print(f"tune: {BARS} bars at {BPM} bpm -> {out}")


if __name__ == "__main__":
    main()
