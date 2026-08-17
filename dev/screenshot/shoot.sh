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
# A CSS selector clicked after load and before the injection, for anything the
# page only shows once something is opened.
export SHOOT_OPEN="${SHOOT_OPEN:-}"
# The viewport. Every run prints the size it actually rendered at; set
# SHOOT_REQUIRE_WIDTH to refuse rather than hand back a narrow-layout shot.
export SHOOT_WIDTH="${4:-1280}"
export SHOOT_HEIGHT="${5:-800}"

# The Xvfb screen, which has to be at least the viewport - a window cannot be
# wider than the display it is on.
#
# THIS WAS THE WHOLE BUG, and it was blamed on the wrong component for a week.
# `xvfb-run -a` with no server args gives a **640x480** screen, so every request
# for 1280x800 came back clamped, and the note at the top of shoot.py concluded
# the browser "answers with a yes and ignores" the resize. It does not. Measured
# side by side, same probe, same driver:
#
#   xvfb-run -a                          window/rect 1280 -> innerWidth 640
#   xvfb-run -a -s "-screen 0 1920x1200" window/rect 1280 -> innerWidth 1280
#
# `shoot-app.sh` had passed a screen size all along, which is why full-app shots
# looked right and these did not. Sized from the request with room for window
# furniture, so a caller asking for 1700px gets a screen that can hold it.
export SHOOT_SCREEN_W=$(( SHOOT_WIDTH + 200 ))
export SHOOT_SCREEN_H=$(( SHOOT_HEIGHT + 200 ))
export SHOOT_PORT=4477
export SHOOT_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# All inputs travel as environment variables (not string-interpolated into the
# inner command) so a URL with spaces/quotes - e.g. a data: URL - is safe.
# Xvfb is the one non-obvious dependency of the whole screenshot loop, and
# without it these scripts died with a bare "xvfb-run: command not found" that
# says nothing about what to install. The screenshot loop is mandatory for any
# UI change, so failing to run it must be actionable, never cryptic.
require_xvfb() {
  local bin="$1"
  command -v "$bin" >/dev/null 2>&1 && return 0
  echo "error: '$bin' not found - the headless screenshot loop needs Xvfb." >&2
  echo "  Arch/EndeavourOS: sudo pacman -S xorg-server-xvfb" >&2
  echo "  Debian/Ubuntu:    sudo apt install xvfb" >&2
  echo "  Fedora:           sudo dnf install xorg-x11-server-Xvfb" >&2
  exit 127
}
require_xvfb xvfb-run

# The host session is cut off here, the same way `shoot-compositor.sh` does it for
# the nested compositor. `xvfb-run` sets DISPLAY and nothing else, so an inherited
# WAYLAND_DISPLAY stays valid and a toolkit that prefers Wayland renders against the
# developer's real session instead of this Xvfb. On 15 August that let an app driven
# through the sibling app-harness capture the real desktop; nothing here captures, but
# a shot that silently came from the wrong display is not a shot of anything.
xvfb-run -a --server-args="-screen 0 ${SHOOT_SCREEN_W}x${SHOOT_SCREEN_H}x24" bash -c '
  unset WAYLAND_DISPLAY
  export GDK_BACKEND=x11
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
  [ -n "${SHOOT_OPEN:-}" ] && args+=(--open "$SHOOT_OPEN")
  [ -n "${SHOOT_REQUIRE_WIDTH:-}" ] && args+=(--require-width "$SHOOT_REQUIRE_WIDTH")
  python3 "$SHOOT_HERE/shoot.py" "${args[@]}"
'
