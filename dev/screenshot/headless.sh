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
    # Kill AND wait: the display goes with xvfb-run the moment this returns, and a
    # WM shutting down against a vanishing server logs noise that reads like a
    # failed shot.
    if [ -n "$ob" ]; then kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null; fi
    exit $rc
  ' _ "$ROOT" "$@" 2>&1 | grep -v "Gdk-WARNING" || true
