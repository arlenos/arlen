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

fail=0
for path in "$@"; do
  url="$base$path?locale=$locale"
  got="$("$here/shoot.sh" "$url" "$shot" "$here/clipped-text.js" 2>&1 \
    | sed -n 's/^inject result: //p')"
  case "$got" in
    "["*"]")
      if [ "$got" = "[]" ]; then
        echo "  ok   $path"
      else
        echo "  clip $path"
        # Printed, not counted. Some of these are a scroll container the probe
        # cannot tell from a cut, and a number would hide which.
        printf '       %s\n' "$got"
        fail=1
      fi
      ;;
    *)
      # A route that did not render is not a clean route, which is the false
      # green this family of checks keeps finding its way back into.
      echo "  FAIL $path did not answer: $got"
      fail=1
      ;;
  esac
done
exit "$fail"
