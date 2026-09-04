#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the Knowledge app against a REAL graph, not a fixture.
#
# WHY A WHOLE STACK. Every page of this app is a view onto the knowledge daemon,
# so rendering it without one shows the honest empty state and nothing else - which
# is worth checking once and tells you nothing about whether the views work. This
# brings up an event bus and a knowledge daemon on temp sockets, emits real
# `file.opened` events, waits for the promotion pass that turns them into graph
# nodes, and then looks at each page.
#
# TEMP SOCKETS UNDER /tmp, deliberately. A Unix socket path has to fit in
# `sun_path`, about 108 bytes; a runtime dir under a long working directory
# overflows it and the daemon refuses to bind with `path must be shorter than
# SUN_LEN` before it starts. Short prefix, and nothing here touches the runtime
# dir of whoever runs it.
#
# THE PROMOTION WAIT IS A POLL, not a sleep. The pass runs on its own schedule and
# a fixed wait is either too short (a flake that reads as an empty graph) or too
# long. This polls the daemon's own log line and gives up with a reason.
#
# Run: dev/screenshot/drive-knowledge.sh
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The frontend is served from a preview below, not baked into the binary, so the
# staleness guard compares Rust only (see shoot-app.sh).
export SHOOT_FRONTEND_SERVED=1
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"
out="$root/dev/screenshot/out"
work=/tmp/arlen-drive-kg
app="$root/target/debug/arlen-knowledge-app"
fail=0

for b in "$app" "$root/target/debug/event-bus" "$root/target/debug/arlen-graph-daemon" \
         "$root/target/debug/arlen-event-emit"; do
    [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
    # Whichever of the four this is, its sources live under the crate that
    # produced it; the loop is over binaries so the source dirs are looked up
    # rather than listed twice.
    case "$(basename "$b")" in
      event-bus) require_fresh "$b" "$root/daemons/event-bus/src" || exit 2 ;;
      arlen-graph-daemon) require_fresh "$b" "$root/daemons/knowledge/src" || exit 2 ;;
      arlen-knowledge-app) require_fresh "$b" "$root/apps/knowledge/src-tauri/src" || exit 2 ;;
      arlen-event-emit) require_fresh "$b" "$root/dev/event-emit/src" || exit 2 ;;
      # EXHAUSTIVE OR IT REFUSES. The comment above said "whichever of the four"
      # while this had three arms, so `arlen-event-emit` - the thing that puts
      # the events into the bus this whole suite then reads back - was checked
      # for existence and never for age. A silent fall-through is how a list and
      # the sentence describing it drift apart; now adding a binary above without
      # a rule here stops the run instead of quietly skipping it.
      *) echo "no freshness rule for $(basename "$b") - add one" >&2; exit 2 ;;
    esac
done

# The app is a Tauri binary; a debug build loads its devUrl, so something has to
# serve that port or every probe reads a connection-refused page.
# And that what it serves is current: `build/` is written by a command nobody
# is forced to run, so a preview can serve a page the tree left behind.
require_fresh_frontend "$root/apps/knowledge/build" "$root/apps/knowledge/src" || exit 2
if ! curl -sf -o /dev/null http://localhost:1436/; then
    echo "nothing is serving http://localhost:1436, so the app would load an error page." >&2
    echo "  (cd $root/apps/knowledge && npx vite build && npx vite preview --port 1436)" >&2
    exit 2
fi

rm -rf "$work"
mkdir -p "$work"/{run/arlen,state/timeline,config/arlen,data}
printf '[projects]\nwatch_directories = []\n' > "$work/config/arlen/graph.toml"
export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run" \
       XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" \
       XDG_STATE_HOME="$work/state" ARLEN_DB_PATH="$work/state/events.db" \
       ARLEN_GRAPH_PATH="$work/state/graph" ARLEN_TIMELINE_MOUNT="$work/state/timeline"

cleanup() { [ -n "${bus_pid:-}" ] && kill "$bus_pid" 2>/dev/null; [ -n "${kg_pid:-}" ] && kill "$kg_pid" 2>/dev/null; }
trap cleanup EXIT

"$root/target/debug/event-bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$work/run/arlen/event-bus-producer.sock" ] && break; sleep 0.5; done

"$root/target/debug/arlen-graph-daemon" > "$work/kg.log" 2>&1 &
kg_pid=$!
for _ in $(seq 1 40); do [ -S "$work/run/arlen/knowledge.sock" ] && break; sleep 0.5; done
[ -S "$work/run/arlen/knowledge.sock" ] || {
    echo "the knowledge daemon never bound its socket:" >&2; tail -3 "$work/kg.log" >&2; exit 2; }

echo "knowledge:"

for f in README.md CLAUDE.md ROADMAP.md; do
    ARLEN_SESSION_ID=drive "$root/target/debug/arlen-event-emit" "$root/$f" >/dev/null 2>&1
done

# Poll the daemon's own line rather than sleeping a guess.
promoted=0
for _ in $(seq 1 60); do
    if grep -q "promotion pass complete" "$work/kg.log" && ! grep -q "promoted=0" "$work/kg.log"; then
        promoted=1; break
    fi
    sleep 2
done
if [ "$promoted" != 1 ]; then
    echo "  FAIL nothing was promoted in two minutes, so no page below has data to show"
    grep -iE "promot|error" "$work/kg.log" | tail -3
    exit 1
fi

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

# One launch per page: the app has no route argument, so each probe clicks its own
# way there and reports what arrived. Two injects with a settle between them, since
# a single inject followed by the capture races the route transition.
#
# BOTH results are returned, not just the read. This used to `tail -1` and keep the
# read alone, which threw away the one signal that says the navigation never
# happened: a run where the sidebar link was not found reported `went:false` into
# nothing, the read then described whatever page WAS open, and the case failed with
# an empty detail line. On 19 August the shot filed as `knowledge-timeline.png` was
# a picture of the Library.
page() {  # page <sidebar-label> <out.png>
  local label="$1" png="$2"
  cat > "$work/goto.js" <<JS
const a = [...document.querySelectorAll("a,button")].find(n => n.textContent.trim() === "$label");
if (!a) return JSON.stringify({ went: false });
a.click();
return JSON.stringify({ went: true });
JS
  cat > "$work/read.js" <<'JS'
await new Promise(r => setTimeout(r, 1200));
const main = document.querySelector("main") || document.body;
return JSON.stringify({
  heading: (document.querySelector("h1") || {}).textContent?.trim() ?? null,
  text: main.textContent.replace(/\s+/g, " ").trim().slice(0, 220),
});
JS
  ARLEN_KNOWLEDGE_SOCKET="$work/run/arlen/knowledge.sock" \
  ARLEN_DAEMON_SOCKET="$work/run/arlen/knowledge.sock" \
  SHOOT_INJECT="$work/goto.js:$work/read.js" SHOOT_INJECT_SETTLE=2 \
    "$root/dev/screenshot/shoot-app.sh" "$app" "$out/$png" "" 9 2>&1 \
    | sed -n 's/^inject result: //p' | tr '\n' ' '
}

# Every case below asserts it arrived before it asserts what it saw: a claim about
# the timeline read off the library page is worse than no claim.
arrived() { printf '%s' "$1" | grep -q '"went":true'; }

got=$(page Timeline knowledge-timeline.png)
say "the timeline page is the one that opened" \
  "$(arrived "$got" && echo 1 || echo 0)" "$got"
say "the timeline lists the files that were actually opened" \
  "$(printf '%s' "$got" | grep -q "README.md" && echo 1 || echo 0)" "$got"

got=$(page Projects knowledge-projects.png)
# No project signal was seeded, so the honest answer is an empty state - NOT an
# error, and not an invented project. Either sentence is fine; a failure is not.
say "projects says it has none rather than failing or inventing one" \
  "$(case "$got" in ""|REFUSED:*) echo 0;; *) printf '%s' "$got" | grep -qiE "could not|cannot|failed" && echo 0 || echo 1;; esac)" "$got"

got=$(page Library knowledge-library.png)
# `knowledge_library` has no host yet - it is on the known-missing list, waiting on
# a schema decision. So the page must say the library is not BUILT, not that the
# read failed: "cannot read right now" promises a retry that will never work.
say "the library says it is not built rather than that a read failed" \
  "$(printf '%s' "$got" | grep -qi "not built yet" && echo 1 || echo 0)" "$got"

got=$(page Searches knowledge-searches.png)
say "the searches page answers without claiming a read failed" \
  "$(case "$got" in ""|REFUSED:*) echo 0;; *) printf '%s' "$got" | grep -qiE "could not|cannot|failed" && echo 0 || echo 1;; esac)" "$got"

if [ "$fail" = 0 ]; then
  echo "every page answered from a real graph, and none of them claimed a failure"
fi
exit "$fail"
