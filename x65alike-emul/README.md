# x65alike-emul

`x65alike-emul` is a small fantasy 65816 machine model for emulator-backed
tests. It is inspired by the X65 project, but it is not a complete X65
emulator.

References:

- X65 repository: <https://github.com/jsyk/x65>
- X65 memory map: <https://github.com/jsyk/x65/blob/main/doc/mem-map.md>
- X65 I/O registers: <https://github.com/jsyk/x65/blob/main/doc/ioregs.md>

## Scope

The crate currently emulates enough of an X65-like modern 65816 machine to run
small assembled programs under the existing `wdc65816` CPU core:

- 24-bit CPU address space.
- 2 MiB SRAM backing store.
- Direct SRAM mapping for `$00_0000..$1F_FFFF`.
- BANK 0 I/O window at `$9F00..$9FFF`.
- NORA block registers:
  - `$9F50`: `RAMBLOCK`
  - `$9F51`: `ROMBLOCK`
  - `$9F52`: `RAMBMASK`
  - `$9F53`: `RMBCTRL`
- RAMBLOCK frame at `$A000..$BFFF`.
- ROMBLOCK frame at `$C000..$FFFF`.
- A scriptable subset of NORA's USB UART registers in the I/O window, including
  minimal UART IRQ line assertion.
- A simple 512-byte-sector block disk device for OS and filesystem
  experiments.

The model deliberately skips most of the real X65 peripheral set for now. VERA,
AURA, VIA, W6100, flash, timers, keyboard, video, and true SD-card/SPI devices
are not implemented yet. Interrupt support is currently limited to USB UART IRQ
assertion and runner-side maskable interrupt dispatch.

## Memory Map

The broad shape follows X65's documented NORA address decoding:

| CPU address range | Current behavior |
| --- | --- |
| `$00_0000..$1F_FFFF` | Direct 2 MiB SRAM mapping |
| `$00_0000` | SRAM byte, or `RAMBLOCK` mirror when `RMBCTRL.MIRROR_ZP` is set |
| `$00_0001` | SRAM byte, or `ROMBLOCK` mirror when `RMBCTRL.MIRROR_ZP` is set |
| `$00_9F00..$00_9FFF` | I/O window |
| `$00_9F50` | NORA `RAMBLOCK` register |
| `$00_9F51` | NORA `ROMBLOCK` register |
| `$00_9F52` | NORA `RAMBMASK` register |
| `$00_9F53` | NORA `RMBCTRL` register |
| `$00_9F60..$00_9F6D` | x65-alike block disk registers |
| `$00_A000..$00_BFFF` | RAMBLOCK-selected 8 KiB SRAM frame |
| `$00_C000..$00_FFFF` | ROMBLOCK-selected 16 KiB frame |

RAMBLOCK uses the X65-style inverted high bit for the selected frame:

```text
physical = ((RAMBLOCK & RAMBMASK) xor $80) * $2000 + (addr - $A000)
```

ROMBLOCK currently maps into the middle SRAM region starting at `$08_0000`:

```text
physical = $08_0000 + (ROMBLOCK & $1f) * $4000 + (addr - $C000)
```

`RMBCTRL.MIRROR_ZP` bit 5 controls whether `$0000/$0001` mirror the block
registers. The emulator defaults that bit off, so plain zero-page reads and
writes hit SRAM until software enables the mirror. `RMBCTRL.MAP_BOOTROM` bit 7
maps the ROM frame to a small placeholder PBL buffer instead of SRAM.

## USB UART I/O

The emulator implements the polling-visible subset of NORA's USB UART registers
so tests can script input and inspect output without a terminal:

| Address | Name | Behavior |
| --- | --- | --- |
| `$9F55` | `USB_UART_CTRL` | Read/write latch for UART control bits |
| `$9F56` | `USB_UART_STAT` | Bit 7: RX FIFO empty, bit 3: TX FIFO full, bit 2: TX empty/sent |
| `$9F57` | `USB_UART_DATA` | Read pops one input byte; write appends one output byte |

The emulated TX path is never full, so bit 3 currently reads as 0 and bit 2
reads as 1. `USB_UART_CTRL` bit 6 asserts the machine IRQ line while RX is not
empty, and bit 7 asserts it while TX is not full.

## Block Disk I/O

The emulator includes a deliberately small hard-disk-like block device. It is
not an SD-card protocol emulator and it does not provide file operations. It is
a physical 512-byte sector backend intended to sit below BIOS block-device calls
and, later, FAT12/FAT16/FAT32 code.

| Address | Name | Behavior |
| --- | --- | --- |
| `$9F60` | `BLOCK_LBA0` | LBA byte 0 |
| `$9F61` | `BLOCK_LBA1` | LBA byte 1 |
| `$9F62` | `BLOCK_LBA2` | LBA byte 2 |
| `$9F63` | `BLOCK_LBA3` | LBA byte 3 |
| `$9F64` | `BLOCK_INDEX_LO` | Low byte of the 0..511 sector-buffer index |
| `$9F65` | `BLOCK_INDEX_HI` | Bit 0 of the sector-buffer index |
| `$9F66` | `BLOCK_DATA` | Read/write sector-buffer byte, then auto-increment index |
| `$9F67` | `BLOCK_COMMAND` | Write `1` to read sector, `2` to write sector, `$7F` to clear error |
| `$9F68` | `BLOCK_STATUS` | Bit 0 present, bit 1 ready, bit 2 error, bit 3 write-protected |
| `$9F69` | `BLOCK_ERROR` | `0` none, `1` no media, `2` LBA out of range, `3` write-protected, `$FF` bad command |
| `$9F6A..$9F6D` | `BLOCK_SECTOR_COUNT0..3` | Attached image size in 512-byte sectors |

The command register operates on one sector at a time. A read command copies
the selected LBA into the internal 512-byte buffer and resets the buffer index
to zero. Software then reads `BLOCK_DATA` 512 times. For writes, software fills
the buffer through `BLOCK_DATA`, sets the LBA, and writes command `2`.

This shape is intentionally closer to a BIOS physical block device than to
VERA's real SD-card SPI transport. A future VERA/SPI model can live underneath
the same BIOS block API without changing filesystem code.

## Interrupts

`Machine::irq_pending()` reports the current level of the emulated IRQ line.
The runner checks that line between CPU instructions. If the CPU interrupt
disable flag is clear, it pushes a 65816 interrupt frame, sets `I`, clears `D`,
sets `PBR` to zero, and loads `PC` through the native IRQ vector at `$FFEE`
or the emulation IRQ vector at `$FFFE`.

This is still deliberately minimal: there is no interrupt priority controller,
edge/latch model, NMI source, or cycle timing.

Example use from tests:

```rust
let mut machine = x65alike_emul::Machine::new();
machine.push_serial_input(b"!");
machine.write(x65alike_emul::USB_UART_DATA, b'O');
assert_eq!(machine.serial_output(), b"O");
```

## Tests

Run the emulator crate tests through the repository root:

```sh
PATH=/tmp/cc65-root/usr/bin:$PATH cargo test
```

The integration test at `tests/fantasy_machine_emulator.rs` assembles a small
65816 program with `ca65`, runs it on the `wdc65816` CPU core with this memory
map behind it, scripts serial input, and checks the captured output.

## Running An Image

The crate also provides a small runner binary:

```sh
cargo run --manifest-path x65alike-emul/Cargo.toml --bin x65alike-emul -- path/to/image.xex
```

The runner currently accepts raw binaries, ld65's Atari/XEX output format, and a
small x65-native container format. Format auto-detection is intentionally
conservative: `.xex` and `.obx` files are parsed as Atari/XEX, `.x65` and
`.x65img` files are parsed as x65-native images, `.bin` and `.raw` files are
treated as flat binaries, and extensionless files default to raw unless
`--format` is supplied.

For raw binaries, the image is loaded at `$02_0000` by default and execution
starts at `PBR:PC = $02:$0000`. For Atari/XEX images, each block is loaded at
its 16-bit address in bank zero, and the optional `RUNAD` block sets the initial
PC. For x65-native images, each block carries a 24-bit SRAM load address and the
entry point carries both PBR and PC. Stdin is queued as USB UART input, and bytes
written to `USB_UART_DATA` are streamed to stdout.

Common options:

```sh
x65alike-emul program.xex
x65alike-emul --format xex program
x65alike-emul --format x65 banked-os.x65
x65alike-emul --format raw --load $020000 --pc $0000 --pbr $02 --max-steps 100000 image.bin
x65alike-emul --stop-at $03:$1234 image.bin
x65alike-emul --disk fat32.img program.xex
x65alike-emul --disk fat32.img --disk-read-only program.xex
printf 'hello' | x65alike-emul --quiet image.bin
```

An ld65 config can produce a directly bootable XEX by selecting the Atari file
format and naming the entry symbol as `RUNAD`:

```text
FILES {
  %O: format = atari;
}
FORMATS {
  atari: runad = main;
}
```

Atari/XEX is useful as a common ld65-emitted, address-bearing test image format,
but it is still a 16-bit loader format. It cannot describe arbitrary 65816
program banks. The x65-native image format fills that gap for emulator-backed
OS work. Its current binary layout is:

```text
8 bytes  magic: "X65IMG01"
3 bytes  little-endian 24-bit entry address
2 bytes  little-endian block count
repeat block count times:
  3 bytes  little-endian 24-bit load address
  4 bytes  little-endian byte count
  n bytes  payload
```

The runner is deliberately minimal. It does not yet emulate reset-vector boot,
NMI delivery, S-record/Intel HEX input, o65 host-side loading, SD-card SPI, or
cycle timing.
