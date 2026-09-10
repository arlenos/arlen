#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Does Tab walk each of an app's surfaces the way the eye reads them?
#
# WHY THIS IS ITS OWN SWEEP rather than a fifth column in the render table. The
# four layout probes answer yes or no and are worth running at three widths on
# every row. This one REPORTS - a toolbar on the right, a sidebar after the
# content and a two-column form all read as inversions and all of them can be
# correct - so it is a census a person reads, not a gate that goes red. Running
# it three times per row would triple a sweep's renders for a signal nobody can
# automate a verdict from.
#
# ONE WIDTH, because tab order is a property of the DOM and the layout at a
# readable size; the narrow and wide passes would report the same pairs with
# different pixel counts.
#
# THE ROUTES COME FROM `sweep-render-all.sh`'s own table, so this cannot drift
# from what the render sweep covers: what is not in that table is invisible to
# both. Selector and `@@host` suffixes are dropped - an open panel is a different
# question and belongs to `sweep-escape.sh`, which already presses the key at it.
#
#   dev/screenshot/sweep-tab-order.sh <app> <base-url> [width]
#
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

app="${1:-}"
base="${2:-}"
width="${3:-1280}"
[ -n "$app" ] && [ -n "$base" ] || {
  echo "usage: sweep-tab-order.sh <app> <base-url> [width]" >&2
  exit 2
}

table="$here/sweep-render-all.sh"
[ -f "$table" ] || { echo "sweep-tab-order.sh: no $table" >&2; exit 2; }

# The app's row, then its routes with the suffixes stripped and duplicates gone.
row="$(sed -n "s|^  \"$app \(.*\)\"$|\1|p" "$table" | head -1)"
[ -n "$row" ] || {
  echo "sweep-tab-order.sh: $table names no app '$app'." >&2
  echo "  A name that matches nothing would sweep nothing and read as a clean run." >&2
  exit 2
}

routes=""
IFS='|' read -ra specs <<<"$row"
for spec in "${specs[@]}"; do
  r="${spec%%::*}"
  r="${r%%@@*}"
  case " $routes " in *" $r "*) continue ;; esac
  routes="$routes $r"
done

# THE CONTROL FIRST, for the reason every probe in this tree has one: a probe
# that answers nothing looks exactly like a page with nothing to say, and this
# one's whole output is a list that is usually short.
shot="$(mktemp /tmp/arlen-taborder-XXXXXX.png)"
trap 'rm -f "$shot"' EXIT
proof="$("$here/headless.sh" --url "file://$here/tab-order-control.html" \
  --out "$shot" --width "$width" --probe-file "$here/tab-order.js" 2>/dev/null \
  | grep -E '^\[' | tail -1)"
case "$proof" in
  *"tab goes up"*) ;;
  *)
    echo "sweep-tab-order.sh: tab-order.js cannot see its own control;" >&2
    echo "  every surface below would read as ordered. it said: $proof" >&2
    exit 2
    ;;
esac

echo "== $app, tab order at ${width}px"
found=0
for r in $routes; do
  case "$r" in
    *\?*) url="$base$r&locale=de" ;;
    *) url="$base$r?locale=de" ;;
  esac
  got="$("$here/headless.sh" --url "$url" --out "$shot" --width "$width" \
    --probe-file "$here/tab-order.js" 2>/dev/null | grep -E '^\[' | tail -1)"
  case "$got" in
    "[]")
      echo "  ok   $r"
      ;;
    "["*"]")
      echo "  read $r"
      printf '%s\n' "$got" | sed 's/^\[//; s/\]$//; s/","/"\n"/g' | sed 's/^/       /'
      found=$((found + 1))
      ;;
    *)
      echo "  FAIL $r did not answer: ${got:-the probe printed nothing}"
      found=$((found + 1))
      ;;
  esac
done

echo "--   $(echo $routes | wc -w) surface(s) read; $found with something to look at."
echo "     A finding here is a QUESTION, not a defect: a sidebar after the content"
echo "     and a toolbar on the right both read as inversions and both can be right."
