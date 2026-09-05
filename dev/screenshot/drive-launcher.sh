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
server=""

cleanup() {
  [ -n "$server" ] && kill "$server" 2>/dev/null
  rm -rf "$work"
  return 0
}
trap cleanup EXIT

say() {
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

# Refuse a port somebody else is serving rather than photograph their page - the
# lesson the preview helper and the axe sweep both learnt the hard way.
if curl -sf -o /dev/null --max-time 2 "http://localhost:$port/" 2>/dev/null; then
  echo "something is already serving $port; stop it first" >&2
  exit 2
fi
( cd "$app" && exec npx vite dev --port "$port" --strictPort ) >"$work/vite.log" 2>&1 &
server=$!
up=0
for _ in $(seq 1 40); do
  sleep 1
  [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "http://localhost:$port/waypointer" 2>/dev/null)" = "200" ] && { up=1; break; }
done
[ "$up" = 1 ] || { echo "the shell never served $port; see $work/vite.log" >&2; exit 1; }

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
say "a query behind > is offered as something to run" \
  "$(printf '%s' "$cmd" | grep -q '"typed":true' \
     && printf '%s' "$cmd" | grep -qE "Run|Ausf" && echo 1 || echo 0)" "$cmd"
say "and it is not offered to a search engine" \
  "$(case "$cmd" in ""|REFUSED:*) echo 0;; *) printf '%s' "$cmd" | grep -qiE "duckduckgo|google" && echo 0 || echo 1;; esac)" "$cmd"

web=$(typed '"? weather"' "$here/out/launcher-web.png")
say "and a query behind ? goes to a search engine, which it names" \
  "$(printf '%s' "$web" | grep -q '"typed":true' \
     && printf '%s' "$web" | grep -qi "duckduckgo" && echo 1 || echo 0)" "$web"
say "and is not offered to a shell" \
  "$(case "$web" in ""|REFUSED:*) echo 0;; *) printf '%s' "$web" | grep -qE "Shift\+Enter: Terminal" && echo 0 || echo 1;; esac)" "$web"

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

[ "$fail" = 0 ] && echo "the launcher says what its prefixes do, and answers a search that found nothing"
exit "$fail"
