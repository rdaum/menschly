# Menschly

Mensch, a systems language built for the 65xx family; Hellord, an
operating system written in it; and the beginnings of games and a GUI for
the MEGA65.

- **Mensch** is a compiled language designed around how the 6502 family
  actually works: static storage by default, the base page as a register
  file, no integer promotion, 8-bit indexes, column-wise (`parallel`) arrays,
  register arguments and carry-flag results, explicit stack/reentrant
  routines, interrupts and traps. The compiler (Rust) emits ca65 assembly for
  the **65816** and the MEGA65's **45GS02**.
- **Hellord** is an OS written in Mensch: a DOS-style shell, FAT16/FAT32 on
  SD card, a GEMDOS/BIOS-style trap interface, and a loader that runs
  relocatable o65 programs. It boots on the MEGA65 as a replacement for the
  C65 ROM, and on an emulated X65-like 65816 machine.
- **`lib/vic4.mensch`** is a MEGA65 graphics and input library (VIC-IV
  full-colour tiles, palettes, DMA, raster interrupts, 16-colour sprites,
  joysticks), the start of a game and of an eventual GEM-like GUI.
- An **o65 relocating loader** in 65816 assembly, where the project began
  (see [`loader/README.md`](loader/README.md)).

## Repository layout

| Path | What |
|---|---|
| `compiler/src/` | The Mensch compiler: frontend, IR, storage assignment, 65816 and 45GS02 backends and optimizers, ca65 emitter, MEGA65 ROM builder and Xemu runner |
| `compiler/src/bin/` | Command-line tools (below) |
| `compiler/*.md` | Language documentation (below) |
| `examples/hellord-*.mensch` | Hellord's modules |
| `examples/mega65-machine.mensch`, `x65-machine.mensch`, `serial-ring.mensch` | Machine drivers: console and block device per machine |
| `examples/programs/` | Programs that Hellord loads and runs |
| `examples/vic4-demo.mensch` | Tiles, a raster sky gradient and 16-colour sprites on the MEGA65 |
| `lib/` | Mensch libraries (`vic4.mensch`) |
| `scripts/` | Boot Hellord or run the demo in an emulator |
| `loader/` | The o65 relocating loader in 65816 assembly |
| `x65alike-emul/` | A small X65-like 65816 machine model used by the tests |
| `tests/` | Emulator-backed integration tests |

## Documentation

- [`compiler/INTRODUCTION.md`](compiler/INTRODUCTION.md): why Mensch, what it
  does better than C on the 65xx (with examples and generated code), and what
  C still does better.
- [`compiler/LANGUAGE.md`](compiler/LANGUAGE.md): the language reference.
- [`compiler/ABI.md`](compiler/ABI.md): how each construct is laid out and
  called on the 65816 and the MEGA65, including window banks and the
  direct-page overlay.
- [`compiler/REGISTER_PLANNING.md`](compiler/REGISTER_PLANNING.md): notes on
  register and direct-page allocation.

## A taste of Mensch

```text
module Game
import Vic4

type Actor = record(x: u8, y: u8, dx: i8)
var actors: parallel [32]Actor          # stored column by column
var floor: [256]u8

procedure move_all()
    var i: u8
    for i = 0 to 31                     # lda actors,x / adc actors+64,x / sta actors,x
        actors[i].x = actors[i].x + u8(actors[i].dx)
    end
    return
end

# Arguments arrive in X and Y; the answer comes back in the carry flag.
function on_ground(x: u8 in x, y: u8 in y): u8 in carry
    if floor[x] == y
        return 1
    end
    return 0
end

export procedure main()
    move_all()
    if on_ground(actors[0].x, actors[0].y)    # a call, then bcc/bcs
        actors[0].dx = 0
    end
    if joystick(2) & JOY_FIRE
        vic4_border(2)
    end
    return
end
```

On a small benchmark the plain-routine code is 208 bytes against 290 for
`cc65 -O`, and 169 with `parallel` arrays (see `INTRODUCTION.md`).

## Requirements

- Rust (stable) and Cargo.
- cc65 with 45GS02 support (`ca65 --cpu 45GS02`; current cc65 master).
  `ca65` and `ld65` must be on `PATH`.
- For the MEGA65 tests and scripts: [Xemu](https://github.com/lgblgblgb/xemu)'s
  `xmega65`, found through `$MENSCH_XMEGA65`,
  `~/src/xemu/build/bin/xmega65.native` or `PATH`. Without it, the MEGA65
  tests still compile and link everything and then skip the run.
  - Booting a ROM with Xemu's virtual SD card (`-virtsd`) needs a local
    patch to Xemu's `sdcontent.c`, or Xemu stops in the ONBOARD setup
    utility.
  - The tests that type at Hellord's shell need an Xemu with the `-typein`
    option (also a local patch); they skip without it.

## Tools

| Binary | Use |
|---|---|
| `menschc` | Compile Mensch to ca65 assembly: `menschc --target mega65 -I lib game.mensch -o game.s` (`--target 65816` is the default) |
| `mensch-run-mega65` | Build MEGA65 modules into a 128 KiB ROM image and boot it headless in Xemu; `--rom FILE --no-run` just writes the ROM, `--window N:module` places a module in a window bank |
| `mensch-mega65-program` | Build a program that Hellord's `RUN` loads from the SD card |
| `mensch-run-x65` | Build and run Mensch modules on the X65-like 65816 machine |

## Running things

```sh
# Hellord on the MEGA65 (headless; prints the final screen)
scripts/boot-hellord-mega65.sh --type-in 'VER\nDIR\nREBOOT\n'

# Hellord on the X65-like 65816 machine (interactive in a terminal)
scripts/boot-hellord-os.sh

# The VIC-IV demo in an Xemu window (move the hero with joystick 2)
scripts/run-vic4-demo.sh

# A program for Hellord, then run it with `S:` and `HELLO`
cargo run --bin mensch-mega65-program -- -o HELLO.PRG examples/programs/hello.mensch
```

Hellord's shell commands are `HELP`, `VER`, `DIR`, `STAT`, `PWD`, `CD`,
`MAP`, `MEM`, `CLS`, `TYPE`, `RUN`, `PEEK`, `POKE`, `DUMP`, `FILL`,
`BLKINFO`, `SET`, `PROMPT`, `PATH`, `ECHO` and `REBOOT`; a drive letter
(`S:`) switches drives and a program's name runs it.

## Tests

```sh
cargo test
```

The suite covers the compiler's units (parsing, type checks, storage
assignment, both optimizers), generated 65816 code run on the `wdc65816`
emulator and the X65-like machine, the o65 loader, and MEGA65 ROMs booted
headless in Xemu: language features end to end, interrupts, window banks,
Hellord booting, reading and writing FAT volumes on a virtual SD card,
loading programs, and the VIC-IV library. `cargo clippy --all-targets` and
`cargo fmt` are kept clean.

## Status and direction

- The MEGA65 is the main target; the 65816 backend is kept working.
- Next: an asset converter (indexed PNG to tiles, sprites, palettes and
  maps), then a game engine on the VIC-IV library (tile collision, player
  physics, the raster rewrite buffer for many moving objects).
- Further out: a GEM-like GUI for Hellord.
- Known language gaps: types cannot refer to each other before both are
  declared, exported routines cannot take register arguments, and the 65816
  backend has no 32-bit integers.
