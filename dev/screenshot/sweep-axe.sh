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
#
# ONE LANDING PAGE PER APP WAS NOT COVERAGE EITHER, which is the same lesson one
# level in. Until 7 September every entry here was a single route, so eleven of
# these apps had their tabs, dialogs and secondary views measured by nothing at
# all - the clock's four other tabs, knowledge's Library and Projects, the file
# manager's Recent and Trash, settings' fifteen pages. The render sweep has
# reached those surfaces for weeks through `sweep-render-all.sh`, and that table
# is where these specs come from: same routes, same clicks, so the two sweeps
# look at the same tree and a surface missing from one is missing from both.
#
# PIPE-SEPARATED, not space-separated, and for the reason its twin records: a CSS
# selector may contain a space, and `.bar-side.left .trigger` split into two
# arguments the first time it was written that way.
#
# A SPEC MAY NAME A HOST, `route@@<file in hosts/>`, and one surface needed it
# badly enough to be worth the syntax. The consent card mounts only on the window
# labelled `consent`, and only once the broker has handed it a request and the
# shell has granted the surface an input region - so a route walk renders an empty
# window, which is what this sweep saw for as long as `/consent` has been in the
# table. It was measured by hand on 6 September for the first time and was
# `serious`: the dialog had no accessible name at all. A surface that needs a
# runtime to exist was, until now, a surface this file could only pretend to
# cover.
#
# THE CONSENT SPECS PIN A LOCALE, and that is not decoration. A host declares
# what its state SAYS, in one sentence, and the sentence is in a language - so a
# spec that leaves the locale to the default is asking the check to hold in two
# languages at once. The first run of these three refused for exactly that: the
# page read "Allow Notes to read your notes and their tags?" against an EXPECT of
# "Zulassen?". The check was right and the spec was wrong, which is the good way
# round.
SURFACES=(
  "files /|/::[data-place=recent]|/::[data-place=trash]"
  "terminal /|/::#terminal-history-open|/::#terminal-new-session"
  "settings /|/accessibility|/appearance/quicksettings|/focus|/keyboard|/knowledge|/printers|/privacy|/privacy/physical|/system-actions|/windows-apps|/workspaces|/keyboard/shortcuts"
  "meetings /|/capture|/meeting/abc"
  "clock /|/::#chrome-add|/::#tab-timers|/::#tab-focus|/::#tab-stopwatch|/::#tab-world"
  "knowledge /|/::button[data-place=projects]|/::button[data-place=library]|/::button[data-place=searches]"
  "system-monitor /|/::#tab-performance"
  "text-editor /|/::.trigger"
  "viewers /|/?demo=image|/?demo=video"
  "screenshot /"
  "greeter /|/::.bar-side.left .trigger|/::.bar-side.right .trigger"
  "mail /|/::.row|/::#folder-sent|/::#folder-drafts|/::#folder-archive|/::#folder-trash"
  "calendar /|/::.seg-pill:nth-of-type(2)|/::.seg-pill:nth-of-type(3)|/::.seg-pill:nth-of-type(4)|/::.seg-pill:nth-of-type(5)|/::#cal-new-event"
  "pdf /"
  # The three request shapes worth their own reading: a standard grant, the
  # permanent delete with its hold-to-confirm, and the external send with a
  # preview of what would leave the machine.
  #
  # THE TWO PANELS ARE HERE BECAUSE THEY WERE NOWHERE. Quick Settings and the
  # notification list open from the bar, and this row carried no click at all - so
  # the two surfaces a person opens most often on this window had never been
  # measured. They are also where the panel-only markup lives, which is exactly
  # the kind that a landing-page walk cannot reach.
  "desktop-shell /waypointer|/|/consent|/consent?consent=1&locale=de@@shell-consent-request|/consent?consent=3&locale=de@@shell-consent-request|/consent?consent=4&locale=de@@shell-consent-request|/::[data-applet-id=quick-settings]|/::[data-applet-id=notifications]"
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
# `-${server:-0}` would be this script's OWN process group whenever `server` is
# unset - which it is on every path that refuses before the first server starts.
# It cost the twin a core dump on an otherwise clean run before it was read here.
trap 'rm -rf "$out"; [ -n "${server:-}" ] && kill -- "-$server" 2>/dev/null; true' EXIT

total=0
swept=0
for entry in "${SURFACES[@]}"; do
  read -r app specs <<<"$entry"
  [ -n "$ONLY" ] && [ "$ONLY" != "$app" ] && continue
  # `-` was the old spelling for "the landing page" and is kept so a one-route
  # entry can still be written that way.
  [ "$specs" = "-" ] && specs="/"
  IFS='|' read -r -a spec_list <<<"$specs"
  # The server comes up once per app; the readiness and title probes below are
  # about the SERVER, so they use the first route and the rest ride on it.
  route="${spec_list[0]%%::*}"

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
  # A SERVER THAT DIED IS NOT A PAGE THAT DID NOT COME UP, and the difference
  # is what the reader does next. `--strictPort` makes vite EXIT when the port
  # is taken, and a server left behind by a killed run answers `curl` and the
  # title probe perfectly well - so the sweep goes on to measure somebody else's
  # app under this app's name. On 5 September a killed run left the pdf server
  # on 5310; the next three sweeps each refused with "the page never reported a
  # title", which is true of the pdf page under this harness and says nothing
  # at all about terminal. Name the actual cause.
  if ! kill -0 "$server" 2>/dev/null; then
    printf '%-16s %s\n' "$app" "REFUSED: this run's dev server exited - port $PORT is held by an earlier one"
    # The one line worth showing is the bind error, not the three frames of
    # node stack that follow it.
    grep -m1 -E 'EADDRINUSE|already in use' "$out/$app.log" \
      | sed 's/^ *//; s/^/                 /' \
      || tail -1 "$out/$app.log" | sed 's/^/                 /'
    PORT=$((PORT + 1))
    continue
  fi

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
  # THROUGH `headless.sh`, NEVER `render-wide.py` DIRECTLY. Both renders below
  # used to call the renderer straight, which inherits whatever DISPLAY and
  # WAYLAND_DISPLAY the caller has - so an accessibility sweep of fifteen apps
  # opened fifteen real windows on the developer's screen, one after another,
  # and measured them under a live window manager rather than the fixed one this
  # tree screenshots against. `headless.sh` is the single place that owns that
  # recipe (an Xvfb at a real size, the host session cut off, a window manager so
  # `fullscreen()` is granted), and its own header records the run in August that
  # drew on somebody's session for exactly this reason.

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
  # ONE launch with time, rather than several without it. The first cut retried
  # the probe twelve times, and each retry is a whole browser and X server: for
  # an app whose title is slow or absent that is minutes of nothing, and it took
  # the pdf surface six of them before the run was killed. `--settle` waits
  # INSIDE the one session, which is what was wanted all along.
  served=""
  for _ in $(seq 1 3); do
    served=$(timeout 240 dev/screenshot/headless.sh \
      --url "http://localhost:$PORT$route" --out /dev/null --width "$WIDTH" \
      --timeout 180 --settle 4 --probe "document.title" 2>/dev/null | tail -1)
    [ -n "$served" ] && break
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

  # BOUNDED, because a surface that never answers must be reported as one rather
  # than eat the run. The pdf app hung here until the whole sweep was killed at
  # its outer timeout, and what that produced was no tally at all - worse than a
  # bad number, because a run that dies prints nothing to disbelieve.
  # THE LOAD ALLOWANCE IS THE WHOLE STORY FOR ONE APP. render-wide's default is
  # 60s and the pdf reader needs more than that on a cold vite server - 142
  # modules to transform before the load event - so it was refused every time
  # except the once the server happened to be warm, and the sweep called that
  # "no result". At 180 it loads and gets judged like everything else.
  # ONE RUN PER SURFACE. A click is a separate page as far as axe is concerned -
  # a dialog that opens over the landing page has its own labels, its own focus
  # order and its own contrast - so each spec gets its own browser, its own shot
  # and its own verdict file, named for the surface rather than the app.
  for spec in "${spec_list[@]}"; do
    # The host comes off first: it is the outermost part of a spec, and a CSS
    # selector may contain almost anything, so peeling it last would make the
    # click's own text able to end the spec.
    shost=""
    sbody="$spec"
    case "$spec" in *@@*) shost="${spec##*@@}"; sbody="${spec%@@*}" ;; esac
    sroute="${sbody%%::*}"
    sclick=""
    case "$sbody" in *::*) sclick="${sbody#*::}" ;; esac
    # AN ARRAY, not a string. The EXPECT line below is a sentence - "Das
    # Dateisystem hat sich geweigert" - and an unquoted string expansion would
    # hand each of its words to the renderer as a separate argument.
    hostargs=()
    if [ -n "$shost" ]; then
      hostfile="dev/screenshot/hosts/$shost.js"
      # A missing host file is a spec that names something that is not there, and
      # the render would silently take the no-runtime path instead - a clean axe
      # answer about a page the table did not ask for.
      if [ ! -f "$hostfile" ]; then
        printf '%-16s %s\n' "$app" "REFUSED: $spec names $hostfile, which does not exist"
        fail=1
        continue
      fi
      # THE HOST'S OWN CLAIM, CHECKED. A page that never reached its state is
      # empty, and axe reports an empty page as clean in exactly the words it
      # uses for a surface that passed - the false green this whole file is
      # written against, one level in. Every host declares what its state SAYS in
      # an `// EXPECT:` line for `probe-host.sh`; the same line is the guard here,
      # inside the one render rather than a second one.
      want=$(sed -n 's|^// EXPECT: *||p' "$hostfile" | head -1)
      if [ -z "$want" ]; then
        printf '%-16s %s\n' "$app" "REFUSED: $hostfile declares no '// EXPECT:' line"
        fail=1
        continue
      fi
      hostargs=(--host-script "$hostfile" --require-text "$want")
    fi
    # A file name a person can find again: the app, then the spec with the
    # characters a path cannot carry folded to dashes.
    slug=$(printf '%s' "$spec" | tr -c 'A-Za-z0-9._-' '-')
    base="$out/$app$([ "$spec" = "/" ] && echo "" || echo "-$slug")"
    if [ -n "$sclick" ]; then
      timeout 300 dev/screenshot/headless.sh \
        --url "http://localhost:$PORT$sroute" --open "$sclick" "${hostargs[@]}" \
        --out "$base.png" --width "$WIDTH" --axe --timeout 180 --settle 3 \
        >"$base.axe" 2>&1
    else
      timeout 300 dev/screenshot/headless.sh \
        --url "http://localhost:$PORT$sroute" "${hostargs[@]}" \
        --out "$base.png" --width "$WIDTH" --axe --timeout 180 --settle 3 \
        >"$base.axe" 2>&1
    fi
    if [ "$?" = 124 ]; then
      printf '%-16s %s\n' "$app" "REFUSED $spec: the page did not finish an axe run in 300s"
      continue
    fi
    # NO RESULT IS NOT NO VIOLATIONS. This printed "axe: no result" and counted the
    # surface as swept and clean; run by hand the same page answered with three,
    # one of them serious. So a missing verdict is a refusal now, and it prints
    # what the run actually said rather than leaving somebody to guess.
    if ! grep -qE '^axe:' "$base.axe"; then
      printf '%-16s %s\n' "$app" "REFUSED $spec: axe returned no verdict"
      tail -3 "$base.axe" | sed 's/^/                 /'
      continue
    fi
    n=$(grep -cE '^  [a-z-]+ \(' "$base.axe" || true)
    total=$((total + n))
    swept=$((swept + 1))
    printf '%-16s %-28s %s\n' "$app" "$spec" "$(grep -E '^axe:' "$base.axe")"
    grep -E '^  [a-z-]+ \(' "$base.axe" | sed 's/^/                 /' || true
  done

  kill -- "-$server" 2>/dev/null; wait "$server" 2>/dev/null
  PORT=$((PORT + 1))
done

echo
# The count of surfaces is part of the result, not a detail: "0 violations" over
# no surfaces and "0 violations" over fifteen are the same sentence and opposite
# facts.
echo "$total violation(s) across $swept surface(s) at ${WIDTH}px"
