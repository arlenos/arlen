#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Watch an app tell the knowledge graph what it is doing.
#
# WHY THIS EXISTS. The graph is this system's headline claim and its app-level
# input was dark: the knowledge daemon has promoted `app.presence.*` and
# `app.timeline.record` into UserAction nodes since they were written, and no app
# had ever sent one. Everything the graph knew about a file came from the eBPF
# sensor, which can see a process open a path and cannot see that the path was
# being EDITED rather than read, in which language, or that a save at 14:03 was
# the moment the work landed.
#
# WHAT IT CHECKS, and it is the CONTENT rather than the delivery. A drive that
# only asserted "something was published" would pass on a window claiming to edit
# the wrong file, so the watcher decodes the payload and this reads the activity,
# the subject and the type off the wire. The path it asserts is the file the
# window was actually given.
#
# WHERE IT STOPS. Promotion into a graph node is the daemon's half and has its own
# tests (`promotion.rs`); the blur clear needs a compositor to take focus away, so
# `app.presence.clear` is not asserted here - what is asserted is that the app
# declares `auto_clear=on-blur`, which is the part this window owns.
#
# Run: dev/screenshot/drive-text-editor-graph.sh [path-to-arlen-text-editor]
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves the
# binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"

app="${1:-$root/target/release/arlen-text-editor}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
work=/tmp/arlen-drive-tegraph
fail=0
bus_pid=""

for b in "$app" "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

cleanup() { [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null; return 0; }
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

rm -rf "$work"
mkdir -p "$work/run/arlen" "$here/out"
sample="$work/sample.rs"
printf 'fn main() {\n    println!("before");\n}\n' > "$sample"

export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

echo "text editor graph input:"

"$bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$producer" ] && break; sleep 0.5; done
[ -S "$producer" ] || { echo "the bus never bound its producer socket:" >&2; tail -3 "$work/bus.log" >&2; exit 2; }

# Subscribe first: the app publishes its presence as soon as the file lands, and
# the bus fans out only to whoever is registered at that moment.
ARLEN_SESSION_ID=drive "$emit" --watch "app." 45 > "$work/watch.log" 2>&1 &
watcher=$!
sleep 1

# Type, then press a real Ctrl+S with the driver, then hold still while the
# watcher collects. The save is what puts the timeline record on the wire, and it
# has to be a save that LANDED - the record is written where the host confirmed
# the write, not beside the keystroke.
cat > "$work/p-type.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
if (!cm) return "no buffer";
cm.focus();
document.execCommand("insertText", false, "// driven\n");
return "typed";
JS
cat > "$work/p-hold.js" <<'JS'
await new Promise(r => setTimeout(r, 15000));
const state = document.querySelector(".savestate");
return JSON.stringify({ state: state && state.textContent.trim() });
JS

got=$(SHOOT_APP_ARGS="$sample" SHOOT_INJECT="$work/p-type.js:$work/p-hold.js" SHOOT_CHORD="Ctrl+s" \
  SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive" \
  "$here/shoot-app.sh" "$app" "$here/out/text-editor-graph.png" 2>&1 \
  | sed -n 's/^inject result: //p')

wait "$watcher"
watched=$(cat "$work/watch.log")

say "the save reached the file on disk" \
  "$(head -1 "$sample" | grep -q "// driven" && echo 1 || echo 0)" \
  "$got (first line: $(head -1 "$sample"))"
say "and it left a picture of text-editor-graph.png" \
  "$([ -s "$here/out/text-editor-graph.png" ] && echo 1 || echo 0)" "$got"

say "the window says it is editing, not merely that it touched a file" \
  "$(printf '%s' "$watched" | grep -q "^saw app.presence.set .*activity=editing" && echo 1 || echo 0)" "$watched"
say "and names the file it was given" \
  "$(printf '%s' "$watched" | grep -q "subject=$sample" && echo 1 || echo 0)" "$watched"
say "and the language, which the sensor could never know" \
  "$(printf '%s' "$watched" | grep -q 'presence.set.*"language": "rust"' && echo 1 || echo 0)" "$watched"
say "and says the claim ends when the window loses focus" \
  "$(printf '%s' "$watched" | grep -q "presence.set.*auto_clear=on-blur" && echo 1 || echo 0)" "$watched"

say "a save that landed is a moment on the timeline" \
  "$(printf '%s' "$watched" | grep -q "^saw app.timeline.record .*type=save" && echo 1 || echo 0)" "$watched"
say "and the timeline names the same file" \
  "$(printf '%s' "$watched" | grep -q "timeline.record.*subject=$sample" && echo 1 || echo 0)" "$watched"
# A TOKEN, not a sentence. A record is written once and read later by a surface
# that may be in another language, so the word belongs to whoever draws it - the
# knowledge app's own timeline stores a message id for exactly this reason.
say "and words nothing, so a German reader is not handed an English one" \
  "$(printf '%s' "$watched" | grep -q "timeline.record.*label=saved" && echo 1 || echo 0)" "$watched"

if [ "$fail" = 0 ]; then
  echo "an app tells the graph what it is doing and what it finished"
else
  echo "graph-input drive FAILED"
fi
exit "$fail"
