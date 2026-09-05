#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Render each route in a language and report every way the text came out wrong.
#
# THREE PROBES, NOT ONE, and that is why this is no longer `sweep-clipped.sh`.
# On 5 September a header change was verified with the parent-cut probe alone,
# came back clean, and had put one row of the header on top of another - which
# the overlap probe would have said in a sentence. The three ask genuinely
# different questions and a layout change can pass any two:
#
#   clipped-text.js       an element outgrew its own box, either axis
#   clipped-by-parent.js  an ancestor that clips cut a child sideways
#   overlapping-text.js   two elements are painted in the same place
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
shot="$(mktemp /tmp/sweep-render-XXXXXX.png)"
trap 'rm -f "$shot"' EXIT

probes="clipped-text clipped-by-parent overlapping-text"

# THE POSITIVE CONTROLS, and they are not ceremony. A probe that returns nothing
# is indistinguishable from a probe that is not running, and `clipped-text.js`
# says in its own header that it returned nothing on three pages before its
# author checked it. So EACH of the three is handed its own committed fixture
# first, every run - a box too small for its word, a child cut by a clipping
# parent, two lines painted in the same place. A probe whose control comes back
# clean cannot see what it exists to see, and the sweep stops rather than
# reporting clean routes on its word.
#
# One word per control, matched loosely on purpose: pinning the exact answer
# here would mean editing this file whenever a fixture gains a case, and the
# fixtures own their exact answers in their own headers.
for probe in $probes; do
  case "$probe" in
    clipped-text) want="Vertrauensstufe" ;;
    clipped-by-parent) want="Cut off by its parent" ;;
    overlapping-text) want="Painted over" ;;
  esac
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
done

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
  path="${spec%%::*}"
  open=""
  [ "$spec" != "$path" ] && open="${spec#*::}"
  url="$base$path?locale=$locale"
  clean=1
  for probe in $probes; do
    got="$(SHOOT_OPEN="$open" "$here/shoot.sh" "$url" "$shot" "$here/$probe.js" "$width" 2>&1 \
      | sed -n 's/^inject result: //p')"
    case "$got" in
      "["*"]")
        checked=$((checked + 1))
        if [ "$got" != "[]" ]; then
          clean=0
          # Named by probe, because the three mean different things: an
          # ellipsis is often correct, a parent cut is a control somebody
          # cannot reach, an overlap is never right.
          echo "  $probe  $spec"
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
        echo "  FAIL $spec did not answer $probe: $got"
        fail=1
        ;;
    esac
  done
  [ "$clean" = 1 ] && echo "  ok   $spec"
 done
done
echo "  --   $checked probe read(s) in $locale across ${widths// /, }px, three probes per view; anything not named here was not looked at"
exit "$fail"
