#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive Settings > Privacy > Physical in the REAL binary, against the REAL
# sentinel daemon, and prove the page says what was measured.
#
# WHY THIS PAGE GETS ITS OWN DRIVE. It is the one surface that tells somebody
# their machine is protected, and until 4 September every sentence on it came
# from a fixture: `sentinel_get_state` had no backend, so a live session read
# nothing and `fixPosture` reported a fix it had never performed. The risk is not
# a blank page, it is a confident one. So this runs the daemon, drives the app,
# and checks both directions - the readout renders real sentences, and with the
# daemon stopped the page says nothing is reporting rather than showing the
# fixture's reassuring lines.
#
# Run: dev/screenshot/drive-privacy-sentinel.sh [out-dir]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/wait.sh
. "$root/dev/screenshot/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$root/dev/screenshot/lib/preview.sh"
out="${1:-$root/dev/screenshot/out}"
app="$root/target/debug/arlen-settings"
sentineld="$root/target/debug/arlen-sentineld"
work="$(mktemp -d)"
daemon_pid=""
cleanup() {
  [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null || true
  stop_preview
  rm -rf "$work"
}
trap cleanup EXIT

[ -x "$app" ] || { echo "!! build it first: cargo build --manifest-path apps/settings/src-tauri/Cargo.toml" >&2; exit 1; }
[ -x "$sentineld" ] || { echo "!! build it first: cargo build --manifest-path daemons/sentineld/Cargo.toml" >&2; exit 1; }
# EXISTS IS NOT CURRENT. This page's whole claim is that it says only what was
# measured, and until 5 September it was driven against a daemon whose age
# nothing checked - a sentineld from last week would answer with last week's
# postures and every assertion below would be about those.
require_fresh "$sentineld" "$root/daemons/sentineld/src" "$root/daemons/sentinel-detect/src" || exit 1
[ -d "$root/apps/settings/build" ] || { echo "!! build the frontend first: (cd apps/settings && npm run build)" >&2; exit 1; }
if [ -n "$(find "$root/apps/settings/src" -newer "$root/apps/settings/build" -name '*.svelte' -o \
                -newer "$root/apps/settings/build" -name '*.ts' 2>/dev/null | head -1)" ]; then
  echo "!! the built frontend is OLDER than apps/settings/src - rebuild before" >&2
  echo "   believing a failure: (cd apps/settings && npm run build)" >&2
fi

# Through the shared helper, which refuses a port it did not start and kills the
# server's whole process group. This was `( ... ) &` with `preview_pid=$!`, and
# that pid is the SUBSHELL's: `npx` spawns the node that holds the port, so the
# kill missed it and every run left a server behind. The next run's readiness
# check then passed against THAT one and read a frontend built at some earlier
# time - a suite passing while testing a page nobody had just built.
start_preview "$root/apps/settings" 1421 || exit 1
wait_for_http "http://localhost:1421/" || exit 1

cat > "$work/goto.js" <<'JS'
location.assign("/privacy/physical");
return JSON.stringify({ went: true });
JS

# The page under a running daemon. Every check is about a claim: that the lines
# are sentences rather than catalogue keys, that nothing asserts the microphone
# is idle when nothing measured it, and that a partial reading says so.
cat > "$work/probe-live.js" <<'JS'
const text = document.body.innerText;
return JSON.stringify({
  reporting: !text.includes("Nothing is reporting this machine"),
  noRawKeys: !text.includes("s.sent.post."),
  someReadout: /Wi-Fi|Bluetooth/.test(text),
  captureHonest:
    text.includes("Nothing here can tell whether the microphone or camera is in use.") &&
    !text.includes("Nothing is using the microphone or camera right now."),
  noFixtureClaim: !text.includes("Saved networks are not broadcast while disconnected."),
});
JS

# The same page with nothing behind it. An unreachable sentinel and a machine
# with nothing wrong must not look alike.
cat > "$work/probe-dead.js" <<'JS'
const text = document.body.innerText;
return JSON.stringify({
  saysSo: text.includes("Nothing is reporting this machine"),
  noReadout: !/Wi-Fi uses|Bluetooth is discoverable/.test(text),
});
JS

runtime="$work/run"
mkdir -p "$runtime"
XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$work/cfg" "$sentineld" >"$work/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_socket "$runtime/arlen/sentinel.sock" || { cat "$work/daemon.log" >&2; exit 1; }

echo ">> the privacy page with the sentinel running"
SHOOT_APP_ENV="XDG_RUNTIME_DIR=$runtime" \
SHOOT_INJECT="$work/goto.js:$work/probe-live.js" SHOOT_INJECT_SETTLE=4 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/privacy-sentinel-live.png" "" 8 \
  | tee "$work/run1.log" | grep -E "inject result" || true

grep -q '"reporting":true' "$work/run1.log" || { echo "!! the page could not read the running daemon" >&2; exit 1; }
grep -q '"noRawKeys":true' "$work/run1.log" || { echo "!! a catalogue key rendered where a sentence belongs" >&2; exit 1; }
grep -q '"someReadout":true' "$work/run1.log" || { echo "!! the readout is empty with the detector on" >&2; exit 1; }
grep -q '"captureHonest":true' "$work/run1.log" || { echo "!! the page claims the microphone is idle on no evidence" >&2; exit 1; }
grep -q '"noFixtureClaim":true' "$work/run1.log" || { echo "!! a fixture sentence leaked into a live session" >&2; exit 1; }

kill "$daemon_pid" 2>/dev/null || true
daemon_pid=""
sleep 1

echo ">> the same page with nothing behind it"
SHOOT_APP_ENV="XDG_RUNTIME_DIR=$work/empty" \
SHOOT_INJECT="$work/goto.js:$work/probe-dead.js" SHOOT_INJECT_SETTLE=4 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/privacy-sentinel-absent.png" "" 8 \
  | tee "$work/run2.log" | grep -E "inject result" || true

grep -q '"saysSo":true' "$work/run2.log" || { echo "!! an unreachable sentinel did not say so" >&2; exit 1; }
grep -q '"noReadout":true' "$work/run2.log" || { echo "!! posture lines rendered with nothing measuring them" >&2; exit 1; }

echo ">> all green: the privacy page reports what was measured, and says when nothing was"
