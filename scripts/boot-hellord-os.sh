#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${TMPDIR:-/tmp}/mensch-hellord-os"

if ! command -v ca65 >/dev/null 2>&1 && [ -x /tmp/cc65-root/usr/bin/ca65 ]; then
  export PATH="/tmp/cc65-root/usr/bin:$PATH"
fi

mkdir -p "$build_dir"

src="$build_dir/hellord-os.mensch"
asm="$build_dir/hellord-os.a65"
obj="$build_dir/hellord-os.o"
cfg="$build_dir/hellord-os.cfg"
bin="$build_dir/hellord-os.bin"
max_steps=10000000
if [ -t 0 ]; then
  max_steps=0
  saved_stty="$(stty -g)"
  trap 'stty "$saved_stty"' EXIT
  stty -echo
fi

cat > "$src" <<'MENSCH'
module HellordOs
application main
fixed($009f56) volatile var serial_status: u8
fixed($009f57) volatile var serial_data: u8

export procedure main()
    serial_data = 'H'
    serial_data = 'e'
    serial_data = 'l'
    serial_data = 'l'
    serial_data = 'o'
    serial_data = 'r'
    serial_data = 'd'
    serial_data = '!'
    serial_data = '\r'
    serial_data = '\n'

    while 1 != 0
        while serial_status & $80 != 0
        end
        serial_data = serial_data
    end
end
MENSCH

cat > "$cfg" <<'CFG'
MEMORY {
  ZP: start = $000080, size = $000080, type = rw;
  CODE: start = $020000, size = $001000, file = %O;
  META: start = $020f00, size = $000100, file = %O;
  BSS: start = $021000, size = $001000, type = rw;
}
SEGMENTS {
  ZEROPAGE: load = ZP, type = zp;
  CODE: load = CODE, type = ro;
  MENSCH_META: load = META, type = ro;
  BSS: load = BSS, type = bss;
}
CFG

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin menschc -- "$src" -o "$asm"
ca65 -g --cpu 65816 "$asm" -o "$obj"
ld65 -C "$cfg" "$obj" -o "$bin"
cargo run --quiet --manifest-path "$repo_root/x65alike-emul/Cargo.toml" --bin x65alike-emul -- \
  --quiet \
  --load '$020000' \
  --pbr '$02' \
  --pc '$0000' \
  --max-steps "$max_steps" \
  "$bin"
