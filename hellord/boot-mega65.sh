#!/usr/bin/env bash
# Build Hellord as a MEGA65 kernel ROM and boot it headless in Xemu. The DOS
# shell runs from window bank 1, the FAT driver and program loader from bank 2.
# Extra arguments go to mensch-run-mega65, e.g.:
#   hellord/boot-mega65.sh --type-in 'VER\nDIR\nREBOOT\n'
#   hellord/boot-mega65.sh --rom MEGA65.ROM --no-run
#   hellord/boot-mega65.sh --sd-file README.TXT --type-in 'S:\nDIR\n'
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
hellord="$repo_root/hellord"

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin mensch-run-mega65 -- \
  "$@" \
  --window "1:$hellord/dos.mensch" \
  --window "2:$hellord/fat.mensch" \
  --window "2:$hellord/exec.mensch" \
  "$hellord/os.mensch" \
  "$hellord/console.mensch" \
  "$hellord/files.mensch" \
  "$hellord/machines/mega65.mensch" \
  "$hellord/bios.mensch" \
  "$hellord/gemdos.mensch" \
  "$hellord/traps-mega65.mensch"
