#!/usr/bin/env bash
# Test Layer 1b: render a web URL headlessly and capture a screenshot the agent
# (or a human) can actually look at - the "screenshot-verify loop" the docs
# mandate, which never existed before. Drives WebKitWebDriver (the same WebKit
# engine the Tauri apps use, webkit2gtk 2.52.x) under Xvfb, so it needs no
# display and runs in CI or an agent shell.
#
# Usage:
#   dev/screenshot/shoot.sh <url> <out.png> [inject.js] [width] [height]
#
#   <url>        what to load (a dev-server URL, file://, or data: URL)
#   <out.png>    where to write the PNG
#   [inject.js]  optional JS run after load + before the shot (e.g. push state
#                into a store so a component renders)
#   [w] [h]      viewport, default 1280x800
#
# This renders a webview/frontend in isolation - it isolates "does this UI
# actually paint" from the Rust/Tauri backend. The full-app variant (launch the
# real Tauri binary via tauri-driver) is shoot-app.sh.
set -euo pipefail

export SHOOT_URL="${1:?usage: shoot.sh <url> <out.png> [inject.js] [w] [h]}"
export SHOOT_OUT="${2:?usage: shoot.sh <url> <out.png> [inject.js] [w] [h]}"
export SHOOT_INJECT="${3:-}"
export SHOOT_WIDTH="${4:-1280}"
export SHOOT_HEIGHT="${5:-800}"
export SHOOT_PORT=4477
export SHOOT_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# All inputs travel as environment variables (not string-interpolated into the
# inner command) so a URL with spaces/quotes - e.g. a data: URL - is safe.
xvfb-run -a bash -c '
  set -euo pipefail
  WebKitWebDriver --port="$SHOOT_PORT" >/tmp/arlen-wkwd.log 2>&1 &
  wd=$!
  trap "kill $wd 2>/dev/null || true" EXIT
  for _ in $(seq 1 25); do
    curl -s "http://localhost:$SHOOT_PORT/status" >/dev/null 2>&1 && break
    sleep 0.2
  done
  args=(--url "$SHOOT_URL" --out "$SHOOT_OUT" --port "$SHOOT_PORT" --width "$SHOOT_WIDTH" --height "$SHOOT_HEIGHT")
  [ -n "$SHOOT_INJECT" ] && args+=(--inject "$SHOOT_INJECT")
  python3 "$SHOOT_HERE/shoot.py" "${args[@]}"
'
