#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Every app, every surface worth rendering, through the four render probes.
#
# WHY THIS EXISTS AND `sweep-render.sh` IS NOT ENOUGH. That one takes a base URL
# and a list of routes, so what it covers is whatever somebody typed on the day.
# On 5 September a night of sweeping found eleven real defects and left no record
# of WHICH surfaces had been looked at - so the next person cannot tell a clean
# app from an app nobody pointed at. `sweep-axe.sh` learnt this first and its
# header says it plainly: the list IS the coverage, and what is missing from it is
# invisible rather than clean. This is that list for the render probes.
#
#   dev/screenshot/sweep-render-all.sh            # everything, German
#   dev/screenshot/sweep-render-all.sh en         # everything, English
#   dev/screenshot/sweep-render-all.sh de calendar  # one app
#
# THE SELECTORS COST REAL WORK TO FIND and that is half the value here. Eleven of
# the thirteen apps keep their content behind tabs, sidebars and popovers, so a
# route walk reads landing pages - and a landing page is the one surface somebody
# looked at while building it. Every `::selector` below was read off a rendered
# DOM, not guessed.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

LOCALE="${1:-de}"
ONLY="${2:-}"
PORT=6100

# `<app> <spec> [spec...]`, where a spec is a route optionally followed by
# `::selector` - the thing to click before the probes run.
#
# NOT HERE, deliberately: `harness` and `store` are arlen-ui's live surfaces, and
# a shared sweep that goes red on another lane's work is a sweep somebody turns
# off. `desktop-shell` is here because its three windows are the surfaces a person
# sees most.
SURFACES=(
  "calendar / /::.seg-pill:nth-of-type(2) /::.seg-pill:nth-of-type(3) /::.seg-pill:nth-of-type(4) /::.seg-pill:nth-of-type(5) /::#cal-new-event"
  "clock / /::#chrome-add /::#tab-timers /::#tab-focus /::#tab-stopwatch /::#tab-world"
  "desktop-shell / /consent /waypointer"
  "files / /::[data-place=recent] /::[data-place=trash]"
  "greeter /"
  "knowledge / /::button[data-place=projects] /::button[data-place=library] /::button[data-place=searches]"
  "mail / /::.row /::#folder-sent /::#folder-drafts /::#folder-archive /::#folder-trash"
  "meetings / /capture /meeting/abc"
  "pdf /"
  "screenshot /"
  "settings / /keyboard/shortcuts /keyboard/shortcuts::[data-action=add-custom] /keyboard/shortcuts::[data-action=reset-all]"
  "system-monitor / /::#tab-performance"
  "terminal / /::#terminal-history-open /::#terminal-new-session"
  "text-editor / /::.trigger"
  "viewers / /?demo=image /?demo=video"
)

# An app name that matches nothing sweeps nothing and would otherwise print a
# tidy empty run - the false clean this whole family of scripts keeps finding its
# way back into. Refuse instead.
if [ -n "$ONLY" ]; then
  known=0
  for entry in "${SURFACES[@]}"; do
    read -r a _ <<<"$entry"
    [ "$a" = "$ONLY" ] && known=1
  done
  if [ "$known" = 0 ]; then
    echo "sweep-render-all.sh: no app named '$ONLY'." >&2
    printf '  known:' >&2
    for entry in "${SURFACES[@]}"; do read -r a _ <<<"$entry"; printf ' %s' "$a" >&2; done
    echo >&2
    exit 2
  fi
fi

swept=0
fail=0
for entry in "${SURFACES[@]}"; do
  read -r app specs <<<"$entry"
  [ -n "$ONLY" ] && [ "$ONLY" != "$app" ] && continue

  # `setsid` so the whole tree gets its own process group: killing the `npm run
  # dev` wrapper leaves the vite child listening, and the next app is then swept
  # against whatever the previous one is still serving.
  setsid bash -c "cd 'apps/$app' && exec npm run dev -- --port $PORT --strictPort" \
    >"/tmp/sweep-render-all-$app.log" 2>&1 &
  server=$!

  ready=""
  for _ in $(seq 1 45); do
    sleep 1
    if curl -sf -o /dev/null "http://localhost:$PORT/"; then ready=1; break; fi
  done

  # A SERVER THAT DIED IS NOT A PAGE THAT DID NOT COME UP. `--strictPort` makes
  # vite exit when the port is taken, and a server left behind by a killed run
  # answers `curl` perfectly well - so the sweep would go on to measure somebody
  # else's app under this app's name. Same check `sweep-axe.sh` gained today.
  if ! kill -0 "$server" 2>/dev/null; then
    printf '%-16s %s\n' "$app" "REFUSED: this run's dev server exited - port $PORT is held by an earlier one"
    grep -m1 -E 'EADDRINUSE|already in use' "/tmp/sweep-render-all-$app.log" | sed 's/^ *//; s/^/                 /'
    fail=1
    PORT=$((PORT + 1))
    continue
  fi
  if [ -z "$ready" ]; then
    printf '%-16s %s\n' "$app" "REFUSED: the dev server never answered"
    kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
    fail=1
    PORT=$((PORT + 1))
    continue
  fi

  echo "== $app"
  # shellcheck disable=SC2086
  "$(dirname "${BASH_SOURCE[0]}")/sweep-render.sh" "http://localhost:$PORT" "$LOCALE" $specs || fail=1
  swept=$((swept + 1))

  kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
  PORT=$((PORT + 1))
done

echo
# The count of apps is part of the result: "no findings" over none and "no
# findings" over fifteen are the same sentence and opposite facts.
echo "-- $swept app(s) swept in $LOCALE; anything not in the table above was not looked at"
exit "$fail"
