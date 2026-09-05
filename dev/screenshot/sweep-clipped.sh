#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Render each route in a language and report the text its boxes are too small to
# hold.
#
# WHY THIS EXISTS AS A SWEEP. `clipped-text.js` has been here since August and it
# works; what has never existed is anything that points it at more than the one
# page somebody happened to be looking at. The defects it finds are the ones a
# fixed pixel width takes when a longer language goes through it -
# `Vertrauensstufe` past a column sized for `Trust level` - and four of those were
# found by hand in one night in August, which says the axis is productive and
# nobody has swept it since.
#
#   dev/screenshot/sweep-clipped.sh http://localhost:1454 de / /settings
#
# A path may carry a CSS selector after `::`, which is clicked before the probe
# runs. That is not a nicety: eleven of the thirteen apps have ONE route and keep
# their content behind tabs and sidebars, so route-walking alone reads a landing
# page and calls the app clean.
#
#   dev/screenshot/sweep-clipped.sh http://localhost:1434 de "/::#tab-timers"
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

base="${1:?usage: sweep-clipped.sh <base-url> <locale> <path> [path...]}"
case "$base" in
  http://*|https://*|file://*) ;;
  *)
    # The same mistake scan-message-ids.sh records: an app NAME here makes every
    # load a relative path, no page renders, and nothing clips - a clean result
    # from a page that never existed.
    echo "sweep-clipped.sh: '$base' is not a base URL." >&2
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
shot="$(mktemp /tmp/sweep-clipped-XXXXXX.png)"
control="$(mktemp -d /tmp/sweep-clipped-ctl-XXXXXX)"
trap 'rm -f "$shot"; rm -rf "$control"' EXIT

# THE POSITIVE CONTROL, and it is not ceremony. A probe that returns nothing is
# indistinguishable from a probe that is not running, and `clipped-text.js` says
# in its own header that it returned nothing on three pages before its author
# checked it. So a box too small for its word is rendered first, every run: if
# that does not come back clipped, this cannot see what it exists to see and it
# stops rather than reporting clean routes.
cat > "$control/index.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>positive control</title>
<div style="width:60px;overflow:hidden;white-space:nowrap">Vertrauensstufe verschlagwortet</div>
HTML
proof="$("$here/shoot.sh" "file://$control/index.html" "$shot" "$here/clipped-text.js" 2>&1 \
  | sed -n 's/^inject result: //p')"
case "$proof" in
  *Vertrauensstufe*) ;;
  *)
    echo "sweep-clipped.sh cannot see a clipped box; it would report every route clean." >&2
    echo "  the control answered: $proof" >&2
    exit 2
    ;;
esac

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
  got="$(SHOOT_OPEN="$open" "$here/shoot.sh" "$url" "$shot" "$here/clipped-text.js" "$width" 2>&1 \
    | sed -n 's/^inject result: //p')"
  case "$got" in
    "["*"]")
      checked=$((checked + 1))
      if [ "$got" = "[]" ]; then
        echo "  ok   $spec"
      else
        echo "  clip $spec"
        # Printed, not counted. Some of these are a scroll container the probe
        # cannot tell from a cut, and a number would hide which.
        printf '       %s\n' "$got"
        fail=1
      fi
      ;;
    *)
      # A route that did not render is not a clean route, which is the false
      # green this family of checks keeps finding its way back into.
      echo "  FAIL $spec did not answer: $got"
      fail=1
      ;;
  esac
 done
done
echo "  --   $checked view(s) read in $locale across ${widths// /, }px; anything not named here was not looked at"
exit "$fail"
