#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the mailbox against a maildir on disk, and against no maildir at all.
#
# WHY SEPARATE FROM drive-mail.sh. That one drives the READER - one message
# handed over by the file manager through `mail_read` - and it is thorough about
# what a reader must and must not show. It says nothing about the mailbox,
# because until 5 September there was no mailbox: `mail_folders`, `mail_list` and
# `mail_open` did not exist and the store's catch answered an empty rail on a real
# host and a fixture under vite.
#
# THE CASE THAT MATTERS IS THE SECOND ONE. A fixture mailbox is the most
# convincing lie this app could tell: three plausible senders and a subject line
# are indistinguishable from somebody's actual mail at a glance. So this drives
# the app with a maildir and WITHOUT one, and the second run asserts the fixture
# names are nowhere on screen.
#
# WHY A DEBUG BINARY AND A PREVIEW rather than the release build the reader drive
# uses: the maildir path override is `#[cfg(debug_assertions)]`, deliberately -
# a release binary reading its mailbox location out of the environment it was
# started in is the hazard `install-helper` and the greeter both gate against. So
# the drive bends to the gate rather than the gate to the drive.
#
# Run: dev/screenshot/drive-mail-mailbox.sh [path-to-debug-arlen-mail-app]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# The frontend is served from a preview below, not baked into the binary, so the
# staleness guard compares Rust only (see shoot-app.sh).
export SHOOT_FRONTEND_SERVED=1
# shellcheck source=dev/screenshot/lib/wait.sh
. "$here/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$here/lib/preview.sh"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$here/lib/fresh.sh"
app="${1:-$root/target/debug/arlen-mail-app}"
work="$(mktemp -d)"
fail=0
PREVIEW_PGID=""

cleanup() { stop_preview; rm -rf "$work"; return 0; }
trap cleanup EXIT

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

require_fresh "$app" "$root/apps/mail/src-tauri/src" "$root/apps/mail/core/src" || exit 2
require_fresh_frontend "$root/apps/mail/build" "$root/apps/mail/src" || exit 2

# A maildir with one read and one unread message, and a Sent folder. Written here
# rather than committed: a fixture mailbox in the tree is a file somebody will
# one day mistake for a test account.
mkdir -p "$work/mail/cur" "$work/mail/new" "$work/mail/.Sent/cur" "$work/mail/.Sent/new"
# The body runs PAST the 140-character snippet cut on purpose: everything after
# it can only be on screen if the message was opened, which is what makes the
# open check below able to fail. See the comment there.
printf 'From: rosa@example.org\nSubject: the roof survey\nDate: Tue, 2 Jan 2024 10:00:00 +0000\n\nThe survey is attached. Page four is the section on the north roof and page five lists what the surveyor could not reach from the ladder. The closing line names the ridge tiles.\n' \
  > "$work/mail/cur/2.host:2,S"
printf 'From: bank@example.com\nSubject: your statement\nDate: Wed, 3 Jan 2024 09:00:00 +0000\n\nYour statement is ready.\n' \
  > "$work/mail/new/1.host"
printf 'From: me@example.org\nSubject: sent thing\nDate: Mon, 1 Jan 2024 08:00:00 +0000\n\nSent body.\n' \
  > "$work/mail/.Sent/cur/3.host:2,S"

echo "mail mailbox:"
start_preview "$root/apps/mail" 1454 || exit 1
wait_for_http "http://localhost:1454/" || exit 1

cat > "$work/probe.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// Wait for the app to have painted SOMETHING of its own, not for a number.
for (let i = 0; i < 60; i++) {
  if (document.body.innerText.trim().length > 20) break;
  await wait(250);
}
await wait(1500);
const text = document.body.innerText.replace(/\s+/g, " ").trim();
return JSON.stringify({
  text: text.slice(0, 600),
  // The fixture's own senders. If these appear, a sample mailbox reached a
  // window that is meant to be reading a real one.
  fixture: /Priya|Dominik|Renata|Aoife/.test(text),
});
JS

got=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/probe.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-mailbox.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

say "the rail and the list come from the maildir on disk" \
  "$(printf '%s' "$got" | grep -q "your statement" && echo 1 || echo 0)" "$got"

say "and a second folder's mail is not mixed into the inbox" \
  "$(printf '%s' "$got" | grep -q "sent thing" && echo 0 || echo 1)" "$got"

say "and no fixture sender is on a screen reading a real mailbox" \
  "$(printf '%s' "$got" | grep -q '"fixture":false' && echo 1 || echo 0)" "$got"

# And that a row opens. `mail_open` is the one that turns an id from the surface
# back into a path, through both gates - `safe_id` on what was typed and a
# containment check on what the filesystem resolved - so a row that lists but
# does not open would mean the id it handed out is not the id it accepts.
# TWICE WRONG AND STILL GREEN, until the picture was opened on 4 September. The
# click walked `closest("[role=row], li, tr, div")`, and a row here is a
# `button[role=option]` - which that list does not match - so `closest` sailed
# past it to the listbox DIV and clicked the container. Nothing opened. The
# assertion then looked for "The survey is attached", which is the SNIPPET the
# list row already draws, so it passed anyway. Either fault alone would have been
# caught by the other; together they made a check that asserts nothing.
#
# So the click names the row's own element, and the proof is two things the list
# CANNOT show: the empty state going away, and a phrase from past the snippet cut.
cat > "$work/open.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const rowFor = (t) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === t)[0];
for (let i = 0; i < 60; i++) { if (rowFor("the roof survey")) break; await wait(250); }
const cell = rowFor("the roof survey");
if (!cell) return JSON.stringify({ listed: false });
const row = cell.closest("button, [role=option], [role=row], li, tr");
if (!row) return JSON.stringify({ listed: true, clickable: false });
row.click();
for (let i = 0; i < 60; i++) {
  if (!document.body.innerText.includes("Select a message")) break;
  await wait(250);
}
const text = document.body.innerText.replace(/\s+/g, " ").trim();
return JSON.stringify({
  listed: true,
  clickable: true,
  // Only the reading pane can carry the tail: the snippet is cut at 140 chars.
  opened: text.includes("the ridge tiles") && !text.includes("Select a message"),
  text: text.slice(-300),
});
JS

opened=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/open.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-mailbox-open.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

say "and a row it listed is a row that opens" \
  "$(printf '%s' "$opened" | grep -q '"opened":true' && echo 1 || echo 0)" "$opened"

# THE case. With no maildir the app must show an unconnected mailbox, not the
# sample one - three plausible senders are indistinguishable from real mail.
empty=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/nothing-here" SHOOT_INJECT="$work/probe.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-no-mailbox.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

say "with no mailbox it shows none, rather than the sample one" \
  "$(printf '%s' "$empty" | grep -q '"fixture":false' && echo 1 || echo 0)" "$empty"

say "and says so rather than rendering an empty frame" \
  "$(printf '%s' "$empty" | grep -qE '"text":"[^"]{10,}' && echo 1 || echo 0)" "$empty"

[ "$fail" = 0 ] && echo "the mailbox reads this machine's maildir, and says so when there is none"
exit "$fail"
