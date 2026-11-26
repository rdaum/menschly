#!/usr/bin/env python3
"""Prepare a working directory for reverse-engineering Bubble Bobble's Z80 code.

Writes, from the US/Romstar ROM set (bublboblr), CPU-view images of each
program ROM and bank, z80dasm disassemblies at the right origins, and seed
symbol files of the addresses known so far, for the main CPU (seed.sym) and
the sound CPU (sound-seed.sym).

  python3 re_prep.py ../assets/bubs            # -> target/bubble-bobble/re
  python3 re_prep.py ROMDIR --output-dir DIR

Needs z80dasm on PATH. Everything written is ROM-derived and stays local.
"""
import argparse
import shutil
import subprocess
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

# (file, CRC32, size) of the program ROMs, as MAME's bublboblr lists them.
FIXED = ("a78-25.51", 0x2D901C9D, 0x8000)    # main CPU $0000-$7FFF
BANKED = ("a78-24.52", 0xB7AFEDC4, 0x10000)  # main CPU $8000-$BFFF, 4 x 16 KiB
SUB = ("a78-08.37", 0xAE11A07B, 0x8000)      # second Z80 $0000-$7FFF
SOUND = ("a78-07.46", 0x4F9A26E8, 0x8000)    # sound Z80 $0000-$7FFF

# Seed symbols for the main CPU, in z80dasm's `name: equ 0x...` form (it
# reads a leading 0 as octal): hardware from MAME's bublbobl.cpp, the rest
# found in the code.
SYMBOLS = """\
; Main CPU hardware (MAME bublbobl.cpp)
videoram:        equ 0xc000  ; $C000-$DCFF
objectram:       equ 0xdd00  ; $DD00-$DFFF, sprite/object list
share1:          equ 0xe000  ; $E000-$F7FF, shared with the second Z80
palette:         equ 0xf800  ; $F800-$F9FF
sound_latch:     equ 0xfa00
sound_flags:     equ 0xfa01
sound_reset:     equ 0xfa03
watchdog:        equ 0xfa80
sub_nmi:         equ 0xfb00
bank_latch:      equ 0xfb40  ; bits 0-2 bank (entry = (v ^ 4) & 7), 4 sub reset, 5 MCU reset, 6 video, 7 flip
mcu_ram:         equ 0xfc00  ; $FC00-$FFFF, shared with the 6801 MCU; IRQ vector byte at $FC00
; Found in the code
select_bank:     equ 0x026c  ; A = logical bank; latch value from $0293 or $0297 by mcu_ram+$21 bit 7
load_round:      equ 0x1775  ; copies the round's 43-byte record to round_record
multiply_a_b:    equ 0x0db1  ; HL = A * B (used for round * 43)
round_table:     equ 0xa73a  ; bank entry 1: 43 bytes per round, 1-based round 1 first
round_record:    equ 0xe598  ; $E598-$E5C2, the current round's record
round_number:    equ 0xe64b  ; 0-based
enemy_drops:     equ 0xe5a9  ; record+$11: per enemy (drop, x, y), 7 entries
"""

# Seed symbols for the sound CPU's own address space.
SOUND_SYMBOLS = """\
; Sound CPU hardware (MAME bublbobl.cpp)
sound_ram:       equ 0x8000  ; $8000-$8FFF
ym2203_address:  equ 0x9000  ; $9000-$9FFF mirror: address/status (even), data (odd)
ym3526_address:  equ 0xa000  ; $A000-$AFFF mirror: address/status (even), data (odd)
command_latch:   equ 0xb000  ; read: command from the main CPU; write: to the main CPU
nmi_enable:      equ 0xb001  ; write: enable the command NMI (on leaving the NMI handler and the dispatcher)
nmi_disable:     equ 0xb002  ; write: disable it (on entering them)
diag_rom:        equ 0xe000  ; optional diagnostic ROM, hooked when $E000 holds $C3
; Found in the code
reset:           equ 0x00dc  ; $0003 jumps here
irq:             equ 0x0195  ; $0038 jumps here; IRQ from either chip (MAME merges both)
queue_command:   equ 0x00b3  ; A < $F0: append to command_queue (carry clear); else carry set
dispatch:        equ 0x02a3  ; takes commands off the queue
play_command:    equ 0x02ef  ; commands $00-$34, through command_table
control_e0:      equ 0x03b0  ; commands $E0-$EF
control_f0:      equ 0x044b  ; commands $F0-$FE
single_entry:    equ 0x0332  ; a descriptor whose first byte is $1B or more
ym3526_channels: equ 0x04d8  ; 9 channels, records at opl_layer_a, $A0 apart
ym3526_layer_b:  equ 0x0535  ; the same 9 channels' second records at opl_layer_b
command_table:   equ 0x329c  ; 2-byte pointers, commands $00-$34
opl_layer_a:     equ 0x8440
opl_layer_b:     equ 0x8490
queue_head:      equ 0x8f80
queue_tail:      equ 0x8f81
command_queue:   equ 0x8f82  ; 16 bytes
chip_base:       equ 0x8f9a  ; the chip the current channel loop writes
store_mode:      equ 0x8fab  ; bit 0: store commands at $8FAC instead of playing them
"""


def load(directory, name, crc, size):
    path = directory / name
    data = path.read_bytes()
    if len(data) != size or zlib.crc32(data) != crc:
        raise SystemExit(f"{path}: not the bublboblr ROM (size {len(data)}, "
                         f"CRC {zlib.crc32(data):08X}; expected {size}, {crc:08X})")
    return data


def disassemble(binary, origin, output, symbols=None):
    command = ["z80dasm", "-a", "-l", "-g", hex(origin), "-o", str(output), str(binary)]
    if symbols is not None:
        command[1:1] = ["-S", str(symbols)]
    subprocess.run(command, check=True, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("roms", type=Path, help="directory with the bublboblr ROM files")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/bubble-bobble/re")
    args = parser.parse_args()
    if shutil.which("z80dasm") is None:
        raise SystemExit("z80dasm is not on PATH")
    out = args.output_dir
    out.mkdir(parents=True, exist_ok=True)
    symbols = out / "seed.sym"
    symbols.write_text(SYMBOLS, encoding="ascii")
    sound_symbols = out / "sound-seed.sym"
    sound_symbols.write_text(SOUND_SYMBOLS, encoding="ascii")

    images = [("main-fixed", load(args.roms, *FIXED), 0x0000)]
    banked = load(args.roms, *BANKED)
    for entry in range(4):
        # Bank entry n is selected by latch value n ^ 4 (4, 5, 6, 7).
        images.append((f"main-bank{entry}", banked[entry * 0x4000:(entry + 1) * 0x4000], 0x8000))
    images.append(("sub", load(args.roms, *SUB), 0x0000))
    images.append(("sound", load(args.roms, *SOUND), 0x0000))

    for name, data, origin in images:
        binary = out / f"{name}.bin"
        binary.write_bytes(data)
        # Each CPU has its own address space and so its own symbols.
        cpu_symbols = {"main": symbols, "sound": sound_symbols}.get(name.split("-")[0])
        disassemble(binary, origin, out / f"{name}.asm", cpu_symbols)
        print(f"{name:11} ${origin:04X}-${origin + len(data) - 1:04X}  {binary.name}, {name}.asm")
    print(f"seed symbols: {symbols}, {sound_symbols}")


if __name__ == "__main__":
    main()
