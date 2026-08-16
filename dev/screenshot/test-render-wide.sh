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
