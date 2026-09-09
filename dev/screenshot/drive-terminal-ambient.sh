#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run a slow command and watch the desktop's ambient pulse go up and come down.
#
# WHY THIS EXISTS. `shell.ambient` had every layer built and no producer at all -
# the SDK surface, the plugin command, the shell's overlay with its clamp and its
# global switch - and no app was even GRANTED it, so nothing in the tree had ever
# reached it. The terminal is the first producer, and the thing worth proving is
# not that a call happens but that the WAIT works: an effect that fired on every
# command would tint the whole screen for `ls`, which is the failure this surface
# is one step away from at all times.
#
# SO IT DRIVES BOTH SIDES OF THE THRESHOLD. A fast command must publish nothing,
# and a slow one must publish a pulse and then take it back. A drive that only
# asserted the slow case would pass on a terminal that tints for everything.
#
# WHAT IT CANNOT CHECK. The RENDER needs a compositor and a shell process, which
# this host does not have; the overlay's own half is covered where it lives
# (`appStateStores.test.ts` for the pick, the component for the clamp and the
# global switch). What is real here is the app's whole side of the contract,
# driven through a real shell: the marks fire, the wait is honoured, and what
# lands on the wire is a slow accent pulse under this app's id.
#
# IT TAKES ABOUT FOUR MINUTES. Most of that is the page's own timers under the
# headless driver rather than anything this script waits for; see the watch
# window's comment below.
#
# Run: dev/screenshot/drive-terminal-ambient.sh [path-to-arlen-terminal]
#
# Build with `tauri build --no-bundle`; the capability that admits `ambient_set`
# is compiled INTO the binary from `capabilities/default.json`, so a stale build
# refuses the call and this suite would report a silent app.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$here/lib/fresh.sh"

app="${1:-$root/target/release/arlen-terminal}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
# Short prefix: a Unix socket path has to fit in `sun_path`.
work=/tmp/arlen-drive-ambient
fail=0
bus_pid=""
watcher=""

for b in "$app" "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$app" "$root/apps/terminal/src-tauri/src" "$root/apps/terminal/src-tauri/capabilities" || exit 2
require_fresh_frontend "$root/apps/terminal/build" "$root/apps/terminal/src" || exit 2
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

cleanup() {
  [ -n "$watcher" ] && kill "$watcher" 2>/dev/null
  [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null
  return 0
}
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

rm -rf "$work"
mkdir -p "$work/run/arlen" "$here/out"
export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

echo "terminal ambient:"

"$bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$producer" ] && break; sleep 0.5; done
[ -S "$producer" ] || { echo "the bus never bound its producer socket:" >&2; tail -3 "$work/bus.log" >&2; exit 2; }

# Subscribe BEFORE the app starts: the bus fans out to whoever is registered when
# an event arrives, so a watcher that registers late has already missed it.
# A GENEROUS CEILING AND A COUNT, rather than a window sized by guesswork. Under
# Xvfb plus WebKitWebDriver this probe's twenty seconds of `setTimeout` waiting
# takes about two MINUTES of wall clock - the page's timers are dilated by
# roughly six - so the window has to cover the slow case. Three windows sized by
# hand (45s, 120s, 300s) each expired with an effect in flight, and the bus log
# showed the same shape every time: the event received, dispatched, and the
# consumer gone in the same millisecond. A watcher that stops before the app has
# done the thing reports a silent app, which is the most expensive kind of green.
#
# So: ten minutes as a ceiling, and stop at the two events this drive is about.
# The run costs what it actually took rather than what the ceiling allows.
ARLEN_SESSION_ID=drive "$emit" --watch "app.ambient." 600 2 > "$work/watch.log" 2>&1 &
watcher=$!
sleep 1

# BOTH SIDES OF THE THRESHOLD IN ONE RUN, fast first. If the fast command
# published, the wire carries a set before the slow command has even started, and
# the ordering assertion below catches it.
cat > "$work/probe.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const ta = document.querySelector(".xterm-helper-textarea");
if (!ta) return JSON.stringify({ error: "no terminal to type into" });
ta.focus();
// THROUGH THE INPUT EVENT, not keydown: xterm.js takes printable characters from
// the helper textarea's `input` event, and a synthetic keydown inserts nothing.
// `drive-terminal.sh` learned this the hard way and its comment says so.
const type = (s) => {
  ta.value = s;
  ta.dispatchEvent(new InputEvent("input", { data: s, inputType: "insertText", bubbles: true }));
};
const enter = () =>
  ta.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }),
  );
const rows = () => document.querySelector(".xterm-rows")?.innerText ?? "";

// Let the prompt settle, or the first keystrokes land before the shell is ready.
for (let i = 0; i < 40; i++) {
  if (/~/.test(rows())) break;
  await wait(250);
}
type("echo fastmarker");
enter();
for (let i = 0; i < 40; i++) {
  if ((rows().match(/fastmarker/g) ?? []).length >= 2) break;
  await wait(100);
}
// Well past the wait, so a terminal that tinted for the fast one has already
// said so on the wire before the slow one starts.
await wait(6500);

type("sleep 8");
enter();
// Past the threshold and past the command, so the take-down is on the wire too.
await wait(12000);
const text = rows().replace(/\s+/g, " ").trim();
return JSON.stringify({ fast: (text.match(/fastmarker/g) ?? []).length, tail: text.slice(-120) });
JS

got=$(SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive" \
  SHOOT_INJECT="$work/probe.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/terminal-ambient.png" "" 280 2>&1 \
  | sed -n 's/^inject result: //p')

wait "$watcher"
watched=$(cat "$work/watch.log")

say "a command ran at all, so the marks had something to fire on" \
  "$(printf '%s' "$got" | grep -qE '"fast":[2-9]' && echo 1 || echo 0)" "$got"
say "and it left a picture of terminal-ambient.png" \
  "$([ -s "$here/out/terminal-ambient.png" ] && echo 1 || echo 0)" "$got"

# THE WAIT, and it is the assertion that matters most: exactly one set, for the
# slow command. Two would mean the fast one tinted the screen as well.
sets=$(printf '%s\n' "$watched" | grep -c "^saw app.ambient.set")
say "a quick command tints nothing, a slow one tints once" \
  "$([ "$sets" = 1 ] && echo 1 || echo 0)" "$watched"
say "under this app's own id" \
  "$(printf '%s' "$watched" | grep -q "app_id=dev.arlen.terminal" && echo 1 || echo 0)" "$watched"
# A slow ACCENT pulse, not a screen-filling anything: the enum values are
# `AmbientEffect.PULSE` = 1 and `AmbientColor.ACCENT` = 1 in the proto, and the
# intensity is well under the 0.5 cap.
say "as a slow accent pulse, well under the cap" \
  "$(printf '%s' "$watched" | grep -qE "effect=1 color=1 intensity=0\.1[0-9]* speed=1" && echo 1 || echo 0)" "$watched"
# And it comes DOWN. An effect that goes up and stays is the worse half of this
# surface: a wash nobody asked for that outlives what it was about.
say "and it comes down when the command finishes" \
  "$(printf '%s' "$watched" | grep -q "^saw app.ambient.cleared" && echo 1 || echo 0)" "$watched"

if [ "$fail" = 0 ]; then
  echo "the desktop pulses while a command runs, and only while it runs"
else
  echo "terminal ambient drive FAILED"
fi
exit "$fail"
