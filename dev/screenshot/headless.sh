#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run `render-wide.py` off-screen. One place owns the recipe, and it is three
# things, all of which have been got wrong at least once:
#
#   - an Xvfb with a real screen size, since `xvfb-run -a` alone gives 640x480;
#   - the host session cut off (`-u WAYLAND_DISPLAY`, `GDK_BACKEND=x11`), or GTK 4
#     prefers the inherited Wayland display and the window opens on the
#     developer's actual screen, interrupting them and shooting the wrong display;
#   - a window manager, or `fullscreen()` is never granted, the surface stays at
#     its unmapped 200px, and render-wide refuses with no file.
#
# `shoot-no-backend.sh` has carried all three since 16 August. This exists because
# that script builds for PRODUCTION and so refuses a `?locale=` route, and its
# own advice for that case printed a bare `python3 render-wide.py` - which is how
# a run on 25 August ended up drawing on the developer's session. The advice now
# names this.
#
# Run: dev/screenshot/headless.sh --url http://localhost:5271/?locale=de --out shot.png --width 1280
# Every argument is passed through to render-wide.py untouched.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

command -v xvfb-run >/dev/null 2>&1 || {
  echo "no xvfb-run. Arch/EndeavourOS: sudo pacman -S xorg-server-xvfb" >&2
  exit 2
}

# THE DISPLAY NUMBER IS OURS, and `-a` is not enough. `xvfb-run -a` looks for a
# free display and then creates its lock, and those two steps are not atomic - so
# two renders starting at the same moment can choose the same `:N`, and what
# happens then is not an error: the second Xvfb loses, openbox and WebKit attach
# to a server that is going away, and BOTH runs hang with no output. Measured on
# 10 September, when a full sweep and another lane's walk deadlocked against each
# other for half an hour and every app after the first reported "the probe cannot
# see its own control" - which reads as a broken probe and is a stuck display.
#
# Derived from the pid the same way `shoot.sh` derives its driver port and
# `sweep-render-all.sh` its app ports, then walked upward on collision, because a
# derived number is only unlikely to clash rather than certain not to. The `-a`
# fallback stays for the case where every number tried is taken.
_display_base=$(( 90 + ($$ % 60) ))
_xvfb_rc=0
for _try in 0 1 2 3 4 5 6 7; do
  _n=$(( _display_base + _try ))
  [ -e "/tmp/.X${_n}-lock" ] && continue
  set +e
  xvfb-run -n "$_n" --server-args="-screen 0 ${SHOOT_SCREEN_W:-1600}x${SHOOT_SCREEN_H:-1200}x24" \
    env -u WAYLAND_DISPLAY GDK_BACKEND=x11 bash -c '
    ob=""
    if command -v openbox >/dev/null 2>&1; then
      openbox >/tmp/arlen-headless-openbox.log 2>&1 &
      ob=$!
      sleep 1.5
    fi
    root="$1"; shift
    rc=0
    python3 "$root/dev/screenshot/render-wide.py" "$@" || rc=$?
    # Kill AND wait: the display goes with xvfb-run the moment this returns, and a
    # WM shutting down against a vanishing server logs noise that reads like a
    # failed shot.
    if [ -n "$ob" ]; then kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null; fi
    exit $rc
  ' _ "$ROOT" "$@" 2>&1 | grep -v "Gdk-WARNING"
  _xvfb_rc=${PIPESTATUS[0]}
  set -e
  # 2 is xvfb-run's "the server would not start", which on a taken display is
  # what a collision looks like. Anything else is the render's own answer and is
  # passed straight out - a failing probe must not be retried into a pass.
  # An `if`, not `[ ] && exit`: under `set -e` a trailing `&&` list that comes out
  # false is a failing command in its own right, and the retry would have exited
  # the script on the one path it exists to survive.
  if [ "$_xvfb_rc" -ne 2 ]; then
    exit "$_xvfb_rc"
  fi
done

# Every derived number was taken. Fall back to letting xvfb-run choose, which is
# where this started: better a small race than no render at all.
xvfb-run -a --server-args="-screen 0 ${SHOOT_SCREEN_W:-1600}x${SHOOT_SCREEN_H:-1200}x24" \
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 bash -c '
    ob=""
    if command -v openbox >/dev/null 2>&1; then
      openbox >/tmp/arlen-headless-openbox.log 2>&1 &
      ob=$!
      sleep 1.5
    fi
    root="$1"; shift
    rc=0
    python3 "$root/dev/screenshot/render-wide.py" "$@" || rc=$?
    if [ -n "$ob" ]; then kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null; fi
    exit $rc
  ' _ "$ROOT" "$@" 2>&1 | grep -v "Gdk-WARNING" || true
