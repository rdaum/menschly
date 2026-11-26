#!/usr/bin/env bash
# Measure lib/actors.mensch with a swarm of N software bubbles (default 60),
# headless in Xemu: the average and worst raster lines actors_frame takes
# per frame over 128 frames, and the actor rows that found no slot.
#
#   games/bubble-bobble/bench-swarm.sh 60
#
# Uses the assets run-demo.sh / run-game.sh generate in target/bubble-bobble.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
assets="$root/target/bubble-bobble"
bubbles="${1:-60}"
work="$assets/bench-$bubbles"
mkdir -p "$work"

(cd "$here/tools" && python3 demo_assets.py >/dev/null)
printf 'module SwarmSize\nexport const BUBBLES = %d\n' "$bubbles" > "$work/swarm-size.mensch"
rom="$work/swarm.rom"
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" --work-dir "$work/build" \
  "$here/src/bubble-swarm.mensch" \
  "$here/src/bubble-video.mensch" \
  "$root/lib/vic4.mensch" \
  "$root/lib/actors.mensch" \
  "$assets/bubble-demo-data.mensch" \
  "$work/swarm-size.mensch" >/dev/null
python3 "$root/tools/inject_bank.py" "$rom" "$assets/demo-images.bin"

xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
mkdir -p "$work/sd"
# The 128 measured frames end at frame 248, about 5 s at 50 Hz; Xemu runs
# in real time and writes the RAM dump when it is stopped.
timeout 8 "$xemu" -headless -besure -nosound -virtsd -sdimg "$work/sd" \
  -rom "$rom" -dumpmem "$work/mem.bin" >/dev/null 2>&1 || true
python3 - "$work/mem.bin" <<'PY'
import struct, sys
total, worst, missing, bubbles = struct.unpack('<4H', open(sys.argv[1], 'rb').read()[0xf00:0xf08])
print(f"{bubbles} bubbles: actors_frame average {total / 128:.1f} raster lines, "
      f"worst {worst}; {missing / 128:.1f} actor rows without a slot per frame")
PY
