#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Pick a menu item and see the app do the thing.
#
# WHY THIS EXISTS. Eleven apps drew a menu whose clicks went nowhere: an app
# publishes its menu through the shell plugin, the shell draws it, somebody picks
# an item, and the shell puts the action back on the bus as
# `app.menu.action_invoked` - which nothing inside the app was listening for.
# The relay is built and unit-tested now, and a unit test proves the decoder, not
# the wire. A gate that says the topic is subscribed is a different fact from a
# menu that works.
#
# WHAT IT ACTUALLY DRIVES. A real event bus, a real release build of the text
# editor with the plugin inside it, and a real event on the wire - everything the
# live path has except the shell's own click, which needs a compositor. The
# editor's File > Save is the item, because its effect is on the DISK: this reads
# the file back rather than trusting a status line, the same reason
# `drive-text-editor.sh` does.
#
# AND THE FILTER, which is half the feature. Every app on the bus sees every other
# app's menu clicks, so the second case sends the same action addressed to a
# different app and asserts this one does nothing at all. Without that, a relay
# that ignored `app_id` and saved on anybody's click would pass the first case.
# The two only mean something together: on its own the second case also passes
# when the relay is dead, since a dead relay saves nothing for anybody either.
#
# Run: dev/screenshot/drive-menu-relay.sh [path-to-arlen-text-editor]
#
# Build the editor with `tauri build --no-bundle`; a plain `cargo build --release`
# leaves the binary pointing at devUrl and every probe reads a refused connection.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"

app="${1:-$root/target/release/arlen-text-editor}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
# Short prefix on purpose: a Unix socket path has to fit in `sun_path`, about 108
# bytes, and a runtime dir under a long checkout overflows it and the bus refuses
# to bind before it starts.
work=/tmp/arlen-drive-menu
fail=0

[ -x "$app" ] || { echo "no text-editor binary at $app - build it with tauri build --no-bundle"; exit 2; }
for b in "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

rm -rf "$work"
mkdir -p "$work/run/arlen" "$here/out"
printf 'fn main() {\n    println!("before");\n}\n' > "$work/sample.rs"

export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

cleanup() { [ -n "${bus_pid:-}" ] && kill "$bus_pid" 2>/dev/null; return 0; }
trap cleanup EXIT

"$bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$producer" ] && break; sleep 0.5; done
[ -S "$producer" ] || { echo "the bus never bound its producer socket:" >&2; tail -3 "$work/bus.log" >&2; exit 2; }

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

# Type into the buffer, then hold still while the script outside picks the menu
# item over and over. The probe runs inside the webview and cannot tell the shell
# script when it has finished typing, so the click is repeated rather than timed:
# the first attempt of the first version landed on a buffer that was still clean,
# saved nothing, and the run then reported the relay broken. A save is idempotent,
# so repeating it costs nothing and removes the guess.
cat > "$work/p-menu.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
if (!cm) return "no buffer";
cm.focus();
document.execCommand("insertText", false, "// via the menu\n");
await new Promise(r => setTimeout(r, 40000));
const state = document.querySelector(".savestate");
return JSON.stringify({ state: state && state.textContent.trim() });
JS

# Pick the item every three seconds until the buffer reaches the file, or until
# the probe stops waiting. `watch` empty means keep going to the end, which is
# what the other-app case wants: it has to click as often as the first one did
# before it can claim the app ignored every one of them.
click_loop() {  # click_loop <app-id> <watch|""> <log>
  local app_id="$1" watch="$2" log="$3"
  for _ in $(seq 1 12); do
    sleep 3
    ARLEN_SESSION_ID=drive "$emit" --menu "$app_id" file.save >> "$log" 2>&1
    if [ -n "$watch" ] && head -1 "$work/sample.rs" | grep -q "// via the menu"; then break; fi
  done
}

drive() {  # drive <out-png>
  SHOOT_APP_ARGS="$work/sample.rs" SHOOT_INJECT="$work/p-menu.js" \
  SHOOT_APP_ENV="ARLEN_PRODUCER_SOCKET=$producer;ARLEN_CONSUMER_SOCKET=$work/run/arlen/event-bus-consumer.sock" \
    "$here/shoot-app.sh" "$app" "$here/out/$1" 2>&1 | sed -n 's/^inject result: //p'
}

echo "menu relay:"

: > "$work/emit-mine.log"
click_loop dev.arlen.text-editor watch "$work/emit-mine.log" &
clicker=$!
got=$(drive menu-relay-save.png)
# By pid, not a bare `wait`: the bus is a child of this script too and never
# exits, so waiting on everything waits forever.
wait "$clicker"
say "the bus delivered the menu click" \
  "$(grep -q "^delivered app.menu.action_invoked" "$work/emit-mine.log" && echo 1 || echo 0)" \
  "$(cat "$work/emit-mine.log")"
say "picking File > Save writes the buffer to disk" \
  "$(head -1 "$work/sample.rs" | grep -q "// via the menu" && echo 1 || echo 0)" \
  "$got (first line: $(head -1 "$work/sample.rs"))"

# The same action addressed to another app. Every subscriber sees it; only the
# one it names may act on it.
printf 'fn main() {\n    println!("before");\n}\n' > "$work/sample.rs"
: > "$work/emit-other.log"
click_loop dev.arlen.files "" "$work/emit-other.log" &
clicker=$!
got=$(drive menu-relay-other-app.png)
wait "$clicker"
say "and the same click addressed to another app is delivered" \
  "$(grep -q "^delivered app.menu.action_invoked" "$work/emit-other.log" && echo 1 || echo 0)" \
  "$(cat "$work/emit-other.log")"
# Exactly the original content: a save that went through would leave the typed
# line here, and a `grep` for its absence would pass on an empty file too.
say "but this app does not act on it" \
  "$([ "$(cat "$work/sample.rs")" = "$(printf 'fn main() {\n    println!("before");\n}')" ] && echo 1 || echo 0)" \
  "$(head -2 "$work/sample.rs" | tr '\n' ' ')"

[ "$fail" = 0 ] && echo "a menu click crosses the bus into the app and only into the app it names"
exit "$fail"
