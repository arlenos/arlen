#!/usr/bin/env bash
# Guard against silent drift between a daemon's canonical dist/*.service unit and
# the hand-maintained copy the image ships in dev/mkosi/mkosi.extra. The two are
# separate files today (mkosi.extra is copied verbatim into the image), so a
# hardening directive added to one but not the other deploys a unit that differs
# from the reviewed one - exactly the class that shipped an unaudited producer and
# a broken peer-auth sandbox before. This compares the DIRECTIVE lines only
# (stripping comments and blanks), so a comment reword never fails the gate but a
# real directive difference does. Units with no dist/ counterpart (arlen-ai-proxy,
# arlen-dogfood, arlen-config-broker, arlen-llama, arlen-graph, arlen-timeline)
# are mkosi-only and skipped.
#
# Exit 0 = every packaged unit's directives match its dist/ canonical (or has no
# canonical). Exit 1 = a drift a reviewer must reconcile.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Directive-only view of a unit: drop comments (# ...) and blank lines, so the
# comparison ignores prose and reflow.
directives() {
  grep -vE '^[[:space:]]*#|^[[:space:]]*$' "$1"
}

drift=0
checked=0
skipped=0

# Every packaged systemd unit under mkosi.extra (user + system trees), excluding
# the *.target.wants/ enablement symlinks (they point at the real unit files).
while IFS= read -r pkg; do
  base="$(basename "$pkg")"
  # The canonical unit is the dist/*.service with the same basename, if any.
  canonical="$(find daemons -path '*/dist/*.service' -name "$base" 2>/dev/null | head -1)"
  if [ -z "$canonical" ]; then
    skipped=$((skipped + 1))
    continue
  fi
  checked=$((checked + 1))
  if ! diff <(directives "$canonical") <(directives "$pkg") >/dev/null 2>&1; then
    drift=$((drift + 1))
    echo "DRIFT: $base"
    echo "  canonical: $canonical"
    echo "  packaged:  $pkg"
    echo "  --- directive diff (canonical vs packaged) ---"
    diff <(directives "$canonical") <(directives "$pkg") | sed 's/^/  /' || true
    echo
  fi
done < <(find dev/mkosi/mkosi.extra -name '*.service' -not -path '*.wants/*' | sort)

if [ "$drift" -ne 0 ]; then
  echo "FAIL: $drift packaged unit(s) drifted from their dist/ canonical (directives differ)."
  echo "Reconcile the packaged copy under dev/mkosi/mkosi.extra with the canonical daemons/*/dist unit."
  exit 1
fi

echo "OK: $checked packaged unit(s) match their dist/ canonical; $skipped mkosi-only unit(s) skipped."
