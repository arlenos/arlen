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
# Run: dev/screenshot/drive-text-editor-graph.sh [text-editor] [viewers] [pdf] [files]
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

cleanup() {
  [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null
  rm -f "$work.lock"
  return 0
}
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

# ONE AT A TIME. The runtime dir is a fixed path (a socket has to fit in
# `sun_path`, so it cannot be a mktemp under a long checkout), which means a
# second run wipes the first one's state and binds a bus at the same address -
# and the two runs then read each other's events. That happened once and the
# output was a confusing half-failure that looked like a product defect.
#
# A PID IN A LOCK FILE, not the presence of the socket: the socket FILE outlives
# the run (the trap kills the bus, the inode stays), so a stale one refused a
# perfectly good run the first time this guard was written. The question is
# whether a process is still here, and a pid answers it.
lock="$work.lock"
if [ -f "$lock" ] && kill -0 "$(cat "$lock" 2>/dev/null)" 2>/dev/null; then
  echo "another run of this drive is live (pid $(cat "$lock")); wait for it" >&2
  exit 2
fi

rm -rf "$work"
mkdir -p "$work/run/arlen" "$here/out"
echo "$$" > "$lock"
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

# ─────────────────────────────────────────────────────────────────────
# THE VIEWER, on the same two surfaces and with a different verb. Kept in this
# file rather than a second one because the thing under test is the SURFACE, and
# the interesting assertion is that two apps say DIFFERENT things through it: an
# editor edits, a viewer views, and a graph that could not tell them apart would
# be back to what the sensor already knew.
viewer="${2:-$root/target/release/arlen-viewers}"
if [ -x "$viewer" ]; then
  picture="$work/shot.png"
  # A REAL PNG, from base64 rather than hand-rolled bytes. The first version of
  # this fixture had a wrong IDAT checksum, so the viewer never decoded it,
  # `loaded` stayed null and the presence went out with no kind - which the
  # assertion below then correctly failed on. A fixture that cannot be opened
  # tests the failure path by accident.
  base64 -d > "$picture" <<'B64'
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==
B64

  ARLEN_SESSION_ID=drive "$emit" --watch "app.presence." 25 > "$work/watch-viewer.log" 2>&1 &
  vwatcher=$!
  sleep 1
  cat > "$work/p-viewer.js" <<'JS'
await new Promise(r => setTimeout(r, 12000));
return "held";
JS
  SHOOT_APP_ARGS="$picture" SHOOT_INJECT="$work/p-viewer.js"     SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive"     "$here/shoot-app.sh" "$viewer" "$here/out/viewer-graph.png" > "$work/shoot-viewer.log" 2>&1
  wait "$vwatcher"
  vwatched=$(cat "$work/watch-viewer.log")

  say "the viewer says it is viewing, and the editor said editing" \
    "$(printf '%s' "$vwatched" | grep -q "^saw app.presence.set .*activity=viewing" && echo 1 || echo 0)" "$vwatched"
  say "and names the picture it was given" \
    "$(printf '%s' "$vwatched" | grep -q "subject=$picture" && echo 1 || echo 0)" "$vwatched"
  say "and the kind, so a reader need not infer it from the extension" \
    "$(printf '%s' "$vwatched" | grep -q 'presence.set.*"kind": "image"' && echo 1 || echo 0)" "$vwatched"
else
  echo "  --   no viewers binary at $viewer, so its half was not driven"
fi

# ─────────────────────────────────────────────────────────────────────
# THE PDF READER, which publishes presence and deliberately no timeline record.
# The third verb through the same surface, and the one whose metadata the sensor
# could never reach: a hundred-page report and a one-page receipt are the same
# `openat` to a kernel probe, so the LENGTH is the assertion that matters here.
reader="${3:-$root/target/release/arlen-pdf-app}"
if [ -x "$reader" ]; then
  doc="$work/three.pdf"
  # A real three-page PDF, written as a literal rather than through a library so
  # the file the reader opens is the file this script describes - the same reason
  # `drive-pdf.sh` builds its own.
  python3 - "$doc" <<'PYPDF'
import sys
pages = 3
objs, page_ids = {}, []
n = 4
for _ in range(pages):
    page_ids.append(n)
    objs[n] = f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents {n+1} 0 R /Resources << /Font << /F1 3 0 R >> >> >>"
    body = b"BT /F1 12 Tf 20 100 Td (page) Tj ET"
    objs[n + 1] = f"<< /Length {len(body)} >>\nstream\n{body.decode()}\nendstream"
    n += 2
objs[1] = "<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = "<< /Type /Pages /Kids [" + " ".join(f"{i} 0 R" for i in page_ids) + f"] /Count {pages} >>"
objs[3] = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
out, offsets = b"%PDF-1.4\n", {}
for k in sorted(objs):
    offsets[k] = len(out)
    out += f"{k} 0 obj\n{objs[k]}\nendobj\n".encode()
start = len(out)
out += f"xref\n0 {max(objs)+1}\n0000000000 65535 f \n".encode()
for k in sorted(objs):
    out += f"{offsets[k]:010d} 00000 n \n".encode()
out += f"trailer\n<< /Size {max(objs)+1} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n".encode()
open(sys.argv[1], "wb").write(out)
PYPDF

  ARLEN_SESSION_ID=drive "$emit" --watch "app.presence." 25 > "$work/watch-pdf.log" 2>&1 &
  pwatcher=$!
  sleep 1
  cat > "$work/p-pdf.js" <<'JS'
await new Promise(r => setTimeout(r, 12000));
return "held";
JS
  SHOOT_APP_ARGS="$doc" SHOOT_INJECT="$work/p-pdf.js"     SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive"     "$here/shoot-app.sh" "$reader" "$here/out/pdf-graph.png" > "$work/shoot-pdf.log" 2>&1
  wait "$pwatcher"
  pwatched=$(cat "$work/watch-pdf.log")

  say "the reader says it is reading, a third verb through the same surface" \
    "$(printf '%s' "$pwatched" | grep -q "^saw app.presence.set .*activity=reading" && echo 1 || echo 0)" "$pwatched"
  say "and how long the document is, which no kernel probe can see" \
    "$(printf '%s' "$pwatched" | grep -q 'presence.set.*"pages": "3"' && echo 1 || echo 0)" "$pwatched"
else
  echo "  --   no pdf binary at $reader, so its half was not driven"
fi

# ─────────────────────────────────────────────────────────────────────
# THE FILE MANAGER, which has the most to say of any app here: it is where the
# gap between a trace and a history is widest. The sensor sees the syscalls a
# copy makes and cannot see that they were ONE copy somebody asked for.
#
# The op is driven the way `drive-menu-relay.sh` drives it - a real menu click on
# the bus - so the record under test is one an actual press produced.
manager="${4:-$root/target/release/arlen-files}"
if [ -x "$manager" ]; then
  fwork="$HOME/arlen-drive-graph-files"
  rm -rf "$fwork"; mkdir -p "$fwork"
  printf 'anything\n' > "$fwork/a-file.txt"

  ARLEN_SESSION_ID=drive "$emit" --watch "app." 40 > "$work/watch-files.log" 2>&1 &
  fwatcher=$!
  sleep 1
  ( for _ in $(seq 1 10); do
      sleep 3
      ARLEN_SESSION_ID=drive "$emit" --menu dev.arlen.files file.new_folder >> "$work/emit-files.log" 2>&1
      [ -n "$(find "$fwork" -mindepth 1 -maxdepth 1 -type d -not -name '.*' 2>/dev/null)" ] && break
    done ) &
  clicker=$!
  cat > "$work/p-files.js" <<'JS'
await new Promise(r => setTimeout(r, 33000));
return "held";
JS
  # HOME at the fixture, the way the menu-relay drive does it: the file manager
  # opens at whatever the backend calls home, so this puts the window where the
  # click has to land and keeps a mis-navigated new folder out of the real one.
  SHOOT_INJECT="$work/p-files.js" \
    SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive;HOME=$fwork" \
    "$here/shoot-app.sh" "$manager" "$here/out/files-graph.png" > "$work/shoot-files.log" 2>&1
  wait "$clicker"
  wait "$fwatcher"
  fwatched=$(cat "$work/watch-files.log")

  say "the file manager says where the person is" \
    "$(printf '%s' "$fwatched" | grep -q "^saw app.presence.set .*activity=browsing" && echo 1 || echo 0)" "$fwatched"
  say "and names the folder in front of them" \
    "$(printf '%s' "$fwatched" | grep -q "presence.set.*subject=$fwork" && echo 1 || echo 0)" "$fwatched"
  # THE OP. A new folder really appeared on disk, and the graph was told it was
  # one thing somebody did rather than a mkdir.
  say "a new folder that landed is a moment on the timeline" \
    "$(printf '%s' "$fwatched" | grep -q "^saw app.timeline.record .*type=new_folder" && echo 1 || echo 0)" "$fwatched"
  say "and the folder is really there" \
    "$([ -n "$(find "$fwork" -mindepth 1 -maxdepth 1 -type d -not -name '.*' 2>/dev/null)" ] && echo 1 || echo 0)" \
    "$(ls -a "$fwork")"
  rm -rf "$fwork"
else
  echo "  --   no files binary at $manager, so its half was not driven"
fi

if [ "$fail" = 0 ]; then
  echo "an app tells the graph what it is doing and what it finished"
else
  echo "graph-input drive FAILED"
fi
exit "$fail"
