#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the screenshot tool on a machine that cannot take a screenshot.
#
# WHY THIS EXISTS, AND IT IS THE WORST THING FOUND BY OPENING AN APP SO FAR.
# The tool asked the host for the screen, and on any host that could not give it
# one - a compositor without the screencopy interface, a failed capture call - it
# drew a synthetic desktop instead: a window card reading "Account / Signed in as
# tim@example.com / token: sk-9f2c1a7b4e88". Nothing said it was invented. Copy
# and Save sat beside it, enabled, and the floating thumbnail's Dismiss ran
# `autoSaveAndDismiss`, which WROTE THAT PICTURE to the screenshots directory
# without anyone pressing anything.
#
# So the failure mode was not "the app looks odd". It was: a person opens the
# screenshot tool, sees what looks like their screen, and sends a picture of a
# machine that does not exist, with a made-up account and a made-up token in it,
# to whoever they were trying to show something to.
#
# The fixture itself was not the mistake. Under plain vite there is no screen to
# capture and a sample IS the answer. The mistake was that "no host to ask" and
# "a host that could not capture" arrived at the caller as the same `null`, so
# one branch answered both. Same shape as the meetings shrug next door, and the
# same fix: make the outcomes different types, then say which one happened.
#
# Under this harness there is a real Tauri host and no Wayland compositor, so the
# run lands on exactly the shipped path: capture unavailable.
#
# Run: dev/screenshot/drive-screenshot.sh [path-to-arlen-screenshot]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-screenshot}"
fail=0

[ -x "$app" ] || { echo "no screenshot binary at $app"; exit 2; }

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

probe=$(mktemp)
cat > "$probe" <<'JS'
// ANY visible picture, not just the annotate canvas. The first cut of this asked
// only about `canvas`, and passed with the fault put back - because in the
// thumbnail phase the canvas is hidden and the invented desktop is in an `img`
// inside the floating thumbnail. It was true for a reason other than the one it
// was written for, which is the failure mode of every check in this directory.
const shown = [...document.querySelectorAll("canvas, img")].filter((e) => e.offsetParent !== null);
// What a person could press right now. A disabled or absent Save is the whole
// point: an affordance over an invented picture is worse than a missing one.
const actions = [...document.querySelectorAll("button")]
  .filter((b) => !b.disabled && b.offsetParent !== null)
  .map((b) => (b.innerText.trim() || b.getAttribute("aria-label") || b.title || "").trim())
  .filter(Boolean).join("|");
const body = document.body.innerText.replace(/\s+/g, " ").trim();
return `pictures=${shown.length} `
  + `actions=${JSON.stringify(actions.slice(0, 200))} body=${JSON.stringify(body.slice(0, 300))}`;
JS

echo "screenshot:"

run="$(mktemp -d)"
out=$(env XDG_STATE_HOME="$run/state" XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run" HOME="$run" \
  SHOOT_INJECT="$probe" "$here/shoot-app.sh" "$app" "$here/out/screenshot-app.png" 2>&1 \
  | sed -n 's/^inject result: //p')

say "with no way to capture, it says so" \
  "$(printf '%s' "$out" | grep -q "Could not take a picture of your screen" && echo 1 || echo 0)" "$out"

# Which cause, not just that there was one. "This compositor has no screen
# capture" and "the capture call threw" are different problems.
#
# THE CAUSE IS THE SENTENCE. This grepped for "no screen capture on this
# compositor" until 5 September, which is the COMPOSITOR'S OWN WORDS - the
# untranslated string the bridge used to draw in every locale, and the exact
# thing the `s.why.*` catalogue replaced. The check has been asserting the
# presence of that defect ever since, and could not say so because nothing runs
# these suites in the PR matrix. Same rot, same week, as the greeter's.
say "and it names why" \
  "$(printf '%s' "$out" | grep -q "does not offer screen capture to apps" && echo 1 || echo 0)" "$out"

# And that the sentence is one the app SHIPS. The bridge's own words reaching the
# screen is the regression the two checks above were written against, and a list
# of forbidden strings only ever catches the leaks somebody thought of: the
# greeter's blocklist missed its own historical leak verbatim. Asking whether the
# body contains a catalogue sentence catches every shape of it, and follows a
# rewording instead of going red at one.
catalogue="$root/apps/screenshot/src/lib/i18n/messages.ts"
why=$(grep -oE '"s\.why\.[A-Za-z]+": "[^"]+"' "$catalogue" | sed 's/^[^:]*: "//; s/"$//' | sort -u)
shipped=0
while IFS= read -r line; do
  [ -n "$line" ] || continue
  printf '%s' "$out" | grep -qF "$line" && shipped=1 && break
done <<< "$why"
say "and the reason is a sentence the app ships, not the bridge's own words" \
  "$shipped" "none of the s.why.* catalogue sentences is on screen: $out"

# THE case. This is the assertion that fails if anything ever restores the
# invented desktop on the no-capture path.
say "it does not draw a made-up desktop instead" \
  "$(printf '%s' "$out" | grep -q "pictures=0" && echo 1 || echo 0)" "$out"

# And nothing to press. Save wrote that picture to the screenshots directory;
# Dismiss wrote it without being asked. An offer to save what does not exist is
# the part that turns a rendering bug into somebody's file.
say "and offers nothing to save or copy" \
  "$(printf '%s' "$out" | grep -qE 'actions="[^"]*(Save|Copy|Annotate)' && echo 0 || echo 1)" "$out"

rm -rf "$run" "$probe" 2>/dev/null
[ "$fail" = 0 ] && echo "the screenshot tool shows your screen or nothing, never a plausible fake"
exit "$fail"
