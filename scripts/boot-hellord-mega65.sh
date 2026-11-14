#!/usr/bin/env bash
# Build Hellord as a MEGA65 kernel ROM and boot it headless in Xemu. The DOS
# shell runs from window bank 1, the FAT driver and program loader from bank 2.
# Extra arguments go to mensch-run-mega65, e.g.:
#   scripts/boot-hellord-mega65.sh --type-in 'VER\nDIR\nREBOOT\n'
#   scripts/boot-hellord-mega65.sh --rom MEGA65.ROM --no-run
#   scripts/boot-hellord-mega65.sh --sd-file README.TXT --type-in 'S:\nDIR\n'
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
examples="$repo_root/examples"

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin mensch-run-mega65 -- \
  "$@" \
  --window "1:$examples/hellord-dos.mensch" \
  --window "2:$examples/hellord-fat.mensch" \
  --window "2:$examples/hellord-exec.mensch" \
  "$examples/hellord-os.mensch" \
  "$examples/hellord-console.mensch" \
  "$examples/hellord-files.mensch" \
  "$examples/mega65-machine.mensch" \
  "$examples/hellord-bios.mensch" \
  "$examples/hellord-gemdos.mensch" \
  "$examples/hellord-traps-mega65.mensch"
