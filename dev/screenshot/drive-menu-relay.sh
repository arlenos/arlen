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
# WHAT IT ACTUALLY DRIVES. A real event bus, a real release build of the app with
# the plugin inside it, and a real event on the wire - everything the live path
# has except the shell's own click, which needs a compositor.
#
# TWO APPS, and the second is the point. The text editor's File > Save is the
# first because its effect is on the DISK: this reads the file back rather than
# trusting a status line, the same reason `drive-text-editor.sh` does. The file
# manager is the second because it is the one that had a bespoke consumer of its
# own, deleted when the relay landed, so it is the app with something to lose;
# its File > New folder also lands on the disk, as a directory that either is
# there or is not.
#
# AND THE FILTER, which is half the feature. Every app on the bus sees every other
# app's menu clicks, so the second case sends the same action addressed to a
# different app and asserts this one does nothing at all. Without that, a relay
# that ignored `app_id` and saved on anybody's click would pass the first case.
# The two only mean something together: on its own the second case also passes
# when the relay is dead, since a dead relay saves nothing for anybody either.
#
# Run: dev/screenshot/drive-menu-relay.sh [path-to-arlen-text-editor] [path-to-arlen-files]
#
# Build both with `tauri build --no-bundle`; a plain `cargo build --release`
# leaves the binary pointing at devUrl and every probe reads a refused connection.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"

app="${1:-$root/target/release/arlen-text-editor}"
files_app="${2:-$root/target/release/arlen-files}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
# Short prefix on purpose: a Unix socket path has to fit in `sun_path`, about 108
# bytes, and a runtime dir under a long checkout overflows it and the bus refuses
# to bind before it starts.
work=/tmp/arlen-drive-menu
fail=0

for b in "$app" "$files_app"; do
  [ -x "$b" ] || { echo "no binary at $b - build it with tauri build --no-bundle"; exit 2; }
done
for b in "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

# Directly under $HOME and without a leading dot: the file manager opens at Home
# and hides dotfiles, so a fixture it cannot reach by clicking is a fixture it
# cannot open. The editor's own scratch stays under /tmp with the sockets.
fwork="$HOME/arlen-drive-menu-files"

rm -rf "$work" "$fwork"
mkdir -p "$work/run/arlen" "$fwork" "$here/out"
printf 'fn main() {\n    println!("before");\n}\n' > "$work/sample.rs"
printf 'anything\n' > "$fwork/a-file.txt"

export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

# THE APP IS GIVEN NO SOCKET PIN, ON PURPOSE. It gets `XDG_RUNTIME_DIR` and
# nothing else, which is what a booted session gives it: `arlen-session` pins none
# of the `ARLEN_*_SOCKET` variables, and a test in `daemons/session` asserts it
# does not, because a pin there would override the per-user resolution for every
# process in the session. So the app has to find the bus the way it will have to
# find it in real life, through `$XDG_RUNTIME_DIR/arlen/`.
#
# This drive DID pin both variables, and that hid a second defect for a while: the
# plugin resolved env-or-`/run/arlen` with no tier in between, so on an image every
# app was subscribed to the system bus while the shell published clicks to the
# session one. Menus registered and their clicks never came back. Pinning here
# would have gone on hiding it.
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
click_loop() {  # click_loop <app-id> <action> <landed-predicate|""> <log>
  local app_id="$1" action="$2" landed="$3" log="$4"
  for _ in $(seq 1 12); do
    sleep 3
    ARLEN_SESSION_ID=drive "$emit" --menu "$app_id" "$action" >> "$log" 2>&1
    if [ -n "$landed" ] && eval "$landed"; then break; fi
  done
}

saved() { head -1 "$work/sample.rs" | grep -q "// via the menu"; }
# A directory that is not a dotfile. Pointing HOME at the fixture makes the app
# create `.cache`, `.config` and `.local` there itself, so a bare "any directory"
# test passes without a single menu click ever landing - it was written that way
# first and would have called a dead relay a working one. Not the localised name
# either: `New folder` in English is `Neuer Ordner` in German, and the check
# should not need to know which language the run is in.
foldered() { [ -n "$(find "$fwork" -mindepth 1 -maxdepth 1 -type d -not -name '.*' 2>/dev/null)" ]; }

# The whole of `shoot-app.sh`'s output is kept, not only the `inject result:`
# line. That harness says plainly when it could not reach a frontend or could not
# take a picture, and a drive that greps for one line throws the sentence away -
# which is how three working features once got reported as broken. The log is
# what `shot_written` points at when a picture is missing.
drive() {  # drive <binary> <app-args> <probe-js> <out-png> [extra-env]
  rm -f "$here/out/$4"
  SHOOT_APP_ARGS="$2" SHOOT_INJECT="$3" \
  SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/run${5:+;$5}" \
    "$here/shoot-app.sh" "$1" "$here/out/$4" > "$work/shoot-$4.log" 2>&1
  sed -n 's/^inject result: //p' "$work/shoot-$4.log"
}

# A drive whose picture is missing has not been looked at, whatever its checks
# say, so the missing picture is itself a failure rather than a footnote.
shot_written() {  # shot_written <out-png>
  say "and it left a picture of $1" "$([ -s "$here/out/$1" ] && echo 1 || echo 0)" \
    "$(tail -3 "$work/shoot-$1.log" 2>/dev/null)"
}

echo "menu relay:"

: > "$work/emit-mine.log"
click_loop dev.arlen.text-editor file.save saved "$work/emit-mine.log" &
clicker=$!
got=$(drive "$app" "$work/sample.rs" "$work/p-menu.js" menu-relay-save.png)
# By pid, not a bare `wait`: the bus is a child of this script too and never
# exits, so waiting on everything waits forever.
wait "$clicker"
say "the bus delivered the menu click" \
  "$(grep -q "^delivered app.menu.action_invoked" "$work/emit-mine.log" && echo 1 || echo 0)" \
  "$(cat "$work/emit-mine.log")"
shot_written menu-relay-save.png
say "picking File > Save writes the buffer to disk" \
  "$(head -1 "$work/sample.rs" | grep -q "// via the menu" && echo 1 || echo 0)" \
  "$got (first line: $(head -1 "$work/sample.rs"))"

# The same action addressed to another app. Every subscriber sees it; only the
# one it names may act on it.
printf 'fn main() {\n    println!("before");\n}\n' > "$work/sample.rs"
: > "$work/emit-other.log"
click_loop dev.arlen.files file.save "" "$work/emit-other.log" &
clicker=$!
got=$(drive "$app" "$work/sample.rs" "$work/p-menu.js" menu-relay-other-app.png)
wait "$clicker"
say "and the same click addressed to another app is delivered" \
  "$(grep -q "^delivered app.menu.action_invoked" "$work/emit-other.log" && echo 1 || echo 0)" \
  "$(cat "$work/emit-other.log")"
# Exactly the original content: a save that went through would leave the typed
# line here, and a `grep` for its absence would pass on an empty file too.
say "but this app does not act on it" \
  "$([ "$(cat "$work/sample.rs")" = "$(printf 'fn main() {\n    println!("before");\n}')" ] && echo 1 || echo 0)" \
  "$(head -2 "$work/sample.rs" | tr '\n' ' ')"

# THE FILE MANAGER. It takes no path argument and opens at whatever the backend
# calls Home, so the fixture IS its home for this run: pointing `HOME` at it puts
# the window where the click has to land, instead of walking the probe through a
# double-click and hoping. That also keeps a mis-navigated `New folder` out of the
# real home directory, which is not this script's to write in.
#
# The filter is not re-run here: it is a property of the one relay in the plugin,
# and proving it twice would double the run for a second reading of the same code.
cat > "$work/p-files.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(3000);
await wait(40000);
return JSON.stringify({ listing: [...document.querySelectorAll(".fm-browse *")]
  .filter(e => e.children.length === 0 && (e.textContent||"").trim())
  .map(e => e.textContent.trim()).slice(0, 12) });
JS

: > "$work/emit-files.log"
click_loop dev.arlen.files file.new_folder foldered "$work/emit-files.log" &
clicker=$!
got=$(drive "$files_app" "" "$work/p-files.js" menu-relay-new-folder.png "HOME=$fwork")
wait "$clicker"
shot_written menu-relay-new-folder.png
say "picking File > New folder in the file manager makes the directory" \
  "$(foldered && echo 1 || echo 0)" \
  "$got (in $fwork: $(ls -A "$fwork" | tr '\n' ' '))"

[ "$fail" = 0 ] && echo "a menu click crosses the bus into two different apps, and only into the app it names"
exit "$fail"
