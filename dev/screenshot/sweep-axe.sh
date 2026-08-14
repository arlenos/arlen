#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run axe-core over every app's main surface, in one go, and print the tally.
#
# The kit's own axe gate (`sdk/ui-kit/src/lib/components/a11y.test.ts`) runs the
# primitives under jsdom. This runs the same engine against the real WebKit render
# of a real page, which reaches two things jsdom cannot: anything needing a box
# (colour contrast is ON here, OFF there) and the page AS ASSEMBLED - an app
# composes kit primitives into surfaces the kit never sees.
#
#   dev/screenshot/sweep-axe.sh                # every app at 1280
#   dev/screenshot/sweep-axe.sh 1440           # wider
#   dev/screenshot/sweep-axe.sh 1280 terminal  # one app
#
# A dev server per app, one at a time: they are torn down between runs so a stale
# one cannot answer for the next app - which happened by hand and produced a
# confident report about WebKit's error page. `render-wide.py` refuses a page that
# did not load now, so that failure is loud rather than wrong, and this waits for
# the server to answer before shooting.
#
# Each line is `<app> <route>`; `-` means the app's root.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

WIDTH="${1:-1280}"
ONLY="${2:-}"
PORT=5310

SURFACES=(
  "files -"
  "terminal -"
  "settings -"
  "meetings -"
  "clock -"
  "knowledge -"
  "system-monitor -"
  "text-editor -"
  "viewers -"
  "screenshot -"
  "greeter -"
  "desktop-shell /waypointer"
)

out=$(mktemp -d)
trap 'rm -rf "$out"; kill -- "-${server:-0}" 2>/dev/null' EXIT

total=0
for entry in "${SURFACES[@]}"; do
  read -r app route <<<"$entry"
  [ -n "$ONLY" ] && [ "$ONLY" != "$app" ] && continue
  [ "$route" = "-" ] && route="/"

  # `setsid` so the whole tree gets its own process group: killing the `npm run
  # dev` wrapper leaves the vite child listening, and the next app then shoots
  # whatever the previous one is still serving. Found by finding three of them
  # alive after a sweep.
  setsid bash -c "cd 'apps/$app' && exec npm run dev -- --port $PORT --strictPort" \
    >"$out/$app.log" 2>&1 &
  server=$!

  ready=""
  for _ in $(seq 1 40); do
    sleep 1
    if curl -sf -o /dev/null "http://localhost:$PORT$route"; then ready=1; break; fi
  done
  if [ -z "$ready" ]; then
    printf '%-16s %s\n' "$app" "SKIPPED: the dev server never answered"
    kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
    PORT=$((PORT + 1))
    continue
  fi

  python3 dev/screenshot/render-wide.py \
    --url "http://localhost:$PORT$route" \
    --out "$out/$app.png" --width "$WIDTH" --axe --settle 3 \
    >"$out/$app.axe" 2>&1
  n=$(grep -cE '^  [a-z-]+ \(' "$out/$app.axe" || true)
  total=$((total + n))
  printf '%-16s %s\n' "$app" "$(grep -E '^axe:' "$out/$app.axe" || echo 'axe: no result')"
  grep -E '^  [a-z-]+ \(' "$out/$app.axe" | sed 's/^/                 /' || true

  kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
  PORT=$((PORT + 1))
done

echo
echo "$total violation(s) across the surfaces swept at ${WIDTH}px"
