#!/usr/bin/env bash
# Full-app headless screenshot via a headless sway + grim (Wayland-native).
# The WebDriver path (shoot-app.sh) hangs capturing the wry webview, and plain
# Xvfb has no window manager so the window never maps; a headless sway DOES map
# + composite the real Tauri window, and grim captures it. This is the working
# full-app pixel loop: the REAL binary + its backend, rendered, captured.
#
# Usage: dev/screenshot/shoot-sway.sh <app-binary> <out.png> [vite-dir] [type-text]
#   <vite-dir>   if set, runs `npm run dev` there first (the debug Tauri binary
#                loads its devUrl, e.g. http://localhost:1425, not frontendDist),
#                so the real frontend is served. Omit for a release/bundled binary.
#   [type-text]  if set AND wtype is installed, types it + Enter into the focused
#                window before the shot (to run a command).
# Requires: sway, grim (both present); wtype optional for typing.
set -uo pipefail

BIN="${1:?usage: shoot-sway.sh <app-binary> <out.png> [vite-dir] [type-text]}"
OUT="${2:?usage: shoot-sway.sh <app-binary> <out.png> [vite-dir] [type-text]}"
VITE_DIR="${3:-}"
TYPE_TEXT="${4:-}"
# ALWAYS a fresh private runtime dir - never inherit the real session's
# XDG_RUNTIME_DIR (e.g. /run/user/$UID). If inherited, the headless sway socket
# sits beside the REAL compositor's socket and grim can grab the actual desktop
# (a privacy leak). A fresh dir holds only the headless sway's socket.
export XDG_RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arlen-shot-rt.XXXXXX")"
chmod 700 "$XDG_RUNTIME_DIR"
cleanup() { rm -rf "$XDG_RUNTIME_DIR" 2>/dev/null; }
trap cleanup EXIT

vite=""
if [ -n "$VITE_DIR" ]; then
  ( cd "$VITE_DIR" && npm run dev >/tmp/arlen-shot-vite.log 2>&1 ) &
  vite=$!
  for _ in $(seq 1 90); do curl -s http://localhost:1425 >/dev/null 2>&1 && break; sleep 1; done
fi

cfg="$(mktemp)"
printf 'output HEADLESS-1 resolution 1280x800\nexec env GDK_BACKEND=wayland %q >/tmp/arlen-shot-app.log 2>&1\n' "$BIN" > "$cfg"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$cfg" >/tmp/arlen-shot-sway.log 2>&1 &
sway_pid=$!
sleep 26
WD="$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1)"
if [ -z "$WD" ]; then
  echo "no headless sway socket in $XDG_RUNTIME_DIR - refusing to grab (would capture the real display)" >&2
  kill "$sway_pid" 2>/dev/null
  [ -n "$vite" ] && kill "$vite" 2>/dev/null
  exit 1
fi
if [ -n "$TYPE_TEXT" ] && command -v wtype >/dev/null 2>&1; then
  WAYLAND_DISPLAY="$WD" wtype "$TYPE_TEXT" -k Return >/tmp/arlen-shot-type.log 2>&1
  sleep 3
fi
WAYLAND_DISPLAY="$WD" grim "$OUT"; rc=$?
kill "$sway_pid" 2>/dev/null
[ -n "$vite" ] && kill "$vite" 2>/dev/null
echo "shot rc=$rc -> $OUT"
exit $rc
