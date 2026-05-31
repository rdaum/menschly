#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v ca65 >/dev/null 2>&1 && [ -x /tmp/cc65-root/usr/bin/ca65 ]; then
  export PATH="/tmp/cc65-root/usr/bin:$PATH"
fi

src="$repo_root/examples/hellord-os.mensch"
dos_src="$repo_root/examples/hellord-dos.mensch"
console_src="$repo_root/examples/hellord-console.mensch"
files_src="$repo_root/examples/hellord-files.mensch"
serial_src="$repo_root/examples/serial-ring.mensch"
bios_src="$repo_root/examples/hellord-bios.mensch"
gemdos_src="$repo_root/examples/hellord-gemdos.mensch"
cfg="$repo_root/examples/hellord-os-x65.cfg"
max_steps=10000000
if [ -t 0 ]; then
  max_steps=0
  saved_stty="$(stty -g)"
  trap 'stty "$saved_stty"' EXIT
  stty -echo
fi

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin mensch-run-x65 -- \
  --config "$cfg" \
  --max-steps "$max_steps" \
  "$src" \
  "$dos_src" \
  "$console_src" \
  "$files_src" \
  "$serial_src" \
  "$bios_src" \
  "$gemdos_src"
