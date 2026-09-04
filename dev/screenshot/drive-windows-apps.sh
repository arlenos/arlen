#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive Settings > Windows apps in the REAL binary and prove the page tells the
# truth on a machine with no bottle daemon.
#
# The page's fixture (three example apps, a health warning, a drive table) only
# exists under vite; the risk in the native app is the opposite one - that any of
# it leaks into a live session, where "Notepad++ is installed" would be a claim
# about a machine that has never seen Wine. So this drives the built app and
# asserts the honest empty state end to end: the unavailable banner, the
# "not known" list, no invented runtimes, and the detail route refusing an id
# rather than rendering switches for a bottle nobody can read.
#
# The window is a SPA served from build.devUrl (a debug binary does not use the
# bundled frontend), so the script serves the built frontend on that port first;
# navigation is a real location change, which the preview's SPA fallback serves.
#
# Run: dev/screenshot/drive-windows-apps.sh [out-dir]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# The frontend is served from a preview below, not baked into the binary, so the
# staleness guard compares Rust only (see shoot-app.sh).
export SHOOT_FRONTEND_SERVED=1
# shellcheck source=dev/screenshot/lib/wait.sh
. "$root/dev/screenshot/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$root/dev/screenshot/lib/preview.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"
out="${1:-$root/dev/screenshot/out}"
app="$root/target/debug/arlen-settings"
work="$(mktemp -d)"
daemon_pid=""
cleanup() {
  [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null || true
  stop_preview
  rm -rf "$work"
}
trap cleanup EXIT

[ -x "$app" ] || { echo "!! build it first: cargo build --manifest-path apps/settings/src-tauri/Cargo.toml" >&2; exit 1; }
[ -d "$root/apps/settings/build" ] || { echo "!! build the frontend first: (cd apps/settings && npm run build)" >&2; exit 1; }
# PRESENT IS NOT CURRENT. The binary check one line up says when it is older than
# its source, after a stale one cost a cycle on 28 August; the built frontend is
# the same trap on the other half, and this drive reads the SVELTE - every
# assertion below is about what the page renders. A rebuild is cheap; believing an
# old page is not.
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

cat > "$work/goto-list.js" <<'JS'
location.assign("/windows-apps");
return JSON.stringify({ went: true });
JS

# What a live session with no daemon must and must not say. The one sentence
# this page must never get wrong is claiming knowledge it does not have, so the
# probe checks both directions: the honest lines are there, the fixture is not.
cat > "$work/probe-list.js" <<'JS'
// WAIT FOR THE PAGE, do not count to three. `SHOOT_INJECT_SETTLE` is a fixed
// number and this asserted straight after it, so on 5 September - with a release
// build compiling in the background - the probe read a page that had not painted
// yet and reported the unavailable banner MISSING. A false red naming the app for
// what was the machine being busy, which is the same fault I spent the week
// taking out of the drive scripts' sleeps and had left sitting in my own probe.
//
// The wait is for the page's own heading, not for the banner: waiting for the
// thing under test would turn a real absence into a timeout with the same
// message, and then this could never fail for the reason it exists.
async function settled() {
  for (let i = 0; i < 60; i++) {
    if (/Windows apps/i.test(document.body.innerText)) return true;
    await new Promise((r) => setTimeout(r, 250));
  }
  return false;
}
const painted = await settled();
const text = document.body.innerText;
return JSON.stringify({
  painted,
  unavailable: text.includes("Cannot read your Windows apps right now"),
  noneUnknown: !text.includes("No Windows apps installed yet") && !text.includes("Installed apps"),
  runtimesUnknown: text.includes("Runtimes not known"),
  noFixture: !text.includes("Notepad++") && !text.includes("ledger-setup"),
});
JS

cat > "$work/goto-detail.js" <<'JS'
location.assign("/windows-apps/no-such-bottle");
return JSON.stringify({ went: true });
JS

cat > "$work/probe-detail.js" <<'JS'
const text = document.body.innerText;
return JSON.stringify({
  refused: text.includes("This Windows app is not in the list that could be read."),
  wayBack: [...document.querySelectorAll("button, a")].some((b) => b.textContent.trim() === "Windows apps"),
  noControls: !text.includes("DLL overrides") && !text.includes("Launch"),
});
JS

echo ">> the list on a machine with no bottle daemon"
SHOOT_INJECT="$work/goto-list.js:$work/probe-list.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/windows-apps-live.png" "" 8 \
  | tee "$work/run1.log" | grep -E "inject result" || true

grep -q '"painted":true' "$work/run1.log" || { echo "!! the page never painted; nothing below is about the app" >&2; exit 1; }
grep -q '"unavailable":true' "$work/run1.log" || { echo "!! the unavailable banner is missing" >&2; exit 1; }
grep -q '"noneUnknown":true' "$work/run1.log" || { echo "!! the list claims an emptiness it could not read" >&2; exit 1; }
grep -q '"runtimesUnknown":true' "$work/run1.log" || { echo "!! the runtimes row invents knowledge" >&2; exit 1; }
grep -q '"noFixture":true' "$work/run1.log" || { echo "!! fixture apps leaked into a live session" >&2; exit 1; }

echo ">> the detail route refuses an unreadable id"
SHOOT_INJECT="$work/goto-detail.js:$work/probe-detail.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/windows-apps-live-detail.png" "" 8 \
  | tee "$work/run2.log" | grep -E "inject result" || true

grep -q '"refused":true' "$work/run2.log" || { echo "!! the detail page did not refuse the unknown app" >&2; exit 1; }
grep -q '"wayBack":true' "$work/run2.log" || { echo "!! the sidebar has no way back to the list" >&2; exit 1; }
grep -q '"noControls":true' "$work/run2.log" || { echo "!! controls rendered for a bottle nobody can read" >&2; exit 1; }

# THE OTHER DIRECTION, added 5 September. Everything above proves the page is
# honest when nothing answers, which was the risk worth covering first - a fixture
# claiming "Notepad++ is installed" on a machine that has never seen Wine. But a
# page can be honest about absence and still wrong about presence, and the sentinel
# drive earned that lesson the same week: its value was checking both.
#
# The distinction asserted here is the one the page itself draws. `unavailable`
# means nobody answered; `empty` means the daemon answered and there are no
# bottles. Those are different sentences and only one of them is true with a
# daemon running, so this is not "did anything render" - it is whether the page
# can tell "I could not ask" from "I asked and the answer was none".
#
# The daemon gets its OWN runtime dir. `socket_path()` honours XDG_RUNTIME_DIR, so
# without this the test binds over the session's real bottled socket - which I did
# once already this summer, to a daemon somebody was using.
bottled="$root/target/debug/arlen-bottled"
if [ ! -x "$bottled" ]; then
  echo "!! no arlen-bottled at $bottled - the live half is SKIPPED, and the run" >&2
  echo "   above only says the page is honest with nothing behind it." >&2
  echo "   cargo build --manifest-path daemons/bottled/Cargo.toml" >&2
  exit 0
fi
require_fresh "$bottled" "$root/daemons/bottled/src" || exit 1

runtime="$work/run"
mkdir -p "$runtime/arlen"
echo ">> the same page with a bottle daemon answering"
XDG_RUNTIME_DIR="$runtime" XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/cfg" \
  "$bottled" >"$work/bottled.log" 2>&1 &
daemon_pid=$!
wait_for_socket "$runtime/arlen/bottled.sock" || {
  echo "!! the bottle daemon never bound its socket; its log:" >&2
  cat "$work/bottled.log" >&2
  exit 1
}

cat > "$work/probe-live.js" <<'JS'
for (let i = 0; i < 60; i++) {
  if (/Windows apps/i.test(document.body.innerText)) break;
  await new Promise((r) => setTimeout(r, 250));
}
const text = document.body.innerText.replace(/\s+/g, " ");
// The banner for "nobody answered" must be gone: a daemon IS answering. Its
// absence is the whole assertion - a page that shows it anyway is telling
// somebody their machine cannot be read when it just was.
const unavailable = /could not be reached|not available|no bottle daemon/i.test(text);
// And the page must say the answer was NONE, rather than saying nothing at all.
const saysSomething = text.trim().length > 40;
return JSON.stringify({ askedAndAnswered: !unavailable, saysSomething });
JS

SHOOT_INJECT="$work/goto-list.js:$work/probe-live.js" SHOOT_INJECT_SETTLE=3 \
  SHOOT_APP_ENV="ARLEN_BOTTLED_SOCKET=$runtime/arlen/bottled.sock" \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/windows-apps-live-daemon.png" "" 8 \
  | tee "$work/run3.log" | grep -E "inject result" || true

grep -q '"askedAndAnswered":true' "$work/run3.log" || {
  echo "!! the page still says nothing answered, with a daemon on its socket" >&2
  exit 1
}
grep -q '"saysSomething":true' "$work/run3.log" || {
  echo "!! the page rendered nothing at all with the daemon up" >&2
  exit 1
}

kill "$daemon_pid" 2>/dev/null || true
daemon_pid=""

echo ">> all green: the Windows page tells the truth with a daemon and without one"
