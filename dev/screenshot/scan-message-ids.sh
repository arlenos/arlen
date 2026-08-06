#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Render each route and report any message id that reached the screen as text.
#
# The failure this exists for: a store holds `label: "s.sys.none"` because that entry
# is ours to translate, the component renders `option.label` verbatim, and the app
# shows "s.sys.none" to the user. Nothing catches it. The catalog gate only proves
# messages compile, the hardcoded-string lint only looks for English text in source,
# svelte-check sees two strings, and a sibling call site that resolves correctly makes
# the pattern look right wherever you happen to read.
#
# It cannot be caught reliably in source either, because the verbatim render usually
# happens inside a shared component several files away from the store. What the id has
# in common wherever it goes wrong is that it ends up in the rendered text, so that is
# what this looks at.
#
#   dev/screenshot/scan-message-ids.sh http://localhost:5173 / /appearance /keyboard
#
# Exits non-zero if any route showed an id. Start the app's dev server first.
set -uo pipefail

base="${1:?usage: scan-message-ids.sh <base-url> <path> [path...]}"
shift
[ "$#" -gt 0 ] || { echo "give at least one path" >&2; exit 2; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
inject="$(mktemp /tmp/scan-ids-XXXXXX.js)"
shot="$(mktemp /tmp/scan-ids-XXXXXX.png)"
trap 'rm -f "$inject" "$shot"' EXIT

# Match against the actual catalog keys rather than guessing at the shape of an id.
# The first cut used a dotted-lowercase pattern and flagged `org.arlen.files` and
# `api.openai.com` - an app id and a provider host, both of which the UI is supposed
# to show. There is no pattern that separates our ids from reverse-DNS by looking at
# them, and there does not need to be: the set of ids is known exactly.
keys="$(grep -rhoE '^\s*"[a-z][A-Za-z0-9]*\.[A-Za-z0-9._]+":' \
          "$here/../../apps"/*/src/lib/i18n/messages*.ts \
          "$here/../../sdk/ui-kit/src/lib/i18n"/messages*.ts 2>/dev/null \
        | tr -d ' "' | sed 's/:$//' | sort -u | paste -sd, -)"
[ -n "$keys" ] || { echo "found no catalog keys to match against" >&2; exit 2; }

# Returned rather than drawn, so the caller reads text instead of a picture.
cat > "$inject" <<JS
const keys = new Set("$keys".split(","));
const text = document.body.innerText;
return JSON.stringify([...keys].filter((k) => text.includes(k)));
JS

failed=0
for path in "$@"; do
  out=$("$here/shoot.sh" "$base$path" "$shot" "$inject" 2>&1 | sed -n 's/^inject result: //p')
  case "$out" in
    ""|"[]"|"null")
      echo "ok    $path"
      ;;
    *)
      echo "IDS   $path -> $out"
      failed=1
      ;;
  esac
done

echo
if [ "$failed" -ne 0 ]; then
  echo "a message id reached the screen - the store holds an id where the component"
  echo "  renders text, so resolve it at the render site (see themeSystem's sysOptions)"
  exit 1
fi
echo "no message ids on screen in the routes scanned"
