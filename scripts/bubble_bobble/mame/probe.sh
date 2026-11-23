#!/usr/bin/env bash
# Run Bubble Bobble (bublboblr) headless in MAME with probe.lua for SECONDS
# of emulated time (env as probe.lua; snapshots to $SNAPDIR).
#
#   OUT=trace.tsv RANGES=E690-E693 INPUTS="2500:P1 Right:1" ./probe.sh 50
#
# ROMs come from $BUBBLE_ROMS (default target/bubble-bobble/re/mame/roms,
# holding bublboblr/). Never opens a window: no video, no sound.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
roms="${BUBBLE_ROMS:-$root/target/bubble-bobble/re/mame/roms}"
state="$root/target/bubble-bobble/mame-probe"
snapdir="${SNAPDIR:-$state/snap}"
rm -rf "$snapdir"; mkdir -p "$snapdir" "$state"
SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy DISPLAY= WAYLAND_DISPLAY= \
  timeout $(( ${1:-60} * 3 + 60 )) /usr/games/mame bublboblr -rompath "$roms" \
  -video none -sound none -nothrottle -skip_gameinfo -seconds_to_run "${1:-60}" \
  -cfg_directory "$state/cfg" -nvram_directory "$state/nvram" \
  -snapshot_directory "$snapdir" -autoboot_script "$here/probe.lua" >/dev/null 2>&1 || true
