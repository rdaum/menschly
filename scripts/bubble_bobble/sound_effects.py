"""Turn the arcade's sound effects into SID streams for the MEGA65.

    python3 sound_effects.py 31 25 11 0d 2c 20 18

The arcade plays its effects on the YM2203's FM channels (capture_sound.py
logs their register writes per timer A tick). Each FM channel becomes a SID
voice: its pitch from the channel's F-number and block (times the loudest
carrier's multiplier), its gate on from the channel's first key-on. The
YM2203's envelopes run inside the chip, so the loudness comes instead from
MAME's recording of the effect sent alone (mame/reglog.lua): the level of
each tick of it sets the SID's master volume. The waveform is a guess from
the FM algorithm (WAVES; all carriers sum near a sine: triangle). A command
that goes on into a piece of music (the hurry-up jingle into the fast theme)
is kept up to where the music's loop starts.

Writes target/bubble-bobble/sound/<cmd>.sid, little-endian:
- u16 the tick rate in hundredths of a hertz;
- per tick a mask: bits 0-2 a voice's frequency changes (a u16 follows,
  per voice in order), bit 3 the volume (a u8 follows), bits 4-6 a voice
  starts (its control byte, waveform and gate, follows);
- END: the effect is over (the player silences the SID).
"""

import json
import os
import struct
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SOUND = ROOT / "target" / "bubble-bobble" / "sound"
ROMS = Path(os.environ.get("BUBBLE_ROMS", ROOT / "target" / "bubble-bobble" / "re" / "mame" / "roms"))
# The YM2203's clock, and its frequency: fnum * 2^block * YM_CLOCK / YM_DIVISOR.
YM_CLOCK = 3_000_000
YM_DIVISOR = 72 * (1 << 21)
SID_CLOCK = 1_000_000
AT = 150
FRAME_HZ = 59.185606
LONGEST = 3.0
END = 0x80
TRIANGLE, SAWTOOTH, PULSE = 0x10, 0x20, 0x40
GATE = 0x01
# The waveform for each FM algorithm: those summing several carriers come
# out closest to a triangle, those with modulators brighter.
WAVES = {0: SAWTOOTH, 1: SAWTOOTH, 2: SAWTOOTH, 3: SAWTOOTH,
         4: PULSE, 5: PULSE, 6: TRIANGLE, 7: TRIANGLE}
# The carriers of each algorithm (operator slots 0-3, as registers $30+4n
# address them: operators 1, 3, 2, 4).
CARRIERS = {0: [3], 1: [3], 2: [3], 3: [3], 4: [2, 3], 5: [1, 2, 3], 6: [1, 2, 3], 7: [0, 1, 2, 3]}
VOICES = 3


def record(cmd):
    """MAME's recording of the effect sent alone, mono, from the send."""
    state = SOUND / "mame-samples"
    state.mkdir(parents=True, exist_ok=True)
    wav = state / f"{cmd:02x}.wav"
    seconds = int(AT / FRAME_HZ + LONGEST + 2)
    env = dict(os.environ, CMD=f"{cmd:02X}", AT=str(AT), FRAMES=str(int(seconds * FRAME_HZ)),
               QUIET="100000", OUT=str(state / f"{cmd:02x}.log"), SDL_VIDEODRIVER="dummy",
               SDL_AUDIODRIVER="dummy", DISPLAY="", WAYLAND_DISPLAY="")
    subprocess.run(["timeout", "300", "/usr/games/mame", "bublboblr", "-rompath", str(ROMS),
                    "-video", "none", "-sound", "none", "-nothrottle", "-skip_gameinfo",
                    "-seconds_to_run", str(seconds), "-wavwrite", str(wav),
                    "-cfg_directory", str(state / "cfg"), "-nvram_directory", str(state / "nvram"),
                    "-autoboot_script", str(HERE / "mame" / "reglog.lua")],
                   env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    with wave.open(str(wav)) as w:
        rate, channels = w.getframerate(), w.getnchannels()
        audio = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).reshape(-1, channels)
    return audio.mean(axis=1)[int(AT / FRAME_HZ * rate):] / 32768.0, rate


def voices(cmd):
    """Per tick, per FM channel: (frequency in Hz or None, keyed on yet,
    waveform); and the timer A rate."""
    lines = (SOUND / f"{cmd:02x}.jsonl").read_text().splitlines()
    header = json.loads(lines[0])
    regs = {}
    keyed = [False] * VOICES
    ticks = []
    for line in lines[1:]:
        ev = json.loads(line)
        if ev.get("irq") == "a":
            ticks.append(snapshot(regs, keyed))
        elif ev.get("chip") == "opn":
            regs[ev["reg"]] = ev["value"]
            if ev["reg"] == 0x28 and ev["value"] & 0xF0 and (ev["value"] & 3) < VOICES:
                keyed[ev["value"] & 3] = True
    ticks.append(snapshot(regs, keyed))
    if header["repeat"]:
        ticks = ticks[:header["repeat"]["from_tick"]]
    return ticks, header["timer_a_hz"]


def snapshot(regs, keyed):
    out = []
    for ch in range(VOICES):
        high = regs.get(0xA4 + ch, 0)
        fnum = ((high & 7) << 8) | regs.get(0xA0 + ch, 0)
        algorithm = regs.get(0xB0 + ch, 0) & 7
        carriers = CARRIERS[algorithm]
        loudest = min(carriers, key=lambda op: regs.get(0x40 + op * 4 + ch, 0x7F) & 0x7F)
        multiple = regs.get(0x30 + loudest * 4 + ch, 0) & 0x0F
        hz = fnum * (1 << ((high >> 3) & 7)) * YM_CLOCK / YM_DIVISOR * (multiple or 0.5)
        out.append((hz if fnum else None, keyed[ch], WAVES[algorithm]))
    return out


def levels(cmd, rate_hz, count):
    """The recording's RMS level per tick, from the effect's first sound."""
    audio, rate = record(cmd)
    loud = np.flatnonzero(np.abs(audio) > 0.003)
    audio = audio[loud[0]:] if len(loud) else audio[:0]
    span = rate / rate_hz
    return np.array([np.sqrt(np.mean(audio[int(k * span):int((k + 1) * span)] ** 2))
                     if int(k * span) < len(audio) else 0.0 for k in range(count)])


def encode(ticks, rate_hz, volumes):
    """The SID stream; the effect starts at its first key-on."""
    first = next(i for i, tick in enumerate(ticks) if any(keyed for _, keyed, _ in tick))
    ticks = ticks[first:]
    volumes = volumes[:len(ticks)]
    last = max(i for i, v in enumerate(volumes) if v) if any(volumes) else 0
    out = bytearray(struct.pack("<H", round(rate_hz * 100)))
    freq = [None] * VOICES
    started = [False] * VOICES
    volume = None
    for k in range(last + 1):
        tick = ticks[min(k, len(ticks) - 1)]
        mask, body = 0, bytearray()
        for v, (hz, keyed, wave_) in enumerate(tick):
            reg = min(round(hz * (1 << 24) / SID_CLOCK), 0xFFFF) if hz else 0
            if keyed and reg != freq[v]:
                mask |= 1 << v
                body += struct.pack("<H", reg)
                freq[v] = reg
        if volumes[k] != volume:
            mask |= 8
            body.append(volumes[k])
            volume = volumes[k]
        for v, (hz, keyed, wave_) in enumerate(tick):
            if keyed and not started[v]:
                mask |= 0x10 << v
                body.append(wave_ | GATE)
                started[v] = True
        out.append(mask)
        out += body
    out.append(END)
    return bytes(out), last + 1


def main():
    SOUND.mkdir(parents=True, exist_ok=True)
    cmds = [int(arg, 16) for arg in sys.argv[1:]]
    found = {}
    for cmd in cmds:
        ticks, rate_hz = voices(cmd)
        found[cmd] = (ticks, rate_hz, levels(cmd, rate_hz, len(ticks) + int(LONGEST * rate_hz)))
    # One scale for all, so the effects keep the arcade's loudness against
    # one another.
    peak = max(max(lv.max() for _, _, lv in found.values()), 1e-9)
    for cmd, (ticks, rate_hz, lv) in found.items():
        volumes = [min(15, round(15 * x / peak)) for x in lv]
        data, count = encode(ticks, rate_hz, volumes)
        path = SOUND / f"{cmd:02x}.sid"
        path.write_bytes(data)
        print(f"${cmd:02X}: {count} ticks ({count / rate_hz:.2f} s), {len(data)} bytes -> {path}")


if __name__ == "__main__":
    main()
