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

export SHOOT_APP="${1:?usage: shoot-app.sh <app-binary> <out.png> [type-text]}"
export SHOOT_OUT="${2:?usage: shoot-app.sh <app-binary> <out.png> [type-text]}"
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
xvfb-run -a bash -c '
  set -euo pipefail
  tauri-driver --port "$SHOOT_PORT" --native-driver "$NATIVE" \
    >/tmp/arlen-tauri-driver.log 2>&1 &
  td=$!
  trap "kill $td 2>/dev/null || true" EXIT
  for _ in $(seq 1 50); do
    curl -s "http://localhost:$SHOOT_PORT/status" >/dev/null 2>&1 && break
    sleep 0.2
  done
  args=(--app "$SHOOT_APP" --out "$SHOOT_OUT" --port "$SHOOT_PORT")
  [ -n "$SHOOT_TYPE" ] && args+=(--type "$SHOOT_TYPE")
  [ -n "$SHOOT_SETTLE" ] && args+=(--settle "$SHOOT_SETTLE")
  [ -n "${SHOOT_GRAB:-}" ] && args+=(--grab-x)
  python3 "$SHOOT_HERE/shoot_app.py" "${args[@]}"
'
