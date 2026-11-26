"""Put a blob into a ROM image's upper 64 KiB, which the MEGA65 has at
linear BANK ($30000) and the programs leave free (nothing of theirs is
placed in window banks).

    python3 inject_bank.py game.rom bank.bin

The programs' generated modules say where in the bank each thing lies:
demo_assets.py puts the sprite images at the bank's start, sound_bank.py
the sound after them.
"""

import sys

BANK = 0x30000
ROM_AT = 0x10000
SPACE = 0x10000


def inject(rom_path, blob):
    rom = bytearray(open(rom_path, "rb").read())
    assert len(blob) <= SPACE, f"the bank's blob is {len(blob)} bytes, over {SPACE}"
    if len(rom) < ROM_AT + len(blob):
        rom += bytes(ROM_AT + len(blob) - len(rom))
    assert not any(rom[ROM_AT:ROM_AT + len(blob)]), "the ROM's upper 64 KiB is in use"
    rom[ROM_AT:ROM_AT + len(blob)] = blob
    open(rom_path, "wb").write(rom)


if __name__ == "__main__":
    inject(sys.argv[1], open(sys.argv[2], "rb").read())
