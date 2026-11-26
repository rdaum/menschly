#!/usr/bin/env bash
# Build a ROM that plays one of the arcade's captured sound commands on the
# MEGA65's FM chip (sound-player.mensch), and print how to run it.
#
#   games/bubble-bobble/play-sound.sh 0f
#
# Needs the command captured (capture_sound.py 0f). The stream goes into the
# ROM image's upper 64 KiB (linear $30000; inject_bank.py), where the player
# reads it.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cmd="${1:?sound command, e.g. 0f}"
out="$root/target/bubble-bobble/sound"
rom="$out/play-$cmd.rom"

python3 "$here/tools/sound_stream.py" "$cmd" "$out/$cmd.opl"
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" --work-dir "$out/build-player" \
  "$here/src/sound-player.mensch" "$root/lib/sound.mensch" "$root/lib/vic4.mensch" >/dev/null
python3 "$root/tools/inject_bank.py" "$rom" "$out/$cmd.opl"
xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
# Xemu wants an SD card; a virtual one in a directory will do.
sd="$out/sd"
mkdir -p "$sd"
echo "rom: $rom"
echo "to listen: $xemu -besure -virtsd -sdimg $sd -rom $rom"
