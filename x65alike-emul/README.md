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
- A scriptable subset of NORA's USB UART registers in the I/O window.

The model deliberately skips most of the real X65 peripheral set for now. VERA,
AURA, VIA, W6100, flash, interrupts, timers, keyboard, video, and storage
devices are not implemented yet.

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
reads as 1. UART interrupt-enable bits are retained in `USB_UART_CTRL`, but
interrupt delivery is not implemented yet.

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
cargo run --manifest-path x65alike-emul/Cargo.toml --bin x65alike-emul -- path/to/image.bin
```

By default the image is loaded at `$02_0000` and execution starts at
`PBR:PC = $02:$0000`. Stdin is queued as USB UART input, and bytes written to
`USB_UART_DATA` are streamed to stdout.

Common options:

```sh
x65alike-emul --load $020000 --pc $0000 --pbr $02 --max-steps 100000 image.bin
x65alike-emul --stop-at $03:$1234 image.bin
printf 'hello' | x65alike-emul --quiet image.bin
```

The runner is deliberately minimal. It does not yet emulate reset-vector boot,
interrupt delivery, or real-time terminal input.
