#!/usr/bin/env bash
# Build the playable Bubble Bobble room and boot it in Xemu. Joystick 2:
# left/right to walk, up to jump (and, held, to bounce off bubbles), fire to
# blow a bubble. Xemu takes the arrow keys as the joystick and Right Ctrl as
# fire.
#
#   games/bubble-bobble/run-game.sh              build and run
#   games/bubble-bobble/run-game.sh --rom F.rom  also keep the ROM image
#   games/bubble-bobble/run-game.sh --no-run     build only
#
# Needs the US/Romstar ROM set in games/bubble-bobble/assets/bubs (or set BUBS=DIR) for the
# asset extraction; the generated assets stay in target/bubble-bobble.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
assets="$root/target/bubble-bobble"
bubs="${BUBS:-$here/assets/bubs}"

keep_rom=""
build_only=""
for arg in "$@"; do
  case "$arg" in
    --no-run) build_only=1 ;;
    --rom=*) keep_rom="${arg#--rom=}" ;;
  esac
done
if [[ "${1:-}" == "--rom" ]]; then keep_rom="${2:?--rom needs a file name}"; fi

mkdir -p "$assets"

# 1. Assets: ROM -> sprite atlas -> packed cells -> Mensch source.
if [[ ! -f "$assets/sprites.tsv" || ! -f "$assets/sprite_cells.bin" ]]; then
  echo "extracting assets from $bubs"
  (cd "$here/tools" && python3 extract.py "$bubs" --output-dir "$assets")
  (cd "$here/tools" && python3 convert.py "$assets")
fi
# rom_tables.py lists the sprites its tables use; demo_assets.py takes them.
(cd "$here/tools" && python3 rom_tables.py "$assets" >/dev/null)
(cd "$here/tools" && python3 demo_assets.py)
# The sound bank: the arcade's theme and effects, captured from MAME
# (capture_sound.py 30 0f; sound_effects.py 31 25 11 0d 2c 20 18).
(cd "$here/tools" && python3 sound_bank.py "$assets")
# The input replay module: replay.py writes one with a script of inputs;
# otherwise the game reads the joystick.
replay="${REPLAY_MODULE:-$assets/game-replay.mensch}"
if [[ -z "${REPLAY_MODULE:-}" ]]; then
  printf 'module GameReplay\nexport const REPLAY = 0\nexport const REPLAY_RUNS = 0\nexport const REPLAY_INPUT: [2]u8 = [0, 0]\nexport const REPLAY_LIVES_COUNT = 0\nexport const REPLAY_LIVES: [1]u8 = [0]\n' > "$replay"
fi

# 2. Build the ROM, then boot it windowed.
#    mensch-run-mega65 always runs Xemu headless (it is the test runner), so
#    build with --no-run and start Xemu directly, as run-vic4-demo.sh does.
rom="${GAME_ROM:-$assets/build/game.rom}"
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" \
  --work-dir "$assets/build-game" \
  "$here/src/bubble-game.mensch" \
  "$here/src/bubble-physics.mensch" \
  "$here/src/bubble-bubbles.mensch" \
  "$here/src/bubble-air.mensch" \
  "$here/src/bubble-video.mensch" \
  "$root/lib/vic4.mensch" \
  "$root/lib/actors.mensch" \
  "$assets/bubble-demo-data.mensch" \
  "$assets/bubble-rom-tables.mensch" \
  "$here/src/bubble-sound.mensch" \
  "$assets/bubble-sound-bank.mensch" \
  "$replay"

# The sprite images and the sound go in the ROM image's upper 64 KiB.
python3 "$root/tools/inject_bank.py" "$rom" "$assets/game-bank.bin"
echo "rom: $rom"

if [[ -n "$keep_rom" ]]; then cp "$rom" "$keep_rom"; echo "kept: $keep_rom"; fi
if [[ -n "$build_only" ]]; then exit 0; fi

xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
if [[ ! -x "$xemu" ]]; then
  echo "Xemu not found at $xemu (set MENSCH_XMEGA65)" >&2
  exit 1
fi
# Launch exactly as examples/run-vic4-demo.sh does: a real directory for
# -virtsd, so Xemu never falls back to its default @mega65.img.
sd="$assets/build/sd"
mkdir -p "$sd"
# The arrow keys are joystick 2 (Right Ctrl is fire).
exec "$xemu" -besure -curskeyjoy -joyport 2 -virtsd -sdimg "$sd" -rom "$rom"
