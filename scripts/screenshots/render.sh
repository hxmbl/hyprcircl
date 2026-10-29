#!/usr/bin/env bash
# Render the README/docs screenshots of the shipped config + theme.
#
#   scripts/screenshots/render.sh [path/to/hyprcircl] [out-dir]
#
# Two kinds of image, from the same drawing code:
#   live-*.png    the real daemon on a headless sway output, captured by grim
#                 (needs sway + grim; skipped when they're missing)
#   *.png         `hyprcircl preview` frames for states that need input
#                 (hover, open submenus), which headless sway can't inject
#
# A few stand-in commands (scripts/screenshots/stubs) give the status pill
# stable demo values: workspace 2, 64% volume, a fixed clock.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
bin="$(realpath "${1:-$root/target/debug/hyprcircl}")"
out="$(realpath -m "${2:-$root/docs/screenshots}")"
mkdir -p "$out"

export HYPRCIRCL_CONFIG="$root/hyprcircl.toml"
export PATH="$here/stubs:$PATH"
wall="$here/wallpaper.png"

# --- preview frames -------------------------------------------------------
# name            --open   --hover
states=(
  "root           -        0"
  "capture        3        0"
  "workspace      5,4      2"
  "power          6        1"
)
for scale in 1 2; do
  tag=$([ "$scale" = 1 ] && echo 1080p || echo hidpi)
  for s in "${states[@]}"; do
    read -r name open hover <<<"$s"
    args=(--size 1920x1080 --scale "$scale" --center 960,600 --background "$wall" --hover "$hover")
    [ "$open" != - ] && args+=(--open "$open")
    "$bin" preview "$out/$name-$tag.png" "${args[@]}"
  done
done

# --- live daemon in headless sway -----------------------------------------
if ! command -v sway >/dev/null || ! command -v grim >/dev/null; then
  echo "sway/grim not found: skipping live captures" >&2
  exit 0
fi
for scale in 1 2; do
  tag=$([ "$scale" = 1 ] && echo 1080p || echo hidpi)
  run="$(mktemp -d)"
  chmod 700 "$run"
  cat >"$run/sway.conf" <<CONF
output HEADLESS-1 resolution $((1920 * scale))x$((1080 * scale)) scale $scale bg $wall fill
default_border none
CONF
  XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$run/sway.conf" >"$run/sway.log" 2>&1 &
  sway_pid=$!
  for _ in $(seq 100); do
    sock=$(find "$run" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' | head -1)
    [ -n "$sock" ] && break
    sleep 0.1
  done
  [ -n "${sock:-}" ] || { cat "$run/sway.log" >&2; exit 1; }
  XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$(basename "$sock")" GDK_BACKEND=wayland \
    "$bin" >"$run/hyprcircl.log" 2>&1 &
  hc_pid=$!
  sleep 3 # open animation + first bar refresh
  XDG_RUNTIME_DIR="$run" WAYLAND_DISPLAY="$(basename "$sock")" grim "$out/live-root-$tag.png"
  kill "$hc_pid" "$sway_pid" 2>/dev/null || true
  wait "$hc_pid" "$sway_pid" 2>/dev/null || true
  grep -E "CONFIG|THEME|CSS|rror" "$run/hyprcircl.log" >&2 || true
  rm -rf "$run"
done
echo "screenshots in $out"
