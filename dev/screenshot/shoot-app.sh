#!/usr/bin/env bash
# Test Layer 1b (full app): launch a REAL Tauri binary via tauri-driver under
# Xvfb and screenshot it. Unlike shoot.sh (a webview URL in isolation), this runs
# the actual app - Rust backend + webview together - so it verifies the whole
# thing (IPC + render), e.g. that terminal command output appears.
#
# Usage:
#   dev/screenshot/shoot-app.sh <app-binary> <out.png> [type-text]
#
#   <app-binary>  a built Tauri binary that serves its frontend from frontendDist
#                 (run the app's `npm run build` then `cargo build` first)
#   <out.png>     where to write the PNG
#   [type-text]   optional text typed into the focused input then submitted with
#                 Enter (e.g. a terminal command), so output renders before the shot
#
# Requirements: tauri-driver (cargo install tauri-driver), WebKitWebDriver, Xvfb.
set -euo pipefail

# Usage: shoot-app.sh <app-binary> [out.png] [type-text] [settle]
# Screenshot mode needs <out.png>. Assert mode (SHOOT_EXEC set) runs a command in
# the terminal and asserts SHOOT_EXPECT renders, with no screenshot - leave out.png
# empty (e.g. `SHOOT_EXEC='echo hi' SHOOT_EXPECT=hi shoot-app.sh <bin>`).
export SHOOT_APP="${1:?usage: shoot-app.sh <app-binary> [out.png] [type-text] [settle]}"
export SHOOT_OUT="${2:-}"
export SHOOT_TYPE="${3:-}"
# Seconds to wait for the app to come up and hydrate before querying the DOM or
# screenshotting. A heavy SvelteKit app under WebKitGTK + Xvfb needs more than the
# 3s default, or `.console` is not mounted yet and the shot races the paint.
export SHOOT_SETTLE="${4:-}"
export SHOOT_PORT=4444
export SHOOT_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export NATIVE="$(command -v WebKitWebDriver || echo /usr/bin/WebKitWebDriver)"

# tauri-driver spawns the app + the native WebKitWebDriver; the python client
# talks to tauri-driver. All inputs travel as env (not interpolated) so a typed
# command with spaces/quotes is safe.
xvfb-run -a --server-args="-screen 0 1280x900x24" bash -c '
  set -euo pipefail
  # A window manager so the WebKit app window holds real keyboard focus; without
  # one, synthetic keystrokes never route to the focusable .console surface (so
  # the assert mode cannot drive the terminal). Best-effort: harmless if absent.
  ob=""
  if command -v openbox >/dev/null 2>&1; then openbox >/tmp/arlen-openbox.log 2>&1 & ob=$!; sleep 1.5; fi
  tauri-driver --port "$SHOOT_PORT" --native-driver "$NATIVE" \
    >/tmp/arlen-tauri-driver.log 2>&1 &
  td=$!
  trap "kill $td ${ob} 2>/dev/null || true" EXIT
  for _ in $(seq 1 50); do
    curl -s "http://localhost:$SHOOT_PORT/status" >/dev/null 2>&1 && break
    sleep 0.2
  done
  args=(--app "$SHOOT_APP" --port "$SHOOT_PORT")
  [ -n "$SHOOT_OUT" ] && args+=(--out "$SHOOT_OUT")
  [ -n "$SHOOT_TYPE" ] && args+=(--type "$SHOOT_TYPE")
  [ -n "$SHOOT_SETTLE" ] && args+=(--settle "$SHOOT_SETTLE")
  [ -n "${SHOOT_GRAB:-}" ] && args+=(--grab-x)
  [ -n "${SHOOT_EXEC:-}" ] && args+=(--exec "$SHOOT_EXEC")
  [ -n "${SHOOT_EXPECT:-}" ] && args+=(--expect "$SHOOT_EXPECT")
  python3 "$SHOOT_HERE/shoot_app.py" "${args[@]}"
'
