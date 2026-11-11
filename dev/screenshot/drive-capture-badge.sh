#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Does a screen capture light the top bar's capture badge, end to end?
#
# WHY THIS EXISTS. The badge reads `capture_status`, the shell fills that from
# the compositor's `capture_started` and `capture_stopped` on the overlay
# protocol, and the compositor sends those for whoever opens a capture session.
# Three programs and a protocol version sit between a capture and the light, and
# a unit test on any one of them says nothing about the chain: a shell that still
# binds version 2 passes every test it has and never hears of a capture.
#
# WHAT IT DOES. The compositor runs nested under Xvfb, the shell's debug binary
# runs inside it against a dev server, and `grim` is the capturing client. A
# screenshot is a session of a few milliseconds, so the picture that takes it
# cannot show its own light; the shell keeps a capture shown for two seconds for
# exactly that reason, and the SECOND picture, taken inside those two seconds,
# is the one that shows the badge. A third, taken after the floor has run out,
# shows it gone again.
#
#   before.png   no capture has happened yet
#   lit.png      taken inside the floor of the capture `before.png` was
#   after.png    taken once both floors have run out
#
# It asserts that the bar differs between `before` and `lit` and is the same
# again in `after`. Whether the badge
# reads right is for the person running this to look at: the three pictures are
# left in the output directory.
#
# Run: dev/screenshot/drive-capture-badge.sh [out-dir]
# Needs: the compositor fork built at $ARLEN_COMPOSITOR (default
# ~/Repositories/compositor, `cargo build`), the shell's debug binary
# (`cargo build --manifest-path apps/desktop-shell/src-tauri/Cargo.toml`), grim
# and ImageMagick.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="$root/apps/desktop-shell"
out="${1:-$(mktemp -d)}"
comp="${ARLEN_COMPOSITOR:-$HOME/Repositories/compositor}/target/debug/cosmic-comp"
shell_bin="$root/target/debug/arlen-desktop-shell"
port=1420   # the debug binary loads `devUrl`, which names this port

# shellcheck source=dev/screenshot/lib/preview.sh
. "$here/lib/preview.sh"
# shellcheck source=dev/screenshot/lib/wait.sh
. "$here/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/own-display.sh
. "$here/lib/own-display.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$here/lib/fresh.sh"

if [ "${1:-}" = "--inside" ]; then
  # Inside the Xvfb: DISPLAY is the private one.
  out="$2"
  cfg="$(mktemp -d)"
  log="$out/compositor.log"
  mkdir -p "$cfg/arlen"
  # grim is refused while the sensing master switch is off; set here so the run
  # does not depend on the host's switch.
  printf 'screen_capture = true\n' > "$cfg/arlen/sensing.toml"
  export XDG_CONFIG_HOME="$cfg"
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
  inner_cleanup() {
    kill "${shell_pid:-}" "${comp_pid:-}" 2>/dev/null
    wait 2>/dev/null
    rm -rf "$cfg"
  }
  trap inner_cleanup EXIT

  env -u WAYLAND_DISPLAY "$comp" > "$log" 2>&1 &
  comp_pid=$!
  wl=""
  for _ in $(seq 1 60); do
    kill -0 "$comp_pid" 2>/dev/null || { echo "FAIL: the compositor exited; see $log" >&2; exit 1; }
    wl="$(grep -oE 'wayland-[0-9]+' "$log" | head -1 || true)"
    [ -n "$wl" ] && [ -S "$XDG_RUNTIME_DIR/$wl" ] && break
    wl=""
    sleep 0.5
  done
  [ -n "$wl" ] || { echo "FAIL: no Wayland socket; see $log" >&2; exit 1; }

  WAYLAND_DISPLAY="$wl" GDK_BACKEND=wayland "$shell_bin" > "$out/shell.log" 2>&1 &
  shell_pid=$!
  bound=0
  for _ in $(seq 1 120); do
    kill -0 "$shell_pid" 2>/dev/null || { echo "FAIL: the shell exited; see $out/shell.log" >&2; exit 1; }
    grep -q "arlen_shell_overlay_v1 global bound" "$out/shell.log" && { bound=1; break; }
    sleep 0.5
  done
  [ "$bound" = 1 ] || { echo "FAIL: the shell never bound the overlay global; see $out/shell.log" >&2; exit 1; }
  # Bound is not drawn: give the bar time to mount and the badge its first read.
  sleep "${CAPTURE_BADGE_SETTLE:-12}"

  shoot() { WAYLAND_DISPLAY="$wl" grim "$1" || { echo "FAIL: grim could not capture $1" >&2; exit 1; }; }
  shoot "$out/before.png"
  sleep 0.8
  shoot "$out/lit.png"
  sleep 4
  shoot "$out/after.png"
  exit 0
fi

command -v grim >/dev/null || { echo "grim is not installed" >&2; exit 2; }
command -v magick >/dev/null || { echo "ImageMagick (compare) is not installed" >&2; exit 2; }
[ -x "$comp" ] || { echo "no compositor at $comp - build the fork first" >&2; exit 2; }
[ -x "$shell_bin" ] || { echo "no shell binary at $shell_bin - cargo build it first" >&2; exit 2; }
require_fresh "$shell_bin" "$app/src-tauri/src" || exit 2
mkdir -p "$out"

cleanup() {
  stop_preview
  return 0
}
trap cleanup EXIT

start_dev "$app" "$port" || exit 2
wait_for_http "http://localhost:$port/" || exit 1

own_display "-screen 0 1920x1080x24" bash "$0" --inside "$out" || exit 1

# The bar is the top strip. A capture badge that lit changes it; nothing else in
# the strip moves inside a second, bar a clock that turns over a minute, which
# would make this pass for the wrong reason once an hour and is why the pictures
# are still to be looked at.
strip() { magick "$1" -crop x48+0+0 +repage "$2"; }
strip "$out/before.png" "$out/before-bar.png"
strip "$out/lit.png" "$out/lit-bar.png"
strip "$out/after.png" "$out/after-bar.png"
# `compare` exits 1 when the pictures differ, which is the answer and not a
# failure, so its status is not read: only the count it prints is.
differing() { { compare -metric AE "$1" "$2" null: 2>&1 || true; } | grep -oE '^[0-9]+' | head -1; }
changed="$(differing "$out/before-bar.png" "$out/lit-bar.png")"
gone="$(differing "$out/before-bar.png" "$out/after-bar.png")"
echo "bar pixels that differ, before against lit:   $changed"
echo "bar pixels that differ, before against after: $gone"
echo "pictures in $out"
if [ "${changed:-0}" -eq 0 ] || [ "${gone:-1}" -ne 0 ]; then
  echo "FAIL: the bar should differ inside a capture's floor and be back after it" >&2
  exit 1
fi
echo "ok: the bar changed inside the capture's floor and is back after it; look at lit-bar.png for what it says"
