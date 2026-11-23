#!/usr/bin/env bash
# Builds examples/vic4-demo.mensch with lib/vic4.mensch into a MEGA65 ROM
# and opens it in Xemu (with a window). Move the sprite with joystick 2.
#   scripts/run-vic4-demo.sh               build and run
#   scripts/run-vic4-demo.sh --rom FILE    also keep the ROM (usable as MEGA65.ROM)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
rom="$work/vic4-demo.rom"
if [[ "${1:-}" == "--rom" ]]; then
  rom="$2"
fi

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin mensch-run-mega65 -- \
  --no-run --rom "$rom" --work-dir "$work/build" \
  "$repo_root/examples/vic4-demo.mensch" \
  "$repo_root/lib/vic4.mensch"

xemu="${MENSCH_XMEGA65:-$HOME/src/xemu/build/bin/xmega65.native}"
mkdir -p "$work/sd"
"$xemu" -besure -virtsd -sdimg "$work/sd" -rom "$rom"
