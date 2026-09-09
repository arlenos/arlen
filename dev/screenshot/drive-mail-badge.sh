#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Watch an app put a badge on the bus, and read the number off the wire.
#
# WHY THIS EXISTS. Twenty-one shell surfaces had both consumer ends built and no
# producer at all; the top bar has drawn a badge slot beside the app name since
# the day it was written and nothing has ever filled it. Mail is the first
# producer, because it already counts its own unread mail for the folder rail.
#
# WHAT IT CAN HONESTLY CHECK, and where the line is. The RENDER needs a focused
# window, which needs a compositor, which this host does not have - the shell
# keys the badge it draws on the focused app's id. What is real without one is
# the app's whole half of the contract: that a window reading a maildir publishes
# `app.badge.set` at all, that it carries THIS app's id, and that the count on
# the wire is the number of unread messages actually in the inbox. A drive that
# only asserted "something was published" would pass on a badge that said 7.
#
# The consumer half is covered where it lives: `appStateStores.test.ts` for the
# store, `promotion.rs` for the graph (which deliberately does NOT record a
# count-only badge), and the overlay markup in `GlobalMenuBar.svelte`.
#
# A DEBUG BINARY AND A PREVIEW, for the reason `drive-mail-mailbox.sh` gives: the
# maildir override is `#[cfg(debug_assertions)]` on purpose. The capability that
# admits `badges_set` is compiled INTO the binary from `capabilities/default.json`,
# so a stale build refuses the call and this suite would report a silent app.
#
# Run: dev/screenshot/drive-mail-badge.sh [path-to-debug-arlen-mail-app]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
export SHOOT_FRONTEND_SERVED=1
# shellcheck source=dev/screenshot/lib/wait.sh
. "$here/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$here/lib/preview.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$here/lib/fresh.sh"

app="${1:-$root/target/debug/arlen-mail-app}"
bus="$root/target/debug/event-bus"
emit="$root/target/debug/arlen-event-emit"
# Short prefix: a Unix socket path has to fit in `sun_path`, and a runtime dir
# under a long checkout overflows it before the bus can bind.
work=/tmp/arlen-drive-badge
fail=0
PREVIEW_PGID=""
bus_pid=""

for b in "$app" "$bus" "$emit"; do
  [ -x "$b" ] || { echo "missing $b - build it first" >&2; exit 2; }
done
require_fresh "$app" "$root/apps/mail/src-tauri/src" "$root/apps/mail/src-tauri/capabilities" || exit 2
require_fresh_frontend "$root/apps/mail/build" "$root/apps/mail/src" || exit 2
require_fresh "$bus" "$root/daemons/event-bus/src" || exit 2
require_fresh "$emit" "$root/dev/event-emit/src" || exit 2

cleanup() {
  [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null
  stop_preview
  return 0
}
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

rm -rf "$work"
mkdir -p "$work/run/arlen" "$work/mail/cur" "$work/mail/new" "$here/out"

# TWO UNREAD AND ONE READ, and the read one is what makes the number mean
# something: a badge that counted every message would say three here, and a check
# against a mailbox of only unread mail could not tell the two apart. A maildir
# says read in the filename - out of `new/`, carrying `S`.
printf 'From: rosa@example.org\nSubject: the roof survey\nDate: Tue, 2 Jan 2024 10:00:00 +0000\n\nBody.\n' \
  > "$work/mail/new/1.host"
printf 'From: gutters@example.org\nSubject: the gutter quote\nDate: Wed, 3 Jan 2024 09:00:00 +0000\n\nBody.\n' \
  > "$work/mail/new/2.host"
printf 'From: fences@example.org\nSubject: the fence estimate\nDate: Thu, 4 Jan 2024 11:00:00 +0000\n\nBody.\n' \
  > "$work/mail/cur/3.host:2,S"

export ARLEN_RUNTIME_DIR="$work/run" XDG_RUNTIME_DIR="$work/run"
producer="$work/run/arlen/event-bus-producer.sock"

echo "mail badge:"

"$bus" > "$work/bus.log" 2>&1 &
bus_pid=$!
for _ in $(seq 1 20); do [ -S "$producer" ] && break; sleep 0.5; done
[ -S "$producer" ] || { echo "the bus never bound its producer socket:" >&2; tail -3 "$work/bus.log" >&2; exit 2; }

# Subscribe BEFORE the app starts. The bus fans out to whoever is registered when
# an event arrives, so a watcher that registers afterwards has already missed the
# badge - the app publishes its first one as soon as the mailbox lands.
ARLEN_SESSION_ID=drive "$emit" --watch "app.badge." 40 > "$work/watch.log" 2>&1 &
watcher=$!
sleep 1

# 1454 because that is the `devUrl` in this app's `tauri.conf.json`, and a debug
# binary loads exactly that. Serving the build on any other port is a page the
# app will never ask for, which reads as a frontend that would not start.
start_preview "$root/apps/mail" 1454 || exit 1
wait_for_http "http://localhost:1454/" || exit 1

cat > "$work/probe.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
for (let i = 0; i < 60; i++) {
  if (document.body.innerText.includes("roof survey")) break;
  await wait(250);
}
// Hold still while the watcher outside collects. The probe cannot tell the shell
// script when the app has published; the app publishes on its own schedule.
await wait(12000);
const text = document.body.innerText.replace(/\s+/g, " ").trim();
return JSON.stringify({ listed: /roof survey/.test(text), text: text.slice(0, 200) });
JS

# ARLEN_SESSION_ID IS NOT OPTIONAL FOR A PRODUCER, and this drive found that out
# by publishing nothing on its first green run. An event belongs to a session;
# `UnixEventEmitter::new` refuses to invent an id and says so through `tracing`,
# which in a webview host goes nowhere. `arlen-session` mints it once per login
# and `arlen-run` forwards it into every confined app, so a real mail window has
# one - the app is given one here for the same reason it is given a runtime dir.
got=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail;XDG_RUNTIME_DIR=$work/run;ARLEN_RUNTIME_DIR=$work/run;ARLEN_SESSION_ID=drive" \
  SHOOT_INJECT="$work/probe.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-badge.png" "" 20 2>&1 \
  | sed -n 's/^inject result: //p')

wait "$watcher"
watched=$(cat "$work/watch.log")

say "the mailbox this run is about is on screen" \
  "$(printf '%s' "$got" | grep -q '"listed":true' && echo 1 || echo 0)" "$got"
say "and it left a picture of mail-badge.png" \
  "$([ -s "$here/out/mail-badge.png" ] && echo 1 || echo 0)" "$got"
say "the app put a badge on the bus" \
  "$(printf '%s' "$watched" | grep -q "^saw app.badge.set" && echo 1 || echo 0)" "$watched"
say "under its own app id" \
  "$(printf '%s' "$watched" | grep -q "app_id=dev.arlen.mail" && echo 1 || echo 0)" "$watched"
# THE NUMBER, which is the whole point: two unread of three messages.
say "carrying the count of unread mail, not of mail" \
  "$(printf '%s' "$watched" | grep -q "count=2" && echo 1 || echo 0)" "$watched"

if [ "$fail" = 0 ]; then
  echo "an app publishes the badge the top bar has been drawing an empty slot for"
else
  echo "badge drive FAILED"
fi
exit "$fail"
