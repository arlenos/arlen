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
out="${1:-$root/dev/screenshot/out}"
app="$root/target/debug/arlen-settings"
work="$(mktemp -d)"
preview_pid=""
cleanup() {
  [ -n "$preview_pid" ] && kill "$preview_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

[ -x "$app" ] || { echo "!! build it first: cargo build --manifest-path apps/settings/src-tauri/Cargo.toml" >&2; exit 1; }
[ -d "$root/apps/settings/build" ] || { echo "!! build the frontend first: (cd apps/settings && npm run build)" >&2; exit 1; }

(cd "$root/apps/settings" && npx vite preview --port 1421 --outDir build >/dev/null 2>&1) &
preview_pid=$!
sleep 2

cat > "$work/goto-list.js" <<'JS'
location.assign("/windows-apps");
return JSON.stringify({ went: true });
JS

# What a live session with no daemon must and must not say. The one sentence
# this page must never get wrong is claiming knowledge it does not have, so the
# probe checks both directions: the honest lines are there, the fixture is not.
cat > "$work/probe-list.js" <<'JS'
const text = document.body.innerText;
return JSON.stringify({
  unavailable: text.includes("Cannot read your Windows apps right now"),
  noneUnknown: text.includes("Not known: your Windows apps could not be read."),
  runtimesUnknown: text.includes("Runtimes not known"),
  noFixture: !text.includes("Notepad++") && !text.includes("LegacyTool"),
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
  wayBack: text.includes("All Windows apps"),
  noControls: !text.includes("DLL overrides") && !text.includes("Launch"),
});
JS

echo ">> the list on a machine with no bottle daemon"
SHOOT_INJECT="$work/goto-list.js:$work/probe-list.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/windows-apps-live.png" "" 8 \
  | tee "$work/run1.log" | grep -E "inject result" || true

grep -q '"unavailable":true' "$work/run1.log" || { echo "!! the unavailable banner is missing" >&2; exit 1; }
grep -q '"noneUnknown":true' "$work/run1.log" || { echo "!! the list claims an emptiness it could not read" >&2; exit 1; }
grep -q '"runtimesUnknown":true' "$work/run1.log" || { echo "!! the runtimes row invents knowledge" >&2; exit 1; }
grep -q '"noFixture":true' "$work/run1.log" || { echo "!! fixture apps leaked into a live session" >&2; exit 1; }

echo ">> the detail route refuses an unreadable id"
SHOOT_INJECT="$work/goto-detail.js:$work/probe-detail.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/windows-apps-live-detail.png" "" 8 \
  | tee "$work/run2.log" | grep -E "inject result" || true

grep -q '"refused":true' "$work/run2.log" || { echo "!! the detail page did not refuse the unknown app" >&2; exit 1; }
grep -q '"wayBack":true' "$work/run2.log" || { echo "!! the way back to the list is missing" >&2; exit 1; }
grep -q '"noControls":true' "$work/run2.log" || { echo "!! controls rendered for a bottle nobody can read" >&2; exit 1; }

echo ">> all green: the Windows page tells the truth without a daemon"
