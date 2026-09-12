#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Render each route in a language and report every way the text came out wrong.
#
# FOUR PROBES, NOT ONE, and that is why this is no longer `sweep-clipped.sh`.
# On 5 September a header change was verified with the parent-cut probe alone,
# came back clean, and had put one row of the header on top of another - which
# the overlap probe would have said in a sentence. The three ask genuinely
# different questions and a layout change can pass any two:
#
#   clipped-text.js       an element outgrew its own box, either axis
#   clipped-by-parent.js  an ancestor that clips cut a child sideways
#   overlapping-text.js   two elements are painted in the same place
#   no-focus-ring.js      a control takes keyboard focus and looks no different
#
# WHY THIS EXISTS AS A SWEEP. `clipped-text.js` has been here since August and it
# works; what has never existed is anything that points it at more than the one
# page somebody happened to be looking at. The defects it finds are the ones a
# fixed pixel width takes when a longer language goes through it -
# `Vertrauensstufe` past a column sized for `Trust level` - and four of those were
# found by hand in one night in August, which says the axis is productive and
# nobody has swept it since.
#
#   dev/screenshot/sweep-render.sh http://localhost:1454 de / /settings
#
# A path may carry a CSS selector after `::`, which is clicked before the probe
# runs. That is not a nicety: eleven of the thirteen apps have ONE route and keep
# their content behind tabs and sidebars, so route-walking alone reads a landing
# page and calls the app clean.
#
#   dev/screenshot/sweep-render.sh http://localhost:1434 de "/::#tab-timers"
#
# THE SELECTORS ARE ALREADY WRITTEN DOWN. `sweep-no-backend.sh`'s SHOTS table
# lists every app's route plus the click that reaches its other surfaces - the
# clock's four tabs, the file manager's Recent and Trash, knowledge's Library,
# Projects and Searches, the shortcuts dialog. Reuse them; rediscovering the
# selectors per run is how a sweep ends up reading only landing pages, and mail
# hid a name rendering at ZERO width one click from a list that swept clean
# twice.
#
# Start the app's dev server first, and it must be `vite dev`: the kit's
# `applyDevLocale` only runs under a dev build, so `vite preview` renders the
# source language whatever the query says.
#
# `sweep-render-all.sh <locale> [app]` does that part, and it also holds the table
# of which routes and clicks reach each app's other surfaces - which is the half
# worth having. Prefer it over starting a server by hand for a one-off: a server
# started by hand is one nobody stops, and it then answers the NEXT run's
# readiness check.
#
# Exits non-zero if any route clipped. A CLIP IS A FINDING, NOT A VERDICT, and
# the first run says why: mail's list came back with five, every one of them the
# row snippet - a one-line preview of a long body, which is cut on purpose. The
# probe cannot tell that from a label whose word ran out of column, and neither
# can a rule: `text-overflow: ellipsis` is on both. So it prints what it saw and
# a person decides. Settings came back clean over five routes including the
# physical-privacy page, which is the same run saying the probe was awake.
set -uo pipefail

base="${1:?usage: sweep-render.sh <base-url> <locale> <path> [path...]}"
case "$base" in
  http://*|https://*|file://*) ;;
  *)
    # The same mistake scan-message-ids.sh records: an app NAME here makes every
    # load a relative path, no page renders, and nothing clips - a clean result
    # from a page that never existed.
    echo "sweep-render.sh: '$base' is not a base URL." >&2
    exit 2
    ;;
esac
locale="${2:?give a locale, e.g. de}"
# WIDTH IS PART OF THE QUESTION, not a detail. This swept every app at 1280 and
# reported them clean, which was true and narrow: at 720 the calendar's title
# truncates to a single glyph because the view switcher takes the row. A window
# is resizable, so a clean line means clean AT THAT WIDTH and the sweep has to
# say which.
#
# THREE WIDTHS BY DEFAULT, since a sweep that has to be run three times to learn
# that gets run once. 720 is a window dragged narrow, 1280 the shape most of these
# open at, 1920 a monitor - and all three have now produced a finding the other two
# missed: the collapsed date title at 720, the clean baseline at 1280, the event
# labels cut by their own blocks at 1920. `SWEEP_WIDTH` still overrides, with one
# width or several: `SWEEP_WIDTH="800 1440"`.
widths="${SWEEP_WIDTH:-720 1280 1920}"
shift 2
[ "$#" -gt 0 ] || { echo "give at least one path" >&2; exit 2; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# A sweep IS the render that is running, so its own renders must not refuse each
# other - `own_display` reads this. A hand render started beside a sweep has no
# such flag and is refused, which is the point.
export ARLEN_SWEEP=1
shot="$(mktemp /tmp/sweep-render-XXXXXX.png)"
trap 'rm -f "$shot"' EXIT

# THIS FILE'S OWN FINGERPRINT, TAKEN BEFORE THE FIRST RENDER. A full sweep runs
# for the better part of an hour, and bash reads a script INCREMENTALLY - so an
# edit landing mid-run changes what the rest of the run executes, and the table
# it prints is then about two different scripts. That happened twice on
# 8 September, both times to somebody who knew the rule and thought the edit was
# too small to matter. Small is the size of edit that gets made without checking
# what is running.
#
# It cannot be prevented from in here, so it is REPORTED: the hash is compared
# again before the tally, and a change says so loudly next to the numbers rather
# than leaving them to be trusted. Same reasoning as every other line in this
# file that names what it did not look at.
self_hash_start="$(sha256sum "${BASH_SOURCE[0]}" 2>/dev/null | cut -d" " -f1)"

probes="clipped-text clipped-by-parent overlapping-text no-focus-ring"

# A FIFTH, ON HOST ROWS ONLY. `container-focus-ring.js` reads whatever currently
# HAS focus, so it can only ever speak on a surface where something focused a
# container - a dialog, an overlay, a switcher - which in this table is exactly
# the rows that name a host. Running it on every route would cost a quarter more
# sweep for a probe with nothing to look at, and a probe that always answers `[]`
# is how a table stops being read.
host_probes="container-focus-ring"

# THE ANSWER IS THE LAST LINE THAT LOOKS LIKE ONE, not the last line. Mesa
# writes `DRI3 error: Could not get DRI3 device` to STDOUT on this machine, and
# after the answer as often as before it - so `tail -n 1` read a driver warning
# as the probe's verdict and reported four clean surfaces as unanswered on the
# first full run of the host rows. A run that really did not answer still leaves
# no `[...]` line at all, so a refusal stays loud.
answer() { grep -E '^\[' | tail -n 1; }

# Whether this run carries a host spec, decided before the controls so they can
# guard the runner that will actually be used.
has_host=""
for spec in "$@"; do
  case "$spec" in *@@*) has_host=1 ;; esac
done

# THE POSITIVE CONTROLS, and they are not ceremony. A probe that returns nothing
# is indistinguishable from a probe that is not running, and `clipped-text.js`
# says in its own header that it returned nothing on three pages before its
# author checked it. So EACH of the three is handed its own committed fixture
# first, every run - a box too small for its word, a child cut by a clipping
# parent, two lines painted in the same place, a button that lights up for
# nobody. A probe whose control comes back
# clean cannot see what it exists to see, and the sweep stops rather than
# reporting clean routes on its word.
#
# One word per control, matched loosely on purpose: pinning the exact answer
# here would mean editing this file whenever a fixture gains a case, and the
# fixtures own their exact answers in their own headers.
for probe in $probes $host_probes; do
  # CLEARED FIRST, and then required to be non-empty. Both halves matter and the
  # second one is not hypothetical: an empty expectation makes `*"$want"*` match
  # every answer including `[]`, so the control passes on a blind probe - which
  # is the exact vacuity three gates in `dev/scripts` were caught with in
  # September ("0 app(s) checked" also exits 0). Without the clear, a probe added
  # to `$probes` with no arm below would silently inherit the previous one's
  # word and be gated against the wrong fixture, which is worse than either.
  want=""
  case "$probe" in
    clipped-text) want="Vertrauensstufe" ;;
    clipped-by-parent) want="Cut off by its parent" ;;
    overlapping-text) want="Painted over" ;;
    no-focus-ring) want="takes focus" ;;
    container-focus-ring) want="round itself" ;;
  esac
  [ -n "$want" ] || {
    echo "sweep-render.sh: no control expectation for $probe.js; add its arm above." >&2
    exit 2
  }
  proof="$("$here/shoot.sh" "file://$here/$probe-control.html" "$shot" "$here/$probe.js" 2>&1 \
    | sed -n 's/^inject result: //p')"
  case "$proof" in
    *"$want"*) ;;
    *)
      echo "sweep-render.sh: $probe.js cannot see its own control; it would report every route clean." >&2
      echo "  the control answered: $proof" >&2
      exit 2
      ;;
  esac
  # AND THROUGH THE OTHER RUNNER, when this run has a host spec in it. A host row
  # cannot go through `shoot.sh` at all, so its probes run under `render-wide.py`
  # - a second path with its own stringification, which the controls above do not
  # touch. The first version of that path answered `"[]"` for two of the four
  # probes, a quoted string the sweep reads as no answer; had it answered `[]` by
  # accident instead, every host row would have read clean forever. So each probe
  # is handed the same control again through the runner that will actually carry
  # it.
  if [ -n "$has_host" ]; then
    proof="$("$here/headless.sh" --url "file://$here/$probe-control.html" \
      --out "$shot" --probe-file "$here/$probe.js" 2>/dev/null | answer)"
    # BOTH HALVES: the control's own word, AND the `[...]` shape the loop below
    # reads. Checking the word alone let a broken stringification through - the
    # array probes came back as a comma-joined string that still contained
    # "Vertrauensstufe", so the control passed and every host route then answered
    # nothing. Loud rather than clean, but the control is supposed to be the loud
    # part.
    case "$proof" in
      "["*"]") ;;
      *)
        echo "sweep-render.sh: $probe.js does not answer in the [...] shape through headless.sh;" >&2
        echo "  every host row would report as unanswered. it said: $proof" >&2
        exit 2
        ;;
    esac
    case "$proof" in
      *"$want"*) ;;
      *)
        echo "sweep-render.sh: $probe.js cannot see its own control through headless.sh;" >&2
        echo "  every host row would read clean. the control answered: $proof" >&2
        exit 2
        ;;
    esac
  fi
done

# AND THE CONTROL FOR THE CLICK ITSELF, once rather than per probe. Forty-odd
# rows in the table carry a `::selector`, and until 10 September the runner asked
# for it exactly once, the moment `--settle` ended. A control the app renders a
# tick later was a coin toss: the mail sweep refused `#folder-trash` at 1280px
# and `#folder-archive` at the same width an hour before, on rows that pass at
# 720px. It now polls for five seconds.
#
# Only run when this run actually clicks something. A route-only sweep does not
# depend on the open path, and the fixture costs a driver start.
if printf '%s\n' "$@" | grep -q '::'; then
  proof="$(SHOOT_OPEN="#open-control-late" "$here/shoot.sh" \
    "file://$here/open-clicked-control.html" "$shot" "$here/open-clicked.js" 2>&1 \
    | sed -n 's/^inject result: //p')"
  case "$proof" in
    *"the late button was pressed"*) ;;
    *)
      echo "sweep-render.sh: --open cannot reach a control that appears after the settle;" >&2
      echo "  every ::selector row would report on the page behind it. it said: $proof" >&2
      exit 2
      ;;
  esac
fi

# WHAT IT LOOKED AT, said out loud at the end. On its second run this reported
# "ok /" for the clock and the file manager and that reads like "the app is
# clean" - it means "the landing page is". Both are single-route apps whose real
# surfaces (the clock's four tabs, the file manager's Trash and Recent) live
# behind clicks, so walking routes finds one page and the coverage line has to
# say so rather than let a tidy list imply the rest.
checked=0

fail=0
for width in $widths; do
 echo "  ==   at ${width}px"
 for spec in "$@"; do
  # A SPEC MAY NAME A HOST, `route[::selector]@@<file in hosts/>`, and four
  # surfaces need one to exist at all: no sound server, no battery, no tray
  # client, no player, so the applet is not in the DOM and the click has nothing
  # to hit. `shoot.sh` drives WebKitWebDriver, which cannot run a script BEFORE
  # the page's own - so a host has to come from `render-wide.py`, which installs
  # one as a document-start user script. Those four rows sat in the table for a
  # day printing "did not answer" four times each, because the `@@` fell into the
  # click selector and matched no element. Loud, and still unmeasured.
  #
  # A HOST ROW PINS ITS OWN LOCALE in the path and keeps it: the browser reads
  # the FIRST value of a repeated parameter, so the `&locale=` appended below
  # loses. That is right for these four - their fixtures speak one language, and
  # the panel's German is the picture worth having whichever way the sweep runs.
  host=""
  case "$spec" in
    *@@*) host="${spec##*@@}"; spec="${spec%@@*}" ;;
  esac
  path="${spec%%::*}"
  open=""
  [ "$spec" != "$path" ] && open="${spec#*::}"
  # A PATH MAY ALREADY CARRY A QUERY, and gluing `?locale=` onto it made a second
  # `?` - which the browser reads as part of the first parameter's value, so
  # `/?demo=image` came back as the DEFAULT demo with the sweep reporting it under
  # the name of the one it meant to look at. The viewers app reaches its image and
  # video faces only that way, so this is the difference between sweeping them and
  # sweeping the audio face three times. Measured both ways on the viewers app:
  # `/?demo=image?locale=de` renders an empty window and reads its demo as
  # `image?locale=de`, and the German never arrives either; `&` renders the image
  # face in German.
  case "$path" in
    *\?*) url="$base$path&locale=$locale" ;;
    *) url="$base$path?locale=$locale" ;;
  esac
  clean=1
  # A HOST ROW HAS TO REACH ITS STATE FIRST, and until 10 September this path did
  # not ask. `probe-host.sh` has checked it since the day `files-refuses-op`
  # dispatched `contextmenu` at an ancestor, opened no menu, refused nothing, and
  # answered clean for weeks - but the sweep runs host rows through its own
  # branch, which only ran the probes. A fixture that stops reaching its state
  # renders the ordinary page, and four probes come back empty, and the row reads
  # `ok`.
  #
  # ONLY IN GERMAN, because that is what the fixtures declare: every `// EXPECT:`
  # in `hosts/` is the German sentence. In an English run there is nothing to
  # compare against and the check is skipped rather than guessed - so `de` is the
  # run that verifies its fixtures and `en` is the run that does not, which is
  # worth knowing before reading an `en` sweep as proof.
  if [ -n "$host" ] && [ "$locale" = "de" ]; then
    want="$(sed -n 's|^// EXPECT: *||p' "$here/hosts/$host.js" | head -1)"
    if [ -z "$want" ]; then
      echo "  FAIL $spec@@$host declares no '// EXPECT: <text>' line"
      fail=1
      continue
    fi
    # BY SHAPE, not by position. `lib/host-state.js` answers as a one-element array
    # like every probe, and the line is found with the same `^\[` the probes use -
    # because "the last line" is a MESA driver warning one run in ten, and this
    # check then reported the graphics driver as the page's own words. Two of
    # calendar's rows failed that way on 10 September before the reader was fixed
    # on both sides.
    # STDERR IS KEPT, and it is the whole difference between a finding and an
    # afternoon. This threw it away, so a row whose `--open` selector never
    # appeared - the renderer says so, in one clear line - reported only "the
    # page read: " with nothing after it, and the reason was invisible. On
    # 11 September four rows failed that way and I spent an hour on the wrong
    # cause. The renderer's own refusals now come out with the finding.
    why="$("$here/headless.sh" --url "$url" --out "$shot" --width "$width" \
      --host-script "$here/hosts/$host.js" --probe-file "$here/lib/host-state.js" \
      ${open:+--open "$open"} 2>&1)"
    seen="$(printf '%s' "$why" | grep -E '^\[' | tail -1)"
    case "$seen" in
      *"$want"*) ;;
      *)
        echo "  FAIL $spec@@$host never reached its state; it says it should show:"
        echo "       $want"
        echo "       the page read: $(printf '%s' "$seen" | head -c 200)"
        # Everything the renderer said that is not the answer and not a driver
        # warning: its refusals, its viewport line, its notes about other
        # displays.
        printf '%s\n' "$why" | grep -vE '^\[|MESA|DRI3|Gdk-WARNING|^$' \
          | tail -3 | sed 's/^/       /'
        fail=1
        continue
        ;;
    esac
  fi
  # The host rows get the container-ring probe as well; a route walk has nothing
  # focused for it to read.
  row_probes="$probes"
  [ -n "$host" ] && row_probes="$probes $host_probes"
  for probe in $row_probes; do
    if [ -n "$host" ]; then
      hostfile="$here/hosts/$host.js"
      if [ ! -f "$hostfile" ]; then
        echo "  FAIL $spec names no host: $hostfile is not there"
        fail=1
        clean=0
        continue
      fi
      hostargs=(--url "$url" --out "$shot" --width "$width"
                --host-script "$hostfile" --probe-file "$here/$probe.js")
      [ -n "$open" ] && hostargs+=(--open "$open")
      # A run that refuses prints no `[...]` line at all, so `got` is empty and the
      # loop below reports it as a route that did not answer - loud, not clean.
      #
      # THE WHOLE OUTPUT IS KEPT, not just the answer line. It used to be piped
      # straight into `answer`, so a refusal left `got` empty and the failure line
      # read `did not answer overlapping-text:` with nothing after the colon - the
      # reason had been printed and thrown away one pipe earlier. `answer` only
      # ever takes the `[...]` line, so carrying stderr through it costs nothing.
      raw="$("$here/headless.sh" "${hostargs[@]}" 2>&1)"
      got="$(printf '%s\n' "$raw" | answer)"
    else
      raw="$(SHOOT_OPEN="$open" "$here/shoot.sh" "$url" "$shot" "$here/$probe.js" "$width" 2>&1)"
      got="$(printf '%s\n' "$raw" | sed -n 's/^inject result: //p')"
    fi
    # ONE RETRY, for the one answer that is about TIMING rather than about the
    # page. `shoot.py` waits a FIXED `--settle` after load and then injects, so a
    # route whose first paint slips past it is measured before it exists; the
    # focus probe says so in those words rather than reporting a defect, and it
    # is the only probe outcome in the set that does. Everything else here is a
    # statement about what was rendered and must not be re-rolled - retrying a
    # real finding until it goes away is how a sweep stops meaning anything.
    #
    # Found on 13 September: settings' typography page came back keyboard-less at
    # 1280 in a three-width run and `ok` at 720 and 1920 in the same two runs -
    # a page with no focusable control fails all three. Its controls are rendered
    # unconditionally, with no load guard in front of them, so it had not painted.
    # Six standalone runs of that route pass 6/6, which is what makes CONTENTION
    # the reading: the miss happened inside a fifty-seven-route sweep, not alone.
    # The `--open` path already learned this lesson and polls; the plain probe
    # path never got it.
    #
    # WHAT IT DOES NOT FIX, since the retry re-navigates: the second attempt gets
    # the same fixed settle as the first, so a page that is RELIABLY slower than
    # that fails twice and is reported. That is the right way round - this can
    # convert a transient miss into a measurement and cannot convert a real one
    # into silence - but it is not a fix for a slow page, and nobody should read
    # it as one.
    #
    # BOTH HALVES WERE OBSERVED, not reasoned about: forcing `shoot.py`'s settle
    # to zero makes the condition fire every time, and the run then printed the
    # re-read line AND still reported the finding on the second attempt. So the
    # branch announces itself and a persistent miss survives it, which are the
    # two things it had to be true of.
    case "$got" in
      *"has not painted yet"*)
        # SAID OUT LOUD, never silent. A measurement tool that re-rolls a reading
        # without telling you has stopped being one; this line is what makes the
        # retry auditable instead of something a reader has to take on trust, and
        # it is why the retry needs no separate control - every firing declares
        # itself in the table it affects.
        echo "  --   $spec re-read at ${width}px: $probe measured before the page painted"
        sleep 2
        if [ -n "$host" ]; then
          raw="$("$here/headless.sh" "${hostargs[@]}" 2>&1)"
          got="$(printf '%s\n' "$raw" | answer)"
        else
          raw="$(SHOOT_OPEN="$open" "$here/shoot.sh" "$url" "$shot" "$here/$probe.js" "$width" 2>&1)"
          got="$(printf '%s\n' "$raw" | sed -n 's/^inject result: //p')"
        fi
        ;;
    esac
    case "$got" in
      "["*"]")
        checked=$((checked + 1))
        if [ "$got" != "[]" ]; then
          clean=0
          # Named by probe, because the three mean different things: an
          # ellipsis is often correct, a parent cut is a control somebody
          # cannot reach, an overlap is never right.
          echo "  $probe  $spec${host:+@@$host}"
          # Printed, not counted. Some of these are a scroll container the
          # probe cannot tell from a cut, and a number would hide which.
          printf '       %s\n' "$got"
          fail=1
        fi
        ;;
      *)
        # A route that did not render is not a clean route, which is the false
        # green this family of checks keeps finding its way back into.
        clean=0
        # And say WHY, in the run's own words. An empty `got` is the common case -
        # the probe refused or the driver died before printing an answer - and the
        # line used to end at the colon, which told a reader the reading failed and
        # nothing about what failed. The last two non-empty lines of the run are
        # where the refusal or the traceback lands.
        why="$got"
        if [ -z "$why" ]; then
          why="$(printf '%s\n' "$raw" | grep -v '^[[:space:]]*$' | tail -2 | tr '\n' ' ')"
        fi
        [ -z "$why" ] && why="the probe printed nothing at all"
        echo "  FAIL $spec${host:+@@$host} did not answer $probe: $why"
        fail=1
        ;;
    esac
  done
  [ "$clean" = 1 ] && echo "  ok   $spec${host:+@@$host}"
 done
done
# FOUR on a route row and FIVE on a host row, because that is what ran. It said
# "four probes per view" flat until the container-ring probe was added on
# 8 September, which left the closing line of every sweep understating itself by
# one - in the file whose whole job is to say what was looked at.
self_hash_end="$(sha256sum "${BASH_SOURCE[0]}" 2>/dev/null | cut -d" " -f1)"
if [ -n "$self_hash_start" ] && [ "$self_hash_start" != "$self_hash_end" ]; then
  echo "  !!   THIS SCRIPT WAS EDITED WHILE THE RUN WAS USING IT. bash reads a"
  echo "  !!   script incrementally, so the rows above are not all from the same"
  echo "  !!   file. Re-run before treating any of it as evidence."
  fail=1
fi
echo "  --   $checked probe read(s) in $locale across ${widths// /, }px, four probes per route and five per host row; anything not named here was not looked at"
exit "$fail"
