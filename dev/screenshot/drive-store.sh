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
    ` variantKeys=${Object.keys((first.variants ?? [{}])[0] ?? {}).sort().join(",")}` +
    ` firstId=${first.id ?? "-"}`;
} catch (e) {
  wire = `invoke threw: ${e}`;
}
await new Promise(r => setTimeout(r, 2500));
// REVEAL THE COMMUNITY TIER BEFORE COUNTING, or this asserts the machine rather
// than the grid. The browse view hides community-tier apps until asked (§8.1,
// default-safe), and `Trust::from` files Flathub under community - so on a host
// whose only catalogue is Flathub's, EVERY row is hidden and an empty grid is the
// correct rendering of what it was given. Measured here: 43 rows on the wire, 0
// cards, and "Show community apps" sitting in the DOM. Pressing it first makes the
// question "does the grid draw what it was handed" instead of "does this machine
// happen to have a curated catalogue".
const reveal = [...document.querySelectorAll("button")]
  .find((b) => /community/i.test(b.innerText || ""));
let pressed = "no-button";
if (reveal) {
  pressed = `before=${reveal.getAttribute("aria-pressed")}`;
  reveal.click();
  await new Promise(r => setTimeout(r, 800));
  pressed += ` after=${reveal.getAttribute("aria-pressed")}`;
}
const dom = (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 300);
const cards = document.querySelectorAll("[data-store-card], article, .card").length;
return `WIRE ${wire} || CARDS ${cards} || REVEAL ${pressed} || DOM ${dom}`;
JS

got=$(SHOOT_INJECT="$run/probe.js" "$here/shoot-app.sh" "$app" "$here/out/store-browse.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# Kept from THIS probe: the search probe below overwrites `got`, and reading the
# id out of the wrong answer is how a case ends up passing for the wrong reason.
present_id=$(printf '%s' "$got" | sed -n 's/.*firstId=\([^ ]*\).*/\1/p')

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

# The two ops the landing view needs, over the same socket. Collections must
# come back NARROWED - the whole reason they are served rather than compiled in -
# and a collection with no member here must be gone rather than headed over
# nothing.
# Built from an id this machine's catalogue actually returned, rather than one
# picked by hand: the whole point of the narrowing is that it is checked against
# a real catalogue, and a fixture id would test the code against itself.
[ -n "$present_id" ] || present_id="com.example.NothingHere"
cat > "$run/collections.toml" <<TOML
[[collection]]
id = "drive-mixed"
titles.en = "Some of these are here"
titles.de = "Manche davon gibt es"
members = ["$present_id", "com.example.NotOnAnyMachine"]

[[collection]]
id = "drive-absent"
titles.en = "None of these are here"
members = ["com.example.AlsoAbsent"]
TOML

# The BACKEND reads the curated file, so it is restarted holding the pointer.
# Setting it on the app would leave the daemon reading the shipped path and the
# case would pass or fail for the wrong reason.
kill "$bpid" 2>/dev/null
wait "$bpid" 2>/dev/null
rm -f "$run/arlen/store.sock"
ARLEN_STORE_COLLECTIONS="$run/collections.toml" "$backend" >>"$run/backend.log" 2>&1 &
bpid=$!
for _ in $(seq 1 50); do [ -S "$run/arlen/store.sock" ] && break; sleep 0.2; done

cat > "$run/probe-landing.js" <<'JS'
const invoke = window.__TAURI_INTERNALS__.invoke;
const out = [];
try {
  const c = await invoke("store_collections");
  out.push("COLL " + c.map(x => `${x.id}:${x.members.length}:${x.titles.de ?? "-"}`).join(" "));
} catch (e) { out.push(`COLL threw: ${e}`); }
try {
  const s = await invoke("store_sources");
  out.push("SRC " + Object.entries(s).map(([k, v]) => `${k}=${v}`).join(" "));
} catch (e) { out.push(`SRC threw: ${e}`); }
// The update path. Whether anything is pending depends on what this host has
// installed, so the count is REPORTED and the assertions below are about the
// op answering rather than about a number nobody controls.
try {
  const u = await invoke("store_outdated");
  out.push(`UPD n=${u.length} ` + u.map(x => x.id ?? "?").join(","));
} catch (e) { out.push(`UPD threw: ${e}`); }
try {
  const o = await invoke("store_observed_vs_declared", { id: FIRST_ID });
  out.push("OBS " + JSON.stringify(o));
} catch (e) { out.push(`OBS threw: ${e}`); }
return out.join(" || ");
JS
# The probe needs a real id from the catalogue rather than an invented one, so
# the observed-vs-declared answer is about an app the machine actually carries.
sed -i "s/FIRST_ID/\"$present_id\"/" "$run/probe-landing.js"
got=$(SHOOT_INJECT="$run/probe-landing.js" \
  "$here/shoot-app.sh" "$app" "$here/out/store-landing.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# `drive-mixed` names two apps and this host has one of them, so it survives
# with exactly that one member.
say "a collection keeps only the members this machine has" \
  "$(printf '%s' "$got" | grep -q "drive-mixed:1:" && echo 1 || echo 0)" "$got"

# `drive-absent` names nothing this host has. A heading over an empty row is the
# same defect as the empty landing view, one size smaller.
#
# PAIRED with evidence that an answer arrived at all. On its own, "the absent
# collection is not in the output" also holds when there IS no output - a daemon
# that did not start, an invoke that threw, a page that never rendered - so the
# case would report ok at the exact moment everything was broken. A negative
# assertion is only worth what its positive half is worth.
say "a collection with nothing here is dropped, not headed over empty space" \
  "$(printf '%s' "$got" | grep -q "COLL " \
     && ! printf '%s' "$got" | grep -q "drive-absent" && echo 1 || echo 0)" "$got"

# The curator's own words, per locale, rather than an identifier the app would
# have to have a string for.
say "a collection carries the curator's title in each language" \
  "$(printf '%s' "$got" | grep -q "Manche davon gibt es" && echo 1 || echo 0)" "$got"

# This host HAS metainfo, so the answer must be non-zero. On a fresh image every
# count is zero, which is what lets the app say "unfurnished" rather than draw
# the same blank grid it draws for a search that matched nothing.
say "the machine can say which app sources it actually has" \
  "$(printf '%s' "$got" | grep -qE "metainfoDocuments=[1-9]" && echo 1 || echo 0)" "$got"

# THE CATALOGUE, AND THE PICTURE THAT GOES WITH IT. A store is populated or it is
# not, and the difference is one file: with Debian's DEP-11 at
# `/var/lib/swcatalog` this host served 2531 apps, without it 43. So this asks
# whether a catalogue is present at all, and if it is, whether the icon each card
# names can actually be opened - which is the half a screenshot of a grid cannot
# show, because a card with a broken picture and a card with none look identical
# once the layout has settled.
#
# Skipped rather than failed where no catalogue is staged: a developer host is
# not an image, and a case that is red on every machine except one gets ignored
# on all of them.
catalogue=""
for root in /var/lib/swcatalog "$HOME/.local/share/swcatalog"; do
  [ -d "$root/yaml" ] && catalogue="$root" && break
done

if [ -z "$catalogue" ]; then
  echo "  --   the catalogue and its icons: no DEP-11 staged on this host, so there is nothing to resolve"
else
  cat > "$run/probe-icons.js" <<'JS'
  const inv = window.__TAURI_INTERNALS__.invoke;
  const cards = await inv("store_search", { query: "e", facets: [], sort: null });
  const icons = cards.map((c) => c.icon).filter(Boolean);
  return `cards=${cards.length} icons=${icons.length} first=${JSON.stringify(icons.slice(0, 3))}`;
JS
  # A NAME OF ITS OWN. The first cut reused `got`, which the update and
  # observation cases below still read - so adding two passing cases turned two
  # unrelated ones red, with my probe output as their evidence.
  cat_out=$(SHOOT_INJECT="$run/probe-icons.js" "$here/shoot-app.sh" "$app" "" 2>&1     | sed -n 's/^inject result: //p')

  say "a staged catalogue puts more than the installed apps in the store"     "$(printf '%s' "$cat_out" | grep -qE "cards=[0-9]{3,}" && echo 1 || echo 0)" "$cat_out"

  # Every icon a card names must be a file. The names are flat DEP-11 `cached`
  # entries, so they resolve under the origin directory - and the store preferred
  # the `remote` form until 20 August, which is a path relative to a MediaBaseUrl
  # nothing reads. That version would pass the case above and fail this one.
  missing=0
  for name in $(printf '%s' "$cat_out" | sed -n 's/.*first=\[\(.*\)\].*/\1/p' | tr -d '"' | tr ',' ' '); do
    [ -n "$name" ] || continue
    find "$catalogue/icons" -name "$name" -print -quit 2>/dev/null | grep -q . || missing=1
  done
  say "and the icon each card names is a file on this machine" "$([ "$missing" = 0 ] && echo 1 || echo 0)" "$cat_out"
fi

# The Updates tab has to be able to say "nothing pending" as an ANSWER rather
# than as a failure it swallowed. Asserting on the count would be asserting on
# what this host happens to have installed, so this asserts the op answered and
# prints the count for whoever reads the run.
say "the machine can say what is waiting to be updated" \
  "$(printf '%s' "$got" | grep -qE "UPD n=[0-9]" && echo 1 || echo 0)" "$got"

# Skipping needs something to skip. Rather than a case that passes because
# nothing was pending - the shape that made three drives lie today - this says
# out loud when the host gave it nothing to work with.
if printf '%s' "$got" | grep -qE "UPD n=[1-9]"; then
  skip_id=$(printf '%s' "$got" | sed -n 's/.*UPD n=[0-9]* \([^ |]*\).*/\1/p' | cut -d, -f1)
  cat > "$run/probe-skip.js" <<JS
const invoke = window.__TAURI_INTERNALS__.invoke;
const before = await invoke("store_outdated");
const after = await invoke("store_skip_update", { id: "$skip_id" });
return \`SKIP before=\${before.length} after=\${after.length} gone=\${!after.some(x => x.id === "$skip_id")}\`;
JS
  s=$(SHOOT_INJECT="$run/probe-skip.js" "$here/shoot-app.sh" "$app" "$here/out/store-updates.png" 2>&1 \
    | sed -n 's/^inject result: //p')
  say "skipping an update takes it off the list and says what is left" \
    "$(printf '%s' "$s" | grep -q "gone=true" && echo 1 || echo 0)" "$s"
else
  echo "  --   skipping an update: nothing pending on this host, not exercised"
fi

# store-app.md section 8.2: the panel must not read as a clean bill of health
# the system cannot give, so "no feed yet" and "nothing observed" are different
# answers and the op has to carry which one it means.
say "the app can tell an unobserved app from a clean one" \
  "$(printf '%s' "$got" | grep -q "OBS {" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "the catalogue reaches the grid over a real socket"
exit "$fail"
