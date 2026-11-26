#!/usr/bin/env bash
# Build Into the Grid, a MEGA65 demo, and run it in Xemu (in a window).
#
#   demos/into-the-grid/run-demo.sh              build and run
#   demos/into-the-grid/run-demo.sh --no-run     build only
#   demos/into-the-grid/run-demo.sh --rom F.rom  also keep the ROM image
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/target/into-the-grid"
rom="$out/build/demo.rom"
build_only=""
keep_rom=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-run) build_only=1 ;;
    --rom) keep_rom="${2:?--rom needs a file name}"; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

mkdir -p "$out/build"
(cd "$here/tools" && python3 data.py "$out")
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" --work-dir "$out/build" \
  "$here/src/grid-demo.mensch" \
  "$here/src/chunky.mensch" \
  "$here/src/pane.mensch" \
  "$here/src/roto.mensch" \
  "$root/lib/vic4.mensch" \
  "$out/grid-data.mensch"
# The runner's frames go in the ROM image's upper 64 KiB.
python3 "$root/tools/inject_bank.py" "$rom" "$out/grid-bank.bin"
echo "rom: $rom"

if [[ -n "$keep_rom" ]]; then cp "$rom" "$keep_rom"; echo "kept: $keep_rom"; fi
if [[ -n "$build_only" ]]; then exit 0; fi

xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
sd="$out/build/sd"
mkdir -p "$sd"
exec "$xemu" -besure -virtsd -sdimg "$sd" -rom "$rom"
