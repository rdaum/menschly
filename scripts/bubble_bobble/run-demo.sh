#!/usr/bin/env bash
# Build the Bubble Bobble animation demo and boot it in Xemu.
#
#   scripts/bubble_bobble/run-demo.sh              build and run
#   scripts/bubble_bobble/run-demo.sh --rom F.rom  also keep the ROM image
#   scripts/bubble_bobble/run-demo.sh --no-run     build only
#
# Needs the US/Romstar ROM set in examples/bubs (or set BUBS=DIR) for the
# asset extraction; the generated assets stay in target/bubble-bobble.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
assets="$root/target/bubble-bobble"
bubs="${BUBS:-$root/examples/bubs}"

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
if [[ ! -f "$assets/sprites.tsv" || ! -f "$assets/sprite_palettes.rgb" ]]; then
  echo "extracting assets from $bubs"
  (cd "$here" && python3 extract.py "$bubs" --output-dir "$assets")
  (cd "$here" && python3 convert.py "$assets")
fi
(cd "$here" && python3 demo_assets.py)

# 2. Build the ROM, then boot it windowed.
#    mensch-run-mega65 always runs Xemu headless (it is the test runner), so
#    build with --no-run and start Xemu directly, as run-vic4-demo.sh does.
rom="$assets/build/demo.rom"
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" \
  --work-dir "$assets/build" \
  "$here/bubble-demo.mensch" \
  "$here/bubble-anim.mensch" \
  "$here/bubble-video.mensch" \
  "$root/lib/vic4.mensch" \
  "$assets/bubble-demo-data.mensch"

echo "rom: $rom"

if [[ -n "$keep_rom" ]]; then cp "$rom" "$keep_rom"; echo "kept: $keep_rom"; fi
if [[ -n "$build_only" ]]; then exit 0; fi

xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
if [[ ! -x "$xemu" ]]; then
  echo "Xemu not found at $xemu (set MENSCH_XMEGA65)" >&2
  exit 1
fi
# Launch exactly as scripts/run-vic4-demo.sh does: a real directory for
# -virtsd, so Xemu never falls back to its default @mega65.img.
sd="$assets/build/sd"
mkdir -p "$sd"
exec "$xemu" -besure -virtsd -sdimg "$sd" -rom "$rom"
