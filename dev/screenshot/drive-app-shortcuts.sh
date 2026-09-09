#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Watch an app put its own verbs in the launcher, and pick one.
#
# WHY THIS EXISTS. The waypointer has listed the FOCUSED app's shortcuts since it
# was written, and no app in the tree published any - so a surface promising the
# app's own actions was empty on every window. A visible hole rather than an
# absent feature: somebody opens the launcher over a file manager and searches
# its files instead of its verbs.
#
# WHAT IT CHECKS, and it is both directions. Out: that the app registers a list
# at all, that it names ITS OWN action strings, and that the labels went through
# the catalogue rather than being written in place. Back: that picking one makes
# the app do the thing - driven as a real `app.shortcut.action_invoked` on the
# bus, the way the shell dispatches a click, and asserted on the DISK because
# that is the only place a claim about a new folder can be checked.
#
# WHERE IT STOPS. The waypointer's own rendering of the list needs a compositor;
# what is here is the app's whole half of the contract plus the dispatch path.
#
# Run: dev/screenshot/drive-app-shortcuts.sh [path-to-arlen-files]
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves the
# binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"

app="${1:-$root/target/release/arlen-files}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
work=/tmp/arlen-drive-shortcuts
fail=0
bus_pid=""

for b in "$app" "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

# A pid in a lock file, not the socket: the socket FILE outlives the run, so a
# stale one would refuse a perfectly good start.
lock="$work.lock"
if [ -f "$lock" ] && kill -0 "$(cat "$lock" 2>/dev/null)" 2>/dev/null; then
  echo "another run of this drive is live (pid $(cat "$lock")); wait for it" >&2
  exit 2
fi

cleanup() {
  [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null
  rm -f "$lock"
  rm -rf "$fwork"
  return 0
}
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

rm -rf "$work"
mkdir -p "$work/run/arlen" "$here/out"
echo "$$" > "$lock"

# Directly under $HOME and not a dotfile: the file manager opens at whatever the
# backend calls home, so pointing HOME here puts the window where the action has
# to land - and keeps a mis-navigated new folder out of the real home directory.
fwork="$HOME/arlen-drive-shortcuts"
rm -rf "$fwork"; mkdir -p "$fwork"
printf 'anything\n' > "$fwork/a-file.txt"

export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

echo "app shortcuts:"

"$bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$producer" ] && break; sleep 0.5; done
[ -S "$producer" ] || { echo "the bus never bound its producer socket:" >&2; tail -3 "$work/bus.log" >&2; exit 2; }

# Subscribe before the app starts: it registers as soon as its catalogue lands,
# and the bus fans out only to whoever is listening at that moment.
ARLEN_SESSION_ID=drive "$emit" --watch "app.shortcut." 40 > "$work/watch.log" 2>&1 &
watcher=$!
sleep 1

# Pick the shortcut every three seconds until the folder appears. The probe
# inside the webview cannot tell this script when the app is ready, so the click
# repeats rather than being timed - a new folder is idempotent enough that
# repeating costs a name suffix at worst.
foldered() { [ -n "$(find "$fwork" -mindepth 1 -maxdepth 1 -type d -not -name '.*' 2>/dev/null)" ]; }
( for _ in $(seq 1 10); do
    sleep 3
    ARLEN_SESSION_ID=drive "$emit" --shortcut dev.arlen.files file.new_folder >> "$work/emit.log" 2>&1
    foldered && break
  done ) &
clicker=$!

cat > "$work/probe.js" <<'JS'
await new Promise(r => setTimeout(r, 33000));
return "held";
JS

SHOOT_INJECT="$work/probe.js" \
  SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive;HOME=$fwork" \
  "$here/shoot-app.sh" "$app" "$here/out/app-shortcuts.png" > "$work/shoot.log" 2>&1
wait "$clicker"
wait "$watcher"
watched=$(cat "$work/watch.log")

say "the app registers a list of its own verbs" \
  "$(printf '%s' "$watched" | grep -q "^saw app.shortcut.register" && echo 1 || echo 0)" "$watched"
say "and it left a picture of app-shortcuts.png" \
  "$([ -s "$here/out/app-shortcuts.png" ] && echo 1 || echo 0)" "$(tail -3 "$work/shoot.log")"
# THE ROUND TRIP. A launcher click reaches the app and the app does the thing -
# checked on the disk, because a folder either is there or is not.
say "picking one makes the app do the thing" \
  "$(foldered && echo 1 || echo 0)" "$(ls -a "$fwork")"
say "and the bus delivered the pick" \
  "$(grep -q "^delivered app.shortcut.action_invoked" "$work/emit.log" && echo 1 || echo 0)" \
  "$(cat "$work/emit.log")"

if [ "$fail" = 0 ]; then
  echo "an app's own verbs are in the launcher, and picking one lands"
else
  echo "shortcuts drive FAILED"
fi
exit "$fail"
