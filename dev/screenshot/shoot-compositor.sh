#!/usr/bin/env bash
# Headless full-app screenshot under the REAL Arlen compositor. Runs cosmic-comp
# nested in Xvfb (its winit X11 backend), launches a Wayland client under it, and
# grim-captures the composited output.
#
# Unlike shoot-app.sh (X11 / WebKitWebDriver, no Wayland) this hosts genuine
# Wayland clients - the shell's wlr-layer-shell topbar, the Tauri apps - so it is
# the path for verifying shell + compositor UI that the webview-only harness
# cannot reach (the top bar, window decorations, cross-app focus).
#
# Usage:
#   dev/screenshot/shoot-compositor.sh <out.png> <client-cmd> [args...]
#
#   <out.png>     where to write the screenshot
#   <client-cmd>  the Wayland client to launch; it is run with WAYLAND_DISPLAY set
#                 to the compositor's socket and DISPLAY cleared
#
# Env:
#   COMPOSITOR_PATH   the compositor repo (default ~/Repositories/compositor)
#   SHOOT_SETTLE      seconds to wait for the client to render (default 5)
#   SHOOT_DISPLAY     the Xvfb display to use (default :99)
#   SHOOT_CLIENT_LOG  capture the client's stdout/stderr here (default /dev/null);
#                     set it to a file to debug why a client did not render
#
# Requirements: Xvfb, grim, and a built cosmic-comp at
# $COMPOSITOR_PATH/target/debug/cosmic-comp.
set -euo pipefail

OUT="${1:?usage: shoot-compositor.sh <out.png> <client-cmd> [args...]}"
shift
[ "$#" -ge 1 ] || { echo "usage: shoot-compositor.sh <out.png> <client-cmd> [args...]" >&2; exit 2; }

COMPOSITOR_PATH="${COMPOSITOR_PATH:-$HOME/Repositories/compositor}"
CC_BIN="$COMPOSITOR_PATH/target/debug/cosmic-comp"
[ -x "$CC_BIN" ] || { echo "no cosmic-comp at $CC_BIN (build it, or set COMPOSITOR_PATH)" >&2; exit 1; }

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
SETTLE="${SHOOT_SETTLE:-5}"
DISP="${SHOOT_DISPLAY:-:99}"
LOG="$(mktemp)"

cleanup() {
  kill "${CLIENT_PID:-}" "${CC_PID:-}" "${XVFB_PID:-}" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -f "/tmp/.X${DISP#:}-lock" "$LOG" 2>/dev/null || true
}
trap cleanup EXIT

rm -f "/tmp/.X${DISP#:}-lock"
Xvfb "$DISP" -screen 0 1920x1080x24 >/dev/null 2>&1 &
XVFB_PID=$!
sleep 2

DISPLAY="$DISP" "$CC_BIN" >"$LOG" 2>&1 &
CC_PID=$!

# cosmic-comp picks its own socket name (wayland-N, ignoring WAYLAND_DISPLAY); it
# logs "Listening on \"wayland-N\"". Parse it rather than guess, then wait for the
# socket file to actually exist.
WL=""
for _ in $(seq 1 40); do
  WL="$(grep -oE 'wayland-[0-9]+' "$LOG" | head -1 || true)"
  [ -n "$WL" ] && [ -S "$XDG_RUNTIME_DIR/$WL" ] && break
  sleep 0.5
done
if [ -z "$WL" ] || [ ! -S "$XDG_RUNTIME_DIR/$WL" ]; then
  echo "cosmic-comp did not come up on $DISP; last log lines:" >&2
  tail -20 "$LOG" >&2
  exit 1
fi
echo "compositor up on $WL (display $DISP)"

CLIENT_LOG="${SHOOT_CLIENT_LOG:-/dev/null}"
WAYLAND_DISPLAY="$WL" DISPLAY="" "$@" >"$CLIENT_LOG" 2>&1 &
CLIENT_PID=$!
sleep "$SETTLE"

WAYLAND_DISPLAY="$WL" grim "$OUT"
echo "wrote $OUT"
