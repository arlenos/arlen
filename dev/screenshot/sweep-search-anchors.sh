#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Does every Settings search result land on something?
#
# WHY THIS EXISTS. A search entry names a panel and an anchor, and choosing it
# opens the page and scrolls to the element with that id. Nothing held the two
# together: the registry is a list of strings and the pages are markup, so a row
# that was renamed, moved to a sub-page or deleted left its entry pointing at
# nothing. On 11 November 25 of 74 anchors matched no id in the source. The
# result still opens the right page, which is why nobody saw it: the page
# appears and the scroll quietly does not happen.
#
# WHY IT RENDERS. Many ids are built at run time (`cat-${category.id}`,
# `action-${action.key}`, `geom-${f.key}`), so a grep over the source reports
# those as missing when they are fine and cannot tell a dead one from a live
# one. The rendered page is the only place the question has an answer.
#
# WHAT IT RENDERS UNDER. `lib/settings-config-answers.js`: config reads answer
# an empty file and every other call is refused. With no host at all a
# config-backed page draws one sentence in place of its rows, and the first run
# of this sweep read 37 misses where 26 were real.
#
# WHAT IT CANNOT SEE. An id that only exists once a daemon answered (a printer,
# a service's status row) is absent here. The registry must not anchor on
# those; an entry that does is reported like any other miss, which is the right
# reading: the search would scroll to nothing on a machine where that daemon is
# away.
#
#   dev/screenshot/sweep-search-anchors.sh http://localhost:1466
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

export ARLEN_SWEEP=1

base="${1:-}"
[ -n "$base" ] || { echo "usage: sweep-search-anchors.sh <base-url>" >&2; exit 2; }

work="$(mktemp -d /tmp/arlen-anchors-XXXXXX)"
trap 'rm -rf "$work"' EXIT

# One line per page: the route, then its anchors. The page is the entry's own
# `page` when it names one, else the href of its panel.
python3 - "$root/apps/settings/src/lib" > "$work/pages.txt" <<'PY' || exit 2
import pathlib, re, sys
lib = pathlib.Path(sys.argv[1])
nav = (lib / "stores/navigation.ts").read_text()
href = dict(re.findall(r'\{\s*id:\s*"([^"]+)"[^}]*?href:\s*"([^"]+)"', nav))
reg = (lib / "search/settings-registry.ts").read_text()
body = reg[reg.index("SETTINGS_REGISTRY"):]
pages = {}
count = 0
for entry in re.findall(r"\{\s*id:\s*\"[^\"]+\",.*?\n  \},", body, re.S):
    panel = re.search(r'\bpanel:\s*"([^"]+)"', entry)
    anchor = re.search(r'\banchor:\s*"([^"]+)"', entry)
    page = re.search(r'\bpage:\s*"([^"]+)"', entry)
    if not panel or not anchor:
        continue
    route = page.group(1) if page else href.get(panel.group(1))
    if route is None:
        sys.exit(f"panel {panel.group(1)!r} has no href in navigation.ts")
    pages.setdefault(route, []).append(anchor.group(1))
    count += 1
if count < 20:
    sys.exit(f"only {count} entries read from the registry: the pattern stopped matching")
for route, anchors in sorted(pages.items()):
    print(route, *anchors)
PY

probe_for() {
  # A probe that answers with the ids it could not find.
  local out="$1"; shift
  {
    printf 'const want = ['
    printf '"%s",' "$@"
    printf '];\n'
    printf 'return want.filter((id) => !document.getElementById(id));\n'
  } > "$out"
}

shot="$work/shot.png"
host="$here/lib/settings-config-answers.js"

# The control: an id no page has must come back as missing, or an empty answer
# below means nothing.
probe_for "$work/control.js" "arlen-anchor-control-absent"
proof="$("$here/headless.sh" --url "$base/" --out "$shot" --width 1280 \
  --host-script "$host"   --probe-file "$work/control.js" 2>/dev/null | grep -E '^\[' | tail -1)"
case "$proof" in
  *arlen-anchor-control-absent*) ;;
  *)
    echo "sweep-search-anchors.sh: the probe did not report an id that is not there;" >&2
    echo "  every page below would read as complete. it said: ${proof:-nothing}" >&2
    exit 2
    ;;
esac

echo "== settings, search anchors"
missing=0
total=0
while read -r route anchors; do
  # shellcheck disable=SC2086
  set -- $anchors
  total=$((total + $#))
  probe_for "$work/probe.js" "$@"
  got="$("$here/headless.sh" --url "$base$route" --out "$shot" --width 1280 \
  --host-script "$host"     --probe-file "$work/probe.js" 2>/dev/null | grep -E '^\[' | tail -1)"
  case "$got" in
    "[]")
      echo "  ok   $route ($# anchors)"
      ;;
    "["*"]")
      n="$(printf '%s' "$got" | grep -o '","' | wc -l)"
      n=$((n + 1))
      echo "  MISS $route: $got"
      missing=$((missing + n))
      ;;
    *)
      echo "  FAIL $route did not answer: ${got:-the probe printed nothing}"
      missing=$((missing + 1))
      ;;
  esac
done < "$work/pages.txt"

if [ "$missing" -gt 0 ]; then
  echo "$missing of $total search anchors land on nothing"
  exit 1
fi
echo "all $total search anchors land on an element"
