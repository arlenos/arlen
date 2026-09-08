#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the launcher and type into it.
#
# WHY THIS EXISTS. Twenty drives press twenty apps, and none of them pressed the
# surface every session starts at. The launcher is the first thing a person
# touches after logging in and the only way most of them will ever open an app,
# and until now the only thing that had ever looked at it was somebody taking a
# screenshot by hand.
#
# WHAT IT CAN HONESTLY CHECK. Under a plain vite server there is no Tauri, so app
# search, the shell runner and the agent all reject - which is correct and is not
# what this checks. What is real without a backend is the launcher's own
# behaviour: the hints it offers at rest, the prefixes that change what a query
# MEANS (`>` a command, `?` a web search), and the sentence for a search that
# genuinely matched nothing. The app ships DEV fixtures for the last one
# (`?searchmock=empty`) precisely so it can be told apart from a refused search,
# which is the distinction this asserts.
#
# Run: dev/screenshot/drive-launcher.sh
#
# It needs `vite dev` rather than a preview: the fixtures are DEV-gated.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="$root/apps/desktop-shell"
port=1420
fail=0
work="$(mktemp -d)"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$here/lib/preview.sh"

cleanup() {
  # Was `kill $server`, which is npx - and the node npx spawns is what holds the
  # port, so a run left a dev server listening for the next one to mistake for
  # its own. The helper kills the process GROUP and then checks the port went
  # quiet, which is the whole reason it exists.
  stop_preview
  rm -rf "$work"
  return 0
}
trap cleanup EXIT

say() {
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

# The helper refuses a port it did not start, for the reason its header gives:
# a leftover server serves a frontend from whenever it was built, and every
# assertion after that is about somebody else's page.
start_dev "$app" "$port" || exit 2
up=0
for _ in $(seq 1 40); do
  sleep 1
  [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "http://localhost:$port/waypointer" 2>/dev/null)" = "200" ] && { up=1; break; }
done
[ "$up" = 1 ] || { echo "the shell never served $port" >&2; exit 1; }

echo "launcher:"

# The resting state. No query typed, so what it offers is the vocabulary itself.
cat > "$work/rest.js" <<'JS'
await new Promise((r) => setTimeout(r, 3000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200);
JS
rest=$("$here/shoot.sh" "http://localhost:$port/waypointer" "$here/out/launcher-rest.png" "$work/rest.js" 2>&1 \
  | sed -n 's/^inject result: //p')

say "at rest it says what a prefix does, rather than an empty box" \
  "$(printf '%s' "$rest" | grep -q "command" && printf '%s' "$rest" | grep -q "manual" \
     && printf '%s' "$rest" | grep -q "web search" && echo 1 || echo 0)" "$rest"

# A typed query. The prefix decides what the words MEAN, and getting that wrong
# is how a launcher runs a shell command somebody meant as a search.
typed() {  # typed <query> <out.png>
  cat > "$work/type.js" <<JS
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(2500);
const input = document.querySelector("input");
if (!input) return JSON.stringify({ typed: false });
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
set.set.call(input, ${1});
input.dispatchEvent(new Event("input", { bubbles: true }));
await wait(1800);
return JSON.stringify({ typed: true, text: document.body.innerText.replace(/\\s+/g, " ").trim().slice(0, 220) });
JS
  "$here/shoot.sh" "http://localhost:$port/waypointer" "$2" "$work/type.js" 2>&1 | sed -n 's/^inject result: //p'
}

# WHAT THE SURFACE SAYS, not what I assumed it would. The first cut of these two
# looked for the words "command" and "web", and both went red against a launcher
# that was behaving correctly: it offers `Enter: Run / Shift+Enter: Terminal` for
# a command and names the engine for a search, which is more use than either word
# I had imagined. So they assert the MEANING instead - the prefix decides what
# the words are for, and the wrong offer is the failure worth catching.
cmd=$(typed '"> uname -a"' "$here/out/launcher-command.png")
# CASE-INSENSITIVE, and both of these were wrong for it in opposite directions.
# The hint reads `Enter: run. Shift+Enter: terminal.` in sentence case; these
# matched `Run` and `Terminal` capitalised, so this one went RED against a
# launcher doing exactly the right thing, and the negative one below went GREEN
# because it could no longer find the string it exists to forbid. A false red
# wastes an hour; a false green is the check quietly switching itself off, and
# they came from the same character.
say "a query behind > is offered as something to run" \
  "$(printf '%s' "$cmd" | grep -q '"typed":true' \
     && printf '%s' "$cmd" | grep -qiE "enter: run|ausf" && echo 1 || echo 0)" "$cmd"
say "and it is not offered to a search engine" \
  "$(case "$cmd" in ""|REFUSED:*) echo 0;; *) printf '%s' "$cmd" | grep -qiE "duckduckgo|google" && echo 0 || echo 1;; esac)" "$cmd"

web=$(typed '"? weather"' "$here/out/launcher-web.png")
say "and a query behind ? goes to a search engine, which it names" \
  "$(printf '%s' "$web" | grep -q '"typed":true' \
     && printf '%s' "$web" | grep -qi "duckduckgo" && echo 1 || echo 0)" "$web"
say "and is not offered to a shell" \
  "$(case "$web" in ""|REFUSED:*) echo 0;; *) printf '%s' "$web" | grep -qiE "shift\+enter: terminal" && echo 0 || echo 1;; esac)" "$web"

# THE TWO EMPTIES, and telling them apart is the point. Under vite every provider
# rejects, so the launcher's empty line always reads as the refused one - which is
# why the app carries `?searchmock=empty`, answering with nothing instead of
# refusing. Without it the sentence for "your search matched nothing" cannot be
# looked at at all.
cat > "$work/empty.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(2500);
const input = document.querySelector("input");
if (!input) return JSON.stringify({ typed: false });
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
set.set.call(input, "zzzqqq");
input.dispatchEvent(new Event("input", { bubbles: true }));
await wait(2000);
return JSON.stringify({ typed: true, text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200) });
JS
empty=$("$here/shoot.sh" "http://localhost:$port/waypointer?searchmock=empty" \
  "$here/out/launcher-empty.png" "$work/empty.js" 2>&1 | sed -n 's/^inject result: //p')

say "a search that matched nothing says so" \
  "$(printf '%s' "$empty" | grep -qiE "No results|Nichts gefunden|keine Ergebnisse" && echo 1 || echo 0)" "$empty"

# THE EXTENSIONS SECTION, which until 8 September could not draw at all: the
# command that reaches the module runtime had no caller, so a module could ship,
# enable and answer on its socket without ever putting a row on a screen. The
# daemon side has its own tests; what only a render can say is whether an
# installed extension's answer arrives looking like an answer, under its own
# heading, beside the builtins rather than pretending to be one.
cat > "$work/ext.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(2500);
const input = document.querySelector("input");
if (!input) return JSON.stringify({ typed: false });
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
set.set.call(input, "u:heart");
input.dispatchEvent(new Event("input", { bubbles: true }));
await wait(2000);
return JSON.stringify({ typed: true, text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 260) });
JS
ext=$("$here/shoot.sh" "http://localhost:$port/waypointer?searchmock=extensions" \
  "$here/out/launcher-extensions.png" "$work/ext.js" 2>&1 | sed -n 's/^inject result: //p')

say "an installed extension's answer reaches the list" \
  "$(printf '%s' "$ext" | grep -q '"typed":true' \
     && printf '%s' "$ext" | grep -qi "HEAVY BLACK HEART" && echo 1 || echo 0)" "$ext"
say "and it is under its own heading, not among the applications" \
  "$(printf '%s' "$ext" | grep -qiE "Extensions|Erweiterungen" && echo 1 || echo 0)" "$ext"

# THE KEYWORD IS A DOOR, NOT A SECOND ANSWER. `unicode ` used to be its own
# command, its own store and its own section; it now rewrites to the module's
# `u:` prefix and falls through. This discriminates: if the rewrite were gone the
# old branch would set a special mode and never fan out, so the extensions
# section would stay empty no matter what the fixture answers.
cat > "$work/keyword.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(2500);
const input = document.querySelector("input");
if (!input) return JSON.stringify({ typed: false });
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
set.set.call(input, "unicode heart");
input.dispatchEvent(new Event("input", { bubbles: true }));
await wait(2000);
return JSON.stringify({ typed: true, text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 260) });
JS
kw=$("$here/shoot.sh" "http://localhost:$port/waypointer?searchmock=extensions" \
  "$here/out/launcher-unicode-keyword.png" "$work/keyword.js" 2>&1 | sed -n 's/^inject result: //p')

say "the unicode keyword reaches the same answer the prefix does" \
  "$(printf '%s' "$kw" | grep -q '"typed":true' \
     && printf '%s' "$kw" | grep -qi "HEAVY BLACK HEART" && echo 1 || echo 0)" "$kw"

# THE TIER 2 WORKER POOL AND THE CARD IT WAS SAID TO STRETCH. The pool sat off
# behind "PERMANENTLY OFF until it can be initialised without taking over the
# Waypointer's flex layout", and the state that sentence describes could not be
# reached at all without a daemon - the discovery call rejects and the pool does
# nothing. `?searchmock=tier2` reports one eligible module so the host and its
# iframe really mount. The card is 600x74 either way; this fails if it grows.
cat > "$work/tier2.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(3000);
const card = document.querySelector(".wp-card");
if (!card) return JSON.stringify({ card: false });
const r = card.getBoundingClientRect();
return JSON.stringify({
  card: true,
  height: Math.round(r.height),
  fillsViewport: r.height > window.innerHeight * 0.5,
  hosts: document.querySelectorAll("[data-arlen-role='module-worker-host']").length,
});
JS
t2=$("$here/shoot.sh" "http://localhost:$port/waypointer?searchmock=tier2" \
  "$here/out/launcher-tier2.png" "$work/tier2.js" 2>&1 | sed -n 's/^inject result: //p')

say "a mounted Tier 2 worker does not take over the card" \
  "$(printf '%s' "$t2" | grep -q '"hosts":1' \
     && printf '%s' "$t2" | grep -q '"fillsViewport":false' && echo 1 || echo 0)" "$t2"

[ "$fail" = 0 ] && echo "the launcher says what its prefixes do, answers a search that found nothing, and shows what an extension found"
exit "$fail"
