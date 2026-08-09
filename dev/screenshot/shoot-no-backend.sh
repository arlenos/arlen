#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Screenshot an app's FAILURE path: a production build, served with no backend
# behind it, so every `invoke` throws and every catch runs.
#
# This is the one the other shoot scripts do not do. `shoot.sh` renders a URL and
# `shoot-app.sh` launches the real binary with its backend; both show the app
# working. Nothing showed what a user sees when a daemon is down, and on the night
# of 8 August that turned out to be where the bugs were: a task manager reporting
# 85% memory it never measured, an enabled 07:00 alarm nobody set, a printer list
# offering printers to remove, a week of activity that never happened. Every one
# passed svelte-check.
#
# Usage:
#   dev/screenshot/shoot-no-backend.sh <app> [route] [out.png] [w] [h]
#
#   dev/screenshot/shoot-no-backend.sh clock
#   dev/screenshot/shoot-no-backend.sh settings privacy/physical
#   dev/screenshot/shoot-no-backend.sh knowledge "" out/kg.png 1280 900
#
# Three things this gets right that cost an hour each to learn:
#
#   1. It BUILDS. A dev server sets `import.meta.env.DEV`, and the fixtures are
#      gated on exactly that, so a dev-server render shows the sample data and
#      proves nothing about a real session.
#   2. It uses the EXTENSIONLESS route. `vite preview` will serve
#      `privacy/physical.html` happily and SvelteKit's client router then renders
#      a 404 in the content pane, which reads as a broken page rather than a
#      wrong URL.
#   3. It CHECKS WHAT IT CAPTURED. Rebuilding while a preview server is running
#      takes the server down, and the next shot is a "Connection refused" page -
#      written successfully, exit code 0, and worthless. The screenshot existing
#      is not evidence that the app was in it.
#
# Fourth thing, and it took until 9 August to notice: [w] used to be passed to
# WebKitWebDriver, which accepted it, echoed it back and ignored it. Every shot
# this script produced before then was 372 CSS px wide - a phone - so the desktop
# layout of these panels had never once appeared in one, and nothing in the output
# said so. It now renders through `render-wide.py`, which reaches the width by a
# route that works and refuses rather than quietly hand back a narrow render, so
# [w] finally means what it says. [h] is gone: the viewport height follows from
# the width and asking for one would be a knob that cannot work.

set -euo pipefail

APP="${1:?usage: shoot-no-backend.sh <app> [route] [out.png] [w]}"
ROUTE="${2:-}"
OUT="${3:-dev/screenshot/out/${APP}-no-backend.png}"
W="${4:-1280}"

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
DIR="$ROOT/apps/$APP"
[ -f "$DIR/package.json" ] || { echo "no such app: apps/$APP" >&2; exit 2; }

# A port per app, so two of these can run without fighting over one.
PORT=$(( 5300 + $(printf '%s' "$APP" | cksum | cut -d' ' -f1) % 200 ))

echo "building $APP for production (the fixtures are DEV-gated, so this matters)"
( cd "$DIR" && npm run build >/tmp/shoot-nb-build-$APP.log 2>&1 ) || {
  echo "build failed:" >&2; tail -20 /tmp/shoot-nb-build-$APP.log >&2; exit 1
}

# Clear the port before claiming it. `--strictPort` means our server LOSES this
# race silently: a preview left behind by an earlier run keeps answering, the
# readiness poll below is satisfied by it, and the screenshot is of that server's
# build - the previous one, taken after the fix and looking exactly like a
# verification of it. The old cleanup killed `head -1` of the matches while each
# preview is two processes, so leftovers were the normal state, not the odd one.
pkill -f "vite preview --port $PORT" 2>/dev/null || true
for _ in $(seq 1 20); do
  curl -sf -o /dev/null "http://localhost:$PORT/" || break
  sleep 0.5
done
if curl -sf -o /dev/null "http://localhost:$PORT/"; then
  echo "port $PORT is still serving something this run did not start;" >&2
  echo "refusing rather than photographing it" >&2
  exit 1
fi

( cd "$DIR" && npx vite preview --port "$PORT" --strictPort --outDir build \
    >/tmp/shoot-nb-serve-$APP.log 2>&1 & )
for _ in $(seq 1 30); do
  curl -sf -o /dev/null "http://localhost:$PORT/" && break
  sleep 1
done
curl -sf -o /dev/null "http://localhost:$PORT/" || {
  echo "preview never came up:" >&2; tail -10 /tmp/shoot-nb-serve-$APP.log >&2; exit 1
}

URL="http://localhost:$PORT/${ROUTE#/}"
echo "shooting $URL at ${W}px"
# Yesterday's PNG must not be able to stand in for today's run. render-wide
# refuses by writing nothing, and the "no screenshot was written" check below is
# what turns that into a failure - which it can only do if there is nothing there
# to begin with.
rm -f "$OUT"
# --require-width is the same number, so this script can never again produce what
# it produced for months: a narrow-layout PNG that reads as a desktop one.
# Piped through grep, so the exit code here is grep's; the file check below is
# the real verdict.
xvfb-run -a --server-args="-screen 0 1600x1200x24" \
  python3 "$ROOT/dev/screenshot/render-wide.py" \
    --url "$URL" --out "$OUT" --width "$W" --require-width "$W" \
  2>&1 | grep -v "Gdk-WARNING" || true

# Did we photograph the app, or the preview server's corpse? A page whose text is
# a connection error is not a render of anything.
# Truncated in the shell rather than with `| head -c`, which was a silent
# scuttling of everything below it: under `pipefail`, head closing the pipe kills
# curl with SIGPIPE and the script exits 23 - after the screenshot has been
# written, so it looks like a success. It only bit pages whose HTML is much
# larger than the limit, which is every settings route and none of the small
# apps, so it hid for as long as shots were taken one at a time. The checks it
# skipped are the two this script exists for: was that an error page, and was
# anything written at all.
BODY=$(curl -s "$URL")
BODY=${BODY:0:4000}
# All of them: `npx vite preview` is an npm wrapper plus the node process that
# actually holds the socket, so killing one of the two left the port occupied.
pkill -f "vite preview --port $PORT" 2>/dev/null || true

case "$BODY" in
  *"Connection refused"* | *"ERR_CONNECTION"* )
    echo "the page was an error page, not the app - the shot is worthless" >&2
    exit 1 ;;
esac
if [ ! -s "$OUT" ]; then
  echo "no screenshot was written" >&2
  exit 1
fi

echo "wrote $OUT"
echo "Now LOOK at it. What this cannot check: whether a label that exists is"
echo "visible from the claim it covers, which is how four of that night's fixes"
echo "were wrong on the first attempt."
