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
#   dev/screenshot/scan-message-ids.sh http://localhost:1421 / /appearance /keyboard
#
# The port is the app's, from its vite config - settings is 1421, not the 5173 a
# SvelteKit app uses by default.
#
# Exits non-zero if any route showed an id. Start the app's dev server first.
set -uo pipefail

base="${1:?usage: scan-message-ids.sh <base-url> <path> [path...]}"

# The first argument is a BASE URL, not an app name. Passing "settings" makes every
# load "settings/appearance/..." - not a URL, no page, and an empty document that
# reports zero message ids. On 6 August every run in this session was that: a clean
# result from a page that never existed. A check whose happy path is
# indistinguishable from never running is worse than no check.
case "$base" in
  http://*|https://*|file://*) ;;
  *)
    echo "scan-message-ids.sh: '$base' is not a base URL." >&2
    echo "Pass where the app is actually served, e.g. http://localhost:1421" >&2
    exit 2
    ;;
esac
shift
[ "$#" -gt 0 ] || { echo "give at least one path" >&2; exit 2; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
inject="$(mktemp /tmp/scan-ids-XXXXXX.js)"
shot="$(mktemp /tmp/scan-ids-XXXXXX.png)"

# Match against the actual catalog keys rather than guessing at the shape of an id.
# The first cut used a dotted-lowercase pattern and flagged `org.arlen.files` and
# `api.openai.com` - an app id and a provider host, both of which the UI is supposed
# to show. There is no pattern that separates our ids from reverse-DNS by looking at
# them, and there does not need to be: the set of ids is known exactly.
# `-` belongs in the class: `s.idx.system-actions.volume.title` is a real id, and
# without it twelve of them were invisible to this scan - the same character class
# hid four registry ids and twelve catalog entries from me the same evening. An id
# this cannot see is an id it can never report on screen.
keys="$(grep -rhoE '^\s*"[a-z][A-Za-z0-9]*\.[A-Za-z0-9._-]+":' \
          "$here/../../apps"/*/src/lib/i18n/messages*.ts \
          "$here/../../sdk/ui-kit/src/lib/i18n"/messages*.ts 2>/dev/null \
        | tr -d ' "' | sed 's/:$//' | sort -u | paste -sd, -)"
[ -n "$keys" ] || { echo "found no catalog keys to match against" >&2; exit 2; }

# Returned rather than drawn, so the caller reads text instead of a picture.
#
# The delimiter is quoted, so nothing in here is shell. It was not, and a pair of
# backticks around a message id in a comment below became a command substitution:
# the shell tried to run `f.places.places` on every invocation. Prose in this file
# is full of backticked identifiers, so the only durable fix is to stop the shell
# reading the block at all and substitute the one value afterwards.
cat > "$inject" <<'JS'
const keys = new Set("__KEYS__".split(","));
// Visible text AND the attributes that are read aloud or shown on hover. An
// untranslated aria-label is invisible to innerText, so the shell's ten hardcoded
// accessible names could never have been caught here - a screen reader would have
// been the only way to notice, which is exactly the user this check is for.
const attrs = [...document.querySelectorAll("[aria-label],[title],[placeholder],[alt]")]
  .flatMap((el) => ["aria-label", "title", "placeholder", "alt"].map((a) => el.getAttribute(a) || ""))
  .join("\n");
const body = document.body.innerText;
// Whether the app's backend was reachable at all.
//
// Under a plain vite server there is no Tauri, so every command throws and
// everything those commands feed is simply absent from the page. The scan then
// finds no ids for the best possible reason and the worst: there was nothing to
// look at. On 6 August a raw id was planted as a Files sidebar heading and a
// scan of that very route came back clean, because the sidebar had rendered
// nothing at all. Saying "ok" there is a lie of omission.
const tauri = typeof window.__TAURI_INTERNALS__ !== "undefined";
// An empty document scans clean for the same reason a missing one does. Say so
// rather than pass: this is the shape that made a whole session's runs vacuous.
if (body.trim().length === 0 && attrs.length === 0)
  return JSON.stringify({ tauri: false, ids: ["<page rendered nothing>"] });
const text = body + "\n" + attrs;
return JSON.stringify({ tauri, ids: [...keys].filter((k) => text.includes(k)) });
JS
# `|` as the delimiter: a catalog key cannot contain one, and keys are the only
# thing being substituted.
sed -i "s|__KEYS__|$keys|" "$inject"

# The positive control, run before anything is believed.
#
# On 6 August this script reported every route clean for a whole session while
# loading no page at all. It was internally consistent, exited 0, and printed a
# sentence that read like a finding; nothing about reading it would have shown the
# bug. What found it was planting a message id and noticing it was not caught.
#
# So it now plants one on itself, every run. A page carrying a real catalog id in
# body text and in an aria-label must come back with both. If it does not, this
# script cannot see what it exists to see, and it says so rather than going on to
# report clean routes. A checker is not trusted until it has been shown to fail.
control_id="${keys%%,*}"
control="$(mktemp -d /tmp/scan-ids-ctl-XXXXXX)"
trap 'rm -f "$inject" "$shot"; rm -rf "$control"' EXIT
cat > "$control/index.html" <<HTML
<!doctype html><meta charset="utf-8"><title>positive control</title>
<p>$control_id</p>
<button aria-label="$control_id">x</button>
HTML

ctl=$("$here/shoot.sh" "file://$control/index.html" "$shot" "$inject" 2>&1 | sed -n 's/^inject result: //p')
case "$ctl" in
  *"$control_id"*) ;;
  *)
    echo "scan-message-ids.sh: the positive control was not caught." >&2
    echo "  planted: $control_id (in body text and in an aria-label)" >&2
    echo "  got:     ${ctl:-<no result>}" >&2
    echo "This script cannot see a message id on a page, so a clean result from it" >&2
    echo "would mean nothing. Fix the scan before trusting any route." >&2
    exit 2
    ;;
esac

failed=0
for spec in "$@"; do
  # `path::selector` opens something before scanning. Everything behind a click was
  # outside this check: the mint dialog's routes reported clean while the dialog was
  # never opened, and a clean result about a component that did not render is the
  # same false green in a new place.
  path="${spec%%::*}"
  open="${spec#*::}"
  [ "$open" = "$spec" ] && open=""
  out=$(SHOOT_OPEN="$open" "$here/shoot.sh" "$base$path" "$shot" "$inject" 2>&1 | sed -n 's/^inject result: //p')
  ids="$(printf '%s' "$out" | sed -n 's/.*"ids":\[\(.*\)\]}/\1/p')"
  tauri="$(printf '%s' "$out" | grep -c '"tauri":true')"
  case "$out" in
    "")
      # No result at all means the injected script did not run: a throw, a page
      # that never loaded, a harness change. It does NOT mean the page was clean,
      # and reporting it as clean is how this check quietly stops checking. It
      # read as ok until 6 August, when a planted id in an aria-label came back
      # clean and the reason turned out to be an empty result, not a clean page.
      echo "FAIL  $spec -> the page returned no result; the check did not run"
      failed=1
      ;;
    *'"ids":[]'*|"null")
      if [ "$tauri" -eq 1 ]; then
        echo "ok    $spec"
      else
        # Clean, but only about what was on the page. Anything a Tauri command
        # would have supplied is missing here, so this route is under-covered
        # rather than proven, and it says which.
        echo "ok    $spec  (frontend only; no Tauri, command-fed content unseen)"
      fi
      ;;
    *)
      echo "IDS   $spec -> [$ids]"
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
