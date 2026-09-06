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
# THE TWELVE SETTINGS PAGES IN THE MIDDLE came from comparing this table with
# `sweep-no-backend.sh`'s SHOTS list on 6 September. Each of them had a
# no-backend shot - so its unavailable-state copy was photographed - and had
# never been through a render probe. Two lists, two properties, and the gap
# between them was invisible while nobody put them side by side.
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
# THE BASE PORT IS PER-RUN for the same reason `shoot.sh`'s driver port is: two
# of these are worth running at once - a full pass is fifteen apps and most of an
# hour - and with a fixed base the second run refuses on every single app with
# "port 6100 is held by an earlier one". Correct, loud and useless. Derived from
# the pid, in blocks of 40 so one run's fifteen ports cannot walk into another's.
# Checked: two full sweeps started together both answer, where before the second
# refused on every app in its list.
PORT=$(( 6100 + ($$ % 60) * 40 ))

# `<app> <spec>|<spec>|...`, where a spec is a route optionally followed by
# `::selector` - the thing to click before the probes run.
#
# PIPE-SEPARATED, not space-separated, and that is not decoration: a CSS selector
# may contain a space. `.bar-side.left .trigger` split into two arguments the
# first time the greeter's two menus went in the table, and the sweep reported
# `FAIL "/::.bar-side.right did not answer` four times - loud, which is right,
# and still four wasted readings.
#
# NOT HERE, deliberately: `harness` and `store` are arlen-ui's live surfaces, and
# a shared sweep that goes red on another lane's work is a sweep somebody turns
# off. `desktop-shell` is here because its three windows are the surfaces a person
# sees most.
SURFACES=(
  "calendar /|/::.seg-pill:nth-of-type(2)|/::.seg-pill:nth-of-type(3)|/::.seg-pill:nth-of-type(4)|/::.seg-pill:nth-of-type(5)|/::#cal-new-event"
  "clock /|/::#chrome-add|/::#tab-timers|/::#tab-focus|/::#tab-stopwatch|/::#tab-world"
  "desktop-shell /|/consent|/waypointer"
  "files /|/::[data-place=recent]|/::[data-place=trash]"
  "greeter /|/::.bar-side.left .trigger|/::.bar-side.right .trigger"
  "knowledge /|/::button[data-place=projects]|/::button[data-place=library]|/::button[data-place=searches]"
  "mail /|/::.row|/::#folder-sent|/::#folder-drafts|/::#folder-archive|/::#folder-trash"
  "meetings /|/capture|/meeting/abc"
  "pdf /"
  "screenshot /"
  "settings /|/accessibility|/appearance/quicksettings|/appearance/wallpaper|/focus|/keyboard|/knowledge|/printers|/privacy|/privacy/physical|/system-actions|/windows-apps|/workspaces|/keyboard/shortcuts|/keyboard/shortcuts::[data-action=add-custom]|/keyboard/shortcuts::[data-action=reset-all]"
  "system-monitor /|/::#tab-performance"
  "terminal /|/::#terminal-history-open|/::#terminal-new-session"
  "text-editor /|/::.trigger"
  "viewers /|/?demo=image|/?demo=video"
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

# THE SERVER OUTLIVES AN INTERRUPTED RUN WITHOUT THIS. Each app's dev server is
# killed after its sweep, at the bottom of the loop - which never runs if the
# sweep is stopped part-way, and a full pass is most of an hour, so it gets
# stopped. Measured on 6 September: thirteen `vite dev` processes still listening
# on 6431 through 6811, one per interrupted run, hours old.
#
# The leak is not the cost. A left-over server answers `curl` perfectly well, and
# `curl` cannot say who answered - which is the exact failure the check below
# already guards against for a port held by an EARLIER run. It guards the reader
# and leaves the litter.
#
# `sweep-axe.sh`, this script's twin, has carried the trap since the day it was
# written. Same line, and the reason it belongs here rather than only there is
# that a signal is the case a bottom-of-the-loop kill structurally cannot cover.
# NOT `kill -- "-${server:-0}"`. That default is the whole reason this is a
# function: with `server` empty - before the first app starts one, and after each
# is reaped - `-0` is not "nobody", it is THIS PROCESS GROUP. The first cut wrote
# it that way, copied from the twin, and the clean run at the end signalled
# itself: 72 probes green, then exit 139 and a core dump, from a script whose
# work had finished. Signal only what this run actually started.
stop_current() {
  [ -n "${sweep:-}" ] && kill "$sweep" 2>/dev/null
  [ -n "${server:-}" ] && kill -- "-$server" 2>/dev/null
  return 0
}
trap stop_current EXIT INT TERM

swept=0
fail=0
for entry in "${SURFACES[@]}"; do
  read -r app specs <<<"$entry"
  IFS='|' read -r -a spec_list <<<"$specs"
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
    server=""
    fail=1
    PORT=$((PORT + 1))
    continue
  fi

  echo "== $app"
  # RUN IT IN THE BACKGROUND AND WAIT, rather than calling it in the foreground.
  # Bash defers a trap until the current foreground command returns, so a plain
  # call means a SIGTERM - which is what a timeout or a `kill <pid>` sends - is
  # held until this app's whole sweep finishes, minutes later, and until then the
  # server is still up and the sweep still running. `wait` is interruptible, so
  # the trap fires at once and takes the server with it. Measured both ways: with
  # the foreground call the port was still answering four seconds after the TERM.
  "$(dirname "${BASH_SOURCE[0]}")/sweep-render.sh" "http://localhost:$PORT" "$LOCALE" "${spec_list[@]}" &
  sweep=$!
  wait "$sweep" || fail=1
  sweep=""
  swept=$((swept + 1))

  kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
  # Forgotten once it is reaped, so the trap above cannot signal a group id that
  # has since been handed to somebody else.
  server=""
  PORT=$((PORT + 1))
done

echo
# The count of apps is part of the result: "no findings" over none and "no
# findings" over fifteen are the same sentence and opposite facts.
echo "-- $swept app(s) swept in $LOCALE; anything not in the table above was not looked at"
exit "$fail"
