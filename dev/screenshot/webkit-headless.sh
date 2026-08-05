#!/usr/bin/env bash
# Drive a frontend under the engine the image actually ships: WebKitGTK on
# Wayland, headless.
#
# The sibling scripts here reach for Xvfb, which is X and is not what any Arlen
# surface runs on; on a Wayland dev box it is often not installed at all, and
# then the whole loop is unavailable exactly when a WebKit-vs-Chromium question
# comes up. Playwright answers "does it work in Chromium", which is a different
# question and has already sent one diagnosis down the wrong path.
#
# So: a headless sway on its own socket (the live session is untouched), and
# WebKitWebDriver inside it. Anything that speaks WebDriver can then drive the
# real engine - execute scripts, read the DOM, click - against a vite dev server
# or a built app.
#
#   dev/screenshot/webkit-headless.sh 4488 &
#   # then POST to http://127.0.0.1:4488/session with browserName MiniBrowser
#
# Requires sway (headless via WLR_BACKENDS) and WebKitWebDriver.
set -euo pipefail

port="${1:-4488}"
runtime="${XDG_RUNTIME_DIR:-/tmp}/arlen-webkit-headless.$$"
mkdir -p "$runtime"

for bin in sway WebKitWebDriver; do
  command -v "$bin" >/dev/null || {
    echo "error: '$bin' not found - the headless WebKit loop needs it." >&2
    echo "  Arch/EndeavourOS: sudo pacman -S sway webkit2gtk-4.1" >&2
    exit 1
  }
done

conf="$runtime/sway.conf"
printf 'exec true\n' > "$conf"

# Its own XDG_RUNTIME_DIR, so the socket cannot collide with the developer's
# session and the compositor dies with this script rather than lingering.
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
  XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY= \
  sway -c "$conf" >"$runtime/sway.log" 2>&1 &
sway_pid=$!
trap 'kill "$sway_pid" 2>/dev/null || true; rm -rf "$runtime"' EXIT

for _ in $(seq 50); do
  [ -S "$runtime/wayland-1" ] && break
  sleep 0.2
done
[ -S "$runtime/wayland-1" ] || { echo "error: headless sway never bound a socket" >&2; exit 1; }

echo "headless WebKit on http://127.0.0.1:$port (wayland socket $runtime/wayland-1)"
XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-1 GDK_BACKEND=wayland \
  exec WebKitWebDriver --port="$port"
