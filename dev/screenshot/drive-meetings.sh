#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the meetings app and read what it says about meetings it cannot read.
#
# WHY THIS EXISTS. Meetings has a daemon behind it and a KG under that, and the
# app had never been opened against either. Its own tests cover the store in a
# hostless browser, where the answer is always the fixture; what none of them can
# answer is what a real session with nothing behind it shows, which is the state
# every reader hits first.
#
# THE CASE THAT MATTERS IS THE REFUSAL, AND WHETHER IT NAMES A CAUSE. Opening it
# the first time it said "Cannot read your meetings right now, so none are shown"
# and stopped there, because the catch in `loadMeetings` set a flag and dropped
# the error. A reader then cannot tell a daemon that is not running from a
# permission they do not have from a store that is corrupt - three different
# things to do about it, rendered as one shrug. The clock next door says "the
# clock service is not running, so your alarms are not being kept" and is worth
# more for exactly that reason. So this asserts the sentence AND its cause.
#
# The second case is the distinction the app documents in its own markup: "these
# are examples" and "I could not read yours" are different facts, and only one of
# them is about this machine. A real session must never show both.
#
# Run: dev/screenshot/drive-meetings.sh [path-to-arlen-meetings]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-meetings}"
fail=0

[ -x "$app" ] || { echo "no meetings binary at $app"; exit 2; }

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

probe=$(mktemp)
cat > "$probe" <<'JS'
const body = document.body.innerText.replace(/\s+/g, " ").trim();
const buttons = [...document.querySelectorAll("button")]
  .map((b) => b.innerText.trim()).filter(Boolean).join("|");
const rows = document.querySelectorAll("[id^=meeting-]").length;
return `rows=${rows} buttons=${JSON.stringify(buttons.slice(0, 120))} body=${JSON.stringify(body.slice(0, 300))}`;
JS

echo "meetings:"

# A session of its own: no meetings daemon, no graph socket, and the developer's
# own state neither read nor disturbed.
run="$(mktemp -d)"
out=$(env XDG_STATE_HOME="$run/state" XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run" HOME="$run" \
  SHOOT_INJECT="$probe" "$here/shoot-app.sh" "$app" "$here/out/meetings.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# Evidence the window came up as the app rather than as a blank frame, because
# every sentence below is also absent from nothing at all.
say "the app came up as a meetings app" \
  "$(printf '%s' "$out" | grep -q "Start a meeting" && echo 1 || echo 0)" "$out"

say "with nothing behind it, it says the meetings cannot be read" \
  "$(printf '%s' "$out" | grep -qE "meetings service is not running|not allowed to read your meetings|Cannot read your meetings" \
     && echo 1 || echo 0)" "$out"

# THE case, and it changed shape on 22 August. It used to ask for a colon and
# something after it, because the sentence was a generic one with a raw error
# glued on and the question was whether the cause survived the catch. The store
# now carries the WORD (`unavailable` or `denied`), so the question is sharper:
# the window must say WHICH of the two, and this drive runs with no daemon, so
# the answer has to be the absent one. A window that shrugged, or that said
# "refused" here, fails.
say "and it names which of the two, not a shrug" \
  "$(printf '%s' "$out" | grep -q "meetings service is not running" \
     && ! printf '%s' "$out" | grep -q "not allowed to read" && echo 1 || echo 0)" "$out"

# The app's own distinction, asserted so it cannot quietly collapse: a real
# session that could not read anything must not also claim to be showing
# examples. One of those sentences is about this machine and the other is not.
say "it does not also claim to be showing examples" \
  "$(printf '%s' "$out" | grep -q "Example meetings" && echo 0 || echo 1)" "$out"

# And it shows no rows to click. A refusal above a list of clickable meetings is
# the worst of the three states: the click either does nothing, which lies about
# the control, or it acts on invented data.
say "and it offers no meeting rows it cannot stand behind" \
  "$(printf '%s' "$out" | grep -q "rows=0" && echo 1 || echo 0)" "$out"

rm -rf "$run" "$probe" 2>/dev/null
[ "$fail" = 0 ] && echo "the meetings app says what is wrong, not just that something is"
exit "$fail"
