#!/usr/bin/env bash
# Runs bublboblr headless in MAME for SECONDS with reglog.lua (env as there).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
roms="${BUBBLE_ROMS:-$root/target/bubble-bobble/re/mame/roms}"
state="$root/target/bubble-bobble/mame-sound"
mkdir -p "$state"
SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy DISPLAY= WAYLAND_DISPLAY= \
  timeout $(( ${1:-60} * 3 + 60 )) /usr/games/mame bublboblr -rompath "$roms" \
  -video none -sound none -nothrottle -skip_gameinfo -seconds_to_run "${1:-60}" \
  -cfg_directory "$state/cfg" -nvram_directory "$state/nvram" \
  -autoboot_script "$here/reglog.lua" >/dev/null 2>&1 || true
