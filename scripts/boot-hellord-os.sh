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

export procedure main()
    asm "phk"
    asm "plb"
    asm "sep #$20"
    asm ".a8"
    asm "rep #$10"
    asm ".i16"
    asm "ldx #0"
    asm "hellord_loop:"
    asm "lda hellord_text,x"
    asm "beq echo_loop"
    asm "sta f:$009f70"
    asm "inx"
    asm "bra hellord_loop"
    asm "echo_loop:"
    asm "lda f:$009f71"
    asm "and #$01"
    asm "beq echo_loop"
    asm "lda f:$009f70"
    asm "sta f:$009f70"
    asm "bra echo_loop"
    asm "hellord_text:"
    asm ".byte \"Hellord!\", $0d, $0a, 0"
    return
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
