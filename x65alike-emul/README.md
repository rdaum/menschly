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
- BANK 0 control registers:
  - `$0000`: `RAMBLOCK`
  - `$0001`: `ROMBLOCK`
- BANK 0 I/O window at `$9F00..$9FFF`.
- RAMBLOCK frame at `$A000..$BFFF`.
- ROMBLOCK frame at `$C000..$FFFF`.
- A simple scriptable serial device in the I/O window.

The model deliberately skips the real X65 peripheral set for now. VERA, AURA,
VIA, W6100, flash, interrupts, timers, keyboard, video, and storage devices are
not implemented yet.

## Memory Map

The broad shape follows X65's documented NORA address decoding:

| CPU address range | Current behavior |
| --- | --- |
| `$00_0000..$1F_FFFF` | Direct 2 MiB SRAM mapping |
| `$00_0000` | `RAMBLOCK` register in BANK 0 |
| `$00_0001` | `ROMBLOCK` register in BANK 0 |
| `$00_9F00..$00_9FFF` | I/O window |
| `$00_A000..$00_BFFF` | RAMBLOCK-selected 8 KiB SRAM frame |
| `$00_C000..$00_FFFF` | ROMBLOCK-selected 16 KiB frame |

RAMBLOCK uses the X65-style inverted high bit for the selected frame:

```text
physical = (RAMBLOCK xor $80) * $2000 + (addr - $A000)
```

ROMBLOCK currently maps into the middle SRAM region starting at `$08_0000`:

```text
physical = $08_0000 + (ROMBLOCK & $1f) * $4000 + (addr - $C000)
```

If bit 7 of `ROMBLOCK` is set, the ROM frame maps to a small placeholder PBL
buffer instead.

## Serial I/O

The serial device is intentionally simple so tests can script input and inspect
output without a terminal:

| Address | Name | Behavior |
| --- | --- | --- |
| `$9F70` | `SERIAL_DATA` | Read pops one input byte; write appends one output byte |
| `$9F71` | `SERIAL_STATUS` | Bit 0: RX ready, bit 1: TX ready |
| `$9F72` | `SERIAL_CONTROL` | Read/write latch, no behavior yet |

Example use from tests:

```rust
let mut machine = x65alike_emul::Machine::new();
machine.push_serial_input(b"!");
machine.write(x65alike_emul::SERIAL_DATA, b'O');
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
cargo run --manifest-path x65alike-emul/Cargo.toml --bin x65alike-emul -- path/to/image.bin
```

By default the image is loaded at `$02_0000` and execution starts at
`PBR:PC = $02:$0000`. Stdin is queued as serial input, and bytes written to the
serial data register are streamed to stdout.

Common options:

```sh
x65alike-emul --load $020000 --pc $0000 --pbr $02 --max-steps 100000 image.bin
x65alike-emul --stop-at $03:$1234 image.bin
printf 'hello' | x65alike-emul --quiet image.bin
```

The runner is deliberately minimal. It does not yet emulate reset-vector boot,
interrupt delivery, or real-time terminal input.
