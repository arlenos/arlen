#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive a multi-file delete that FAILS, and prove the progress row it registered
# ends rather than sitting there claiming the work is still running.
#
# WHY THIS IS A SEPARATE DRIVE. `drive-files.sh` presses the app the way hands
# do and checks the disk after each step, which is the right suite and covers
# none of this: every operation it performs is single-file, `worth_reporting` is
# `count > 1`, so no job is ever registered in it. And the row a job produces
# renders in the SHELL's notification popover, which that drive cannot see from
# where it stands.
#
# WHAT IT CAUGHT. On 5 September `files_op` returned through seven `?` sites that
# all sat between `JobHandle::start` and `j.finish`, so a delete that failed on
# the third of five files left "Deleting 5 items" registered with the shell and
# never completed it - the popover outlives the window, so it stayed there
# describing work that had stopped minutes before. Worse, `finish` was only ever
# reachable with "done": the protocol has a failure state the app could not send.
# I found that by READING, and this is the run that would have found it.
#
# THE CONSUMER IS REAL. `arlen-notifyd` already owns `org.freedesktop.Notifications`
# and serves the JobViewServer at `/org/arlen/JobViewServer`, so this stands the
# actual daemon on a private bus rather than a stub - a stub would test my reading
# of the interface instead of the interface.
#
# HOW THE FAILURE IS CAUSED, deterministically and without permission games:
# `trash_dir()` resolves through `dirs::data_local_dir()`, so pointing
# XDG_DATA_HOME at a regular FILE makes its `create_dir_all` fail with ENOTDIR.
# That is the first `?` after the job is registered, which is the exact site.
#
# Run: dev/screenshot/drive-files-jobs.sh [path-to-arlen-files]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/bus.sh
. "$here/lib/bus.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$here/lib/fresh.sh"
app="${1:-$root/target/release/arlen-files}"
notifyd="$root/target/debug/arlen-notifyd"
# Directly under $HOME and without a leading dot, for the same reason the sibling
# drive says: the app opens at Home and hides dotfiles.
work="$HOME/arlen-drive-files-jobs"
bus_dir="$(mktemp -d)"
fail=0
BUS_PID=""
BUS_ADDR=""
nd_pid=""
mon_pid=""

cleanup() {
  [ -n "$mon_pid" ] && kill "$mon_pid" 2>/dev/null
  [ -n "$nd_pid" ] && kill "$nd_pid" 2>/dev/null
  [ -n "$BUS_PID" ] && kill "$BUS_PID" 2>/dev/null
  rm -rf "$bus_dir"
  return 0
}
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

[ -x "$app" ] || { echo "no files binary at $app"; exit 2; }
require_fresh "$notifyd" "$root/daemons/notification-daemon/src" || exit 2
[ -x "$notifyd" ] || {
  echo "!! no arlen-notifyd at $notifyd - nothing would answer Register, so the" >&2
  echo "   app would register no job and this suite would pass without testing" >&2
  echo "   anything. cargo build --manifest-path daemons/notification-daemon/Cargo.toml" >&2
  exit 2
}

rm -rf "$work"; mkdir -p "$work" "$here/out"
printf 'one\n' > "$work/alpha.txt"
printf 'two\n' > "$work/beta.txt"
# A VALID data dir whose `Trash` is a regular FILE. Pointing XDG_DATA_HOME
# itself at a file was the first attempt and it stops the app starting at all -
# WebKit wants a data directory - so the run timed out talking to the driver and
# read as the probe being wrong. This breaks only `create_dir_all(Trash/files)`,
# which is `trash_dir()`'s ENOTDIR and the first `?` after the job registers.
mkdir -p "$bus_dir/data"
printf 'not a directory\n' > "$bus_dir/data/Trash"

echo "file manager jobs:"

# Not `addr="$(start_private_bus ...)"`: that subshell would swallow BUS_PID and
# leak the daemon (see lib/bus.sh).
start_private_bus "$bus_dir" || exit 1
addr="$BUS_ADDR"
XDG_DATA_HOME="$bus_dir/data" XDG_CONFIG_HOME="$bus_dir/cfg" XDG_RUNTIME_DIR="$bus_dir" \
  DBUS_SESSION_BUS_ADDRESS="$addr" "$notifyd" >"$bus_dir/notifyd.log" 2>&1 &
nd_pid=$!
wait_for_bus_name "org.arlen.JobViewServer1" "$addr" || {
  echo "       notifyd log:" >&2; tail -5 "$bus_dir/notifyd.log" >&2; exit 1
}

# The wire itself. The daemon answers so the producer's calls succeed; the
# monitor is how this script SEES them, since the rendered row lives in a shell
# this drive does not run.
DBUS_SESSION_BUS_ADDRESS="$addr" dbus-monitor --session \
  "interface='org.arlen.JobViewServer1'" >"$bus_dir/wire.log" 2>&1 &
mon_pid=$!
sleep 1

cat > "$work/.jobs.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// The sibling drive's idiom, not a `.row` class of my own invention - there is
// no such class, and guessing one is how the first run of this returned nothing
// at all and read as the app failing to start.
const cellFor = (name) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === name)[0];
const rowOf = (cell) => cell.closest("[role=row], li, tr, div") || cell;
// Wait for the listing rather than for a number: this runs on whatever machine
// and load, and asserting into an unpainted page reports the app for the
// machine being busy.
const until = async (f) => { for (let i = 0; i < 60; i++) { if (f()) return true; await wait(250); } return false; };

if (!(await until(() => cellFor("arlen-drive-files-jobs")))) {
  return JSON.stringify({ opened: false, step: "home listing" });
}
rowOf(cellFor("arlen-drive-files-jobs"))
  .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
if (!(await until(() => cellFor("alpha.txt")))) {
  return JSON.stringify({ opened: false, step: "folder listing" });
}
const fm = document.querySelector(".fm") || document.body;
// Ctrl+A then Delete: two entries is what makes this a job at all
// (`worth_reporting` is `count > 1`), and one keystroke is steadier to drive
// than two modified clicks.
// On `.fm`, not on window. The sibling drive's header says where each key is
// handled - Delete on the `.fm` container, Ctrl+Z at the layout - and Ctrl+A
// shares Delete's handler. Dispatched at window it is simply not heard, and the
// first run of this deleted nothing while reporting the page opened fine.
fm.dispatchEvent(new KeyboardEvent("keydown",
  { key: "a", ctrlKey: true, bubbles: true, cancelable: true }));
await wait(500);
fm.dispatchEvent(new KeyboardEvent("keydown",
  { key: "Delete", bubbles: true, cancelable: true }));
await wait(4000);
return JSON.stringify({
  opened: true,
  stillListed: [...document.querySelectorAll(".fm-browse *")]
    .filter((e) => e.children.length === 0 && /\.txt$/.test((e.textContent || "").trim()))
    .map((e) => e.textContent.trim()).sort(),
});
JS

got=$(SHOOT_APP_ENV="DBUS_SESSION_BUS_ADDRESS=$addr;XDG_DATA_HOME=$bus_dir/data" \
  SHOOT_INJECT="$work/.jobs.js" \
  "$here/shoot-app.sh" "$app" "$here/out/files-jobs.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

sleep 1
wire="$(cat "$bus_dir/wire.log" 2>/dev/null)"

say "the delete is driven far enough to register a job" \
  "$(printf '%s' "$got" | grep -q '"opened":true' && echo 1 || echo 0)" "$got"

# THE case. A registered job that never finishes is a row that outlives the
# operation, and before 5 September that is exactly what a failed delete left.
say "a multi-file delete registers a job with the real server" \
  "$(printf '%s' "$wire" | grep -q "member=Register" && echo 1 || echo 0)" \
  "no Register on the wire; the app never told anybody it had started"

say "and the failure ends it rather than leaving the row running" \
  "$(printf '%s' "$wire" | grep -q "member=Finish" && echo 1 || echo 0)" \
  "Register but no Finish: the row would sit in the popover forever"

# And in the vocabulary the zone can render. `error-fatal` is KDE JobViewV3's
# token, which the shell translates in one place; a word of our own arrives in a
# state the zone has no branch for and renders as nothing.
say "and says it failed, in the wire's own word" \
  "$(printf '%s' "$wire" | grep -q "error-fatal" && echo 1 || echo 0)" \
  "no error-fatal on the wire: $(printf '%s' "$wire" | grep -oE 'member=[A-Za-z]+' | tr '\n' ' ')"

[ "$fail" = 0 ] && echo "a failed file operation ends its progress row, in a word the zone can render"
exit "$fail"
