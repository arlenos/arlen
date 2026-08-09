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
# What it does NOT get right, found 9 August: [w] and [h] are passed on and the
# browser ignores them. Every shot this script has produced is 372 CSS px wide,
# which is a phone, so the desktop layout of these panels has never appeared in
# one. `shoot.py` now prints the viewport it actually rendered at on every run -
# read that line before drawing a conclusion about spacing or alignment. Set
# SHOOT_REQUIRE_WIDTH to be refused rather than handed a narrow-layout render.

set -euo pipefail

APP="${1:?usage: shoot-no-backend.sh <app> [route] [out.png] [w] [h]}"
ROUTE="${2:-}"
OUT="${3:-dev/screenshot/out/${APP}-no-backend.png}"
W="${4:-1280}"
H="${5:-880}"

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
DIR="$ROOT/apps/$APP"
[ -f "$DIR/package.json" ] || { echo "no such app: apps/$APP" >&2; exit 2; }

# A port per app, so two of these can run without fighting over one.
PORT=$(( 5300 + $(printf '%s' "$APP" | cksum | cut -d' ' -f1) % 200 ))

echo "building $APP for production (the fixtures are DEV-gated, so this matters)"
( cd "$DIR" && npm run build >/tmp/shoot-nb-build-$APP.log 2>&1 ) || {
  echo "build failed:" >&2; tail -20 /tmp/shoot-nb-build-$APP.log >&2; exit 1
}

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
echo "shooting $URL"
"$ROOT/dev/screenshot/shoot.sh" "$URL" "$OUT" "" "$W" "$H" >/dev/null

# Did we photograph the app, or the preview server's corpse? A page whose text is
# a connection error is not a render of anything.
BODY=$(curl -s "$URL" | head -c 4000)
pid=$(pgrep -f "vite preview --port $PORT" | head -1) && [ -n "$pid" ] && kill "$pid" 2>/dev/null || true

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
