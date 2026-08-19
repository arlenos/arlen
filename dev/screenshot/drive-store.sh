#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the store against a live backend and read what came back.
#
# WHY THIS SCRIPT EXISTS. `store-app.md` says a feature counts as built when a
# row appears, not when a composer returns a non-empty Vec, and this app has
# more of the second than the first. Everything under it is covered by unit
# tests; nothing joined the composer to the grid over a real socket. So this
# drives the whole wire in one go: a backend reading this machine's own
# AppStream metadata, the app's Tauri proxy, and the page that renders it.
#
# It asserts on BOTH ends deliberately. The command answering with rows and the
# grid showing none is the exact failure this app has had, and a probe that only
# reads one of them cannot tell the two apart.
#
# Run: dev/screenshot/drive-store.sh [path-to-arlen-store] [path-to-backend]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-store}"
backend="${2:-$root/target/release/arlen-store-backend}"
run="$HOME/.cache/arlen-drive-store"
fail=0

[ -x "$app" ] || { echo "no store binary at $app"; exit 2; }
[ -x "$backend" ] || { echo "no store backend at $backend"; exit 2; }

rm -rf "$run"
mkdir -p "$run/arlen" "$here/out"
export XDG_RUNTIME_DIR="$run"

"$backend" >"$run/backend.log" 2>&1 &
bpid=$!
trap 'kill "$bpid" 2>/dev/null' EXIT

for _ in $(seq 1 50); do
  [ -S "$run/arlen/store.sock" ] && break
  sleep 0.2
done
if [ ! -S "$run/arlen/store.sock" ]; then
  echo "  FAIL the backend never bound its socket"
  sed -n '1,20p' "$run/backend.log"
  exit 1
fi

say() {  # say <name> <ok> <got>
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

echo "store:"

# One probe, both ends: what the command answers and what the page shows. They
# are reported together because the interesting failure is when they disagree.
cat > "$run/probe.js" <<'JS'
// `withGlobalTauri` is off, so the handle is the internals one, not the API
// module - the same one every other drive script here reaches for.
const invoke = window.__TAURI_INTERNALS__.invoke;
let wire = "";
try {
  const rows = await invoke("store_search", { query: "", facets: [] });
  const first = rows[0] ?? {};
  wire = `rows=${rows.length} keys=${Object.keys(first).sort().join(",")}` +
    ` variantKeys=${Object.keys((first.variants ?? [{}])[0] ?? {}).sort().join(",")}`;
} catch (e) {
  wire = `invoke threw: ${e}`;
}
await new Promise(r => setTimeout(r, 2500));
const dom = (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 300);
const cards = document.querySelectorAll("[data-store-card], article, .card").length;
return `WIRE ${wire} || CARDS ${cards} || DOM ${dom}`;
JS

got=$(SHOOT_INJECT="$run/probe.js" "$here/shoot-app.sh" "$app" "$here/out/store-browse.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# The backend end. A machine with /usr/share/metainfo has hundreds of components
# and zero rows would mean discovery, not rendering, is what is broken.
say "the command answers with catalogue rows" \
  "$(printf '%s' "$got" | grep -qE "rows=[1-9]" && echo 1 || echo 0)" "$got"

# The wire's own shape, named rather than assumed. The app declares the fields it
# reads; a card missing one renders as a blank tile with no error anywhere.
say "a row carries the fields the card reads" \
  "$(printf '%s' "$got" | grep -q "defaultVariant" && printf '%s' "$got" | grep -q "variants" && echo 1 || echo 0)" "$got"

say "a variant carries its own capabilities and trust" \
  "$(printf '%s' "$got" | grep -q "variantKeys=.*capabilities" \
     && printf '%s' "$got" | grep -q "variantKeys=.*trust" && echo 1 || echo 0)" "$got"

# The end the doc actually counts.
say "the grid shows the rows the command answered with" \
  "$(printf '%s' "$got" | grep -qE "CARDS [1-9]" && echo 1 || echo 0)" "$got"

# Searching, to separate two failures that look identical from outside: a page
# that cannot render a live row at all, and one whose LANDING view is built from
# ids that only exist in the fixture. Typing is the shortest path past the
# collections, so if rows appear here the data path is whole and the default view
# is what is wrong.
cat > "$run/probe-search.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
const input = document.querySelector("input[type=search], input");
if (!input) return "no search input";
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
set.call(input, "a");
input.dispatchEvent(new Event("input", { bubbles: true }));
await new Promise(r => setTimeout(r, 1200));
const cards = document.querySelectorAll("[data-store-card], article, .card").length;
return `CARDS ${cards} || DOM ` + (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 240);
JS
got=$(SHOOT_INJECT="$run/probe-search.js" "$here/shoot-app.sh" "$app" "$here/out/store-search.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "searching shows the live catalogue" \
  "$(printf '%s' "$got" | grep -qE "CARDS [1-9]" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "the catalogue reaches the grid over a real socket"
exit "$fail"
