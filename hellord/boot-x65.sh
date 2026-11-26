#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v ca65 >/dev/null 2>&1 && [ -x /tmp/cc65-root/usr/bin/ca65 ]; then
  export PATH="/tmp/cc65-root/usr/bin:$PATH"
fi

src="$repo_root/hellord/os.mensch"
dos_src="$repo_root/hellord/dos.mensch"
fat_src="$repo_root/hellord/fat.mensch"
console_src="$repo_root/hellord/console.mensch"
files_src="$repo_root/hellord/files.mensch"
serial_src="$repo_root/hellord/machines/serial-ring.mensch"
bios_src="$repo_root/hellord/bios.mensch"
machine_src="$repo_root/hellord/machines/x65.mensch"
gemdos_src="$repo_root/hellord/gemdos.mensch"
traps_src="$repo_root/hellord/traps.mensch"
cfg="$repo_root/hellord/machines/x65.cfg"
max_steps=10000000
if [ -t 0 ]; then
  max_steps=0
  saved_stty="$(stty -g)"
  trap 'stty "$saved_stty"' EXIT
  stty -echo
fi

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin mensch-run-x65 -- \
  --config "$cfg" \
  --format x65 \
  --entry-bank 0 \
  --entry a000 \
  --block 002000:lowcode.bin \
  --block 00a000:code.bin \
  --block 01c000:bank1.bin \
  --block 009f00:meta.bin \
  --block 00ffe4:vectors.bin \
  --max-steps "$max_steps" \
  "$src" \
  "$dos_src" \
  "$fat_src" \
  "$console_src" \
  "$files_src" \
  "$serial_src" \
  "$bios_src" \
  "$machine_src" \
  "$gemdos_src" \
  "$traps_src"
