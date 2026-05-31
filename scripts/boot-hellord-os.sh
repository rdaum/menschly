#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${TMPDIR:-/tmp}/mensch-hellord-os"

if ! command -v ca65 >/dev/null 2>&1 && [ -x /tmp/cc65-root/usr/bin/ca65 ]; then
  export PATH="/tmp/cc65-root/usr/bin:$PATH"
fi

mkdir -p "$build_dir"

src="$repo_root/examples/hellord-os.mensch"
asm="$build_dir/hellord-os.a65"
obj="$build_dir/hellord-os.o"
cfg="$repo_root/examples/hellord-os-x65.cfg"
xex="$build_dir/hellord-os.xex"
max_steps=10000000
if [ -t 0 ]; then
  max_steps=0
  saved_stty="$(stty -g)"
  trap 'stty "$saved_stty"' EXIT
  stty -echo
fi

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin menschc -- "$src" -o "$asm"
ca65 -g --cpu 65816 "$asm" -o "$obj"
ld65 -C "$cfg" "$obj" -o "$xex"
cargo run --quiet --manifest-path "$repo_root/x65alike-emul/Cargo.toml" --bin x65alike-emul -- \
  --quiet \
  --max-steps "$max_steps" \
  "$xex"
