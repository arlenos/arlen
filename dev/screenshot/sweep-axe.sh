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

# THE LIST IS THE COVERAGE, so what is missing from it is invisible rather than
# clean. On 5 September a sweep of the mail app reported "0 violations" and had
# swept nothing at all: mail was not here, the app filter matched no entry, and
# the tally printed a clean line for an empty run. Mail, calendar and pdf added
# the same day.
#
# `harness` and `store` are deliberately absent: they are arlen-ui's live work,
# and a shared sweep that goes red on another lane's surface is a sweep somebody
# turns off. `trash-rm` is not an app.
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
  "mail -"
  "calendar -"
  "pdf -"
  "desktop-shell /waypointer"
)

# An app name that matches nothing sweeps nothing and, before this, still printed
# a clean tally - the exact false green the three comments below guard the rest of
# this file against. Refuse instead.
if [ -n "$ONLY" ]; then
  known=0
  for entry in "${SURFACES[@]}"; do
    read -r a _ <<<"$entry"
    [ "$a" = "$ONLY" ] && known=1
  done
  if [ "$known" = 0 ]; then
    echo "sweep-axe.sh: no surface named '$ONLY'. It would sweep nothing and report clean." >&2
    printf '  known:' >&2
    for entry in "${SURFACES[@]}"; do read -r a _ <<<"$entry"; printf ' %s' "$a" >&2; done
    echo >&2
    exit 2
  fi
fi

out=$(mktemp -d)
trap 'rm -rf "$out"; kill -- "-${server:-0}" 2>/dev/null' EXIT

total=0
swept=0
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

  # Assert the page is the app we started, not one still listening from an
  # earlier run. A stale server on a taken port answers `curl` perfectly well,
  # and `--strictPort` only stops the NEW vite - so a whole row of this table
  # was once the previous app's page under this app's name.
  #
  # The expected title is DERIVED from the app's own catalogue rather than kept
  # in a table here: `<prefix>.app.title` is the key every app carries for its
  # window, so the sweep reads what the app says its name is. A first version
  # compared each page to the PREVIOUS one, which catches nothing when a single
  # app is swept - and a guard that passes when there is nothing to compare is
  # the kind that reads as protection and is not.
  want=$(grep -hoE '"[a-z]+\.app\.title": "[^"]+"' \
           "apps/$app/src/lib/i18n/"messages*.ts 2>/dev/null \
         | head -1 | sed 's/.*: "\(.*\)"/\1/')
  # THE TITLE IS SET BY THE APP, so it arrives after hydration and not when the
  # server first answers. `curl -sf /` succeeds as soon as vite serves the shell,
  # which for a heavy app is seconds before its script has run - so a single
  # probe here read an empty title and the sweep refused the surface as somebody
  # else's server. Files dropped out of every run that way while the tally
  # printed a number, which is the same false-clean this file guards elsewhere.
  #
  # So it is asked several times, and the two cases are told apart: NO title is a
  # page that has not come up, a DIFFERENT title is genuinely another app.
  served=""
  for _ in $(seq 1 12); do
    served=$(python3 dev/screenshot/render-wide.py \
      --url "http://localhost:$PORT$route" --out /dev/null --width "$WIDTH" \
      --probe "document.title" 2>/dev/null | tail -1)
    [ -n "$served" ] && break
    sleep 2
  done
  if [ -n "$want" ] && [ "$served" != "$want" ]; then
    if [ -z "$served" ]; then
      why="the page never reported a title, so it did not come up"
    else
      why="port $PORT served \"$served\", not \"$want\" - another server holds it"
    fi
    printf '%-16s %s\n' "$app" "REFUSED: $why"
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
  swept=$((swept + 1))
  printf '%-16s %s\n' "$app" "$(grep -E '^axe:' "$out/$app.axe" || echo 'axe: no result')"
  grep -E '^  [a-z-]+ \(' "$out/$app.axe" | sed 's/^/                 /' || true

  kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
  PORT=$((PORT + 1))
done

echo
# The count of surfaces is part of the result, not a detail: "0 violations" over
# no surfaces and "0 violations" over fifteen are the same sentence and opposite
# facts.
echo "$total violation(s) across $swept surface(s) at ${WIDTH}px"
