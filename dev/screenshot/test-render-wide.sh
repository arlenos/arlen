#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# One control for render-wide.py: does `--probe` report the page AFTER `--open`
# clicked it?
#
# It did not, from the day both flags existed until 16 August. `--probe` ran at
# load-finished and returned before the click chain started, so a probe combined
# with a click printed the BEFORE state, said nothing about that, and exited 0.
#
# What that costs is not a missing answer, it is a wrong one that looks right.
# Asking the knowledge app what it says when Pause is clicked with no daemon
# behind it answered "no alert" - a control that fails silently, a real bug, and
# a fix already half-written for it. The app was correct: it reverts the switch
# and says "Recording could not be paused, so it is still running." The probe was
# just one step early.
#
# So the assertion here is specifically the ordering, on a page whose click
# changes text and nothing else. It fails loudly if a probe ever runs first
# again.
#
# Needs a display; run it as-is and it brings its own Xvfb.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

# A page where the only difference between before and after is the click.
PAGE='data:text/html,<button id=b onclick="document.title=String(42)">go</button>'

out=$(xvfb-run -a --server-args="-screen 0 1600x1200x24" \
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 bash -c '
    openbox >/dev/null 2>&1 &
    ob=$!
    sleep 1.5
    python3 dev/screenshot/render-wide.py --url "$1" --out /dev/null \
      --width 1280 --settle 1 --open "#b" --probe "document.title"
    rc=$?
    kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null
    exit $rc
  ' _ "$PAGE" 2>/dev/null) || { echo "render-wide exited non-zero" >&2; exit 1; }

# Before the click the title is the data: URL; after it, 42. Anything else means
# the probe measured the wrong moment.
case "$out" in
  *42*) echo "ok: --probe sees the page after --open clicked it" ;;
  *)    echo "FAIL: probe returned ${out@Q}, which is the pre-click page." >&2
        echo "--probe is running before --open again." >&2
        exit 1 ;;
esac

# --- the stub host, both directions ---------------------------------------
#
# `--stub-host` is what lets this harness render the failure a person actually
# meets: the Tauri runtime present, the command refusing. Without it the only
# reachable path is "no runtime", which for a Tauri app is the browser preview,
# where every fixture guard fires. The flag going quietly inert would not fail a
# sweep - it would just fill the sweep with the wrong picture again.

probe() {  # $1 = flags, $2 = expression
  xvfb-run -a --server-args="-screen 0 1600x1200x24" \
    env -u WAYLAND_DISPLAY GDK_BACKEND=x11 bash -c '
      openbox >/dev/null 2>&1 &
      ob=$!
      sleep 1.5
      python3 dev/screenshot/render-wide.py --url "data:text/html,<p>x" --out /dev/null \
        --width 1280 --settle 1 $1 --probe "$2"
      rc=$?
      kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null
      exit $rc
    ' _ "$1" "$2" 2>/dev/null
}

with=$(probe "--stub-host" "typeof window.__TAURI_INTERNALS__.invoke")
case "$with" in
  *function*) echo "ok: --stub-host installs a runtime before the page runs" ;;
  *) echo "FAIL: with --stub-host the page saw ${with@Q}, not a callable invoke." >&2
     exit 1 ;;
esac

without=$(probe "" "typeof window.__TAURI_INTERNALS__")
case "$without" in
  *undefined*) echo "ok: without it the page has no runtime, as before" ;;
  *) echo "FAIL: a plain run already carries a runtime (${without@Q}), so the" >&2
     echo "two modes are the same picture." >&2
     exit 1 ;;
esac

# --- the flat-frame warning, both directions ------------------------------
#
# A frame of one colour is a shot of nothing, and one arrived on 24 August that
# read as "this app renders nothing when its daemon fails" - it was the harness.
# Silent, that teaches something false about every app a sweep touches.

shoot() {  # $1 = page, $2 = out
  xvfb-run -a --server-args="-screen 0 1600x1200x24" \
    env -u WAYLAND_DISPLAY GDK_BACKEND=x11 bash -c '
      openbox >/dev/null 2>&1 &
      ob=$!
      sleep 1.5
      python3 dev/screenshot/render-wide.py --url "$1" --out "$2" --width 800 --settle 1
      rc=$?
      kill "$ob" 2>/dev/null; wait "$ob" 2>/dev/null
      exit $rc
    ' _ "$1" "$2" 2>/dev/null
}

flat=$(shoot 'data:text/html,<body style=background:%23111>' /tmp/render-wide-flat.png)
case "$flat" in
  *FLAT*) echo "ok: a frame with no second colour says so" ;;
  *) echo "FAIL: a single-colour frame was written without a word about it." >&2
     exit 1 ;;
esac

full=$(shoot 'data:text/html,<body style=background:%23111><p style=color:%23fff>text' /tmp/render-wide-full.png)
case "$full" in
  *FLAT*) echo "FAIL: a page with text was called flat, so the check cries wolf." >&2
          exit 1 ;;
  *) echo "ok: a frame with content is not called flat" ;;
esac
rm -f /tmp/render-wide-flat.png /tmp/render-wide-full.png
