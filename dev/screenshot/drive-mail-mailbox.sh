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
mkdir -p "$work/mail/cur" "$work/mail/new" "$work/mail/.Sent/cur" "$work/mail/.Sent/new" \
  "$work/mail/.Archive/cur" "$work/mail/.Archive/new" "$work/mail/.Trash/cur" "$work/mail/.Trash/new"
# The body runs PAST the 140-character snippet cut on purpose: everything after
# it can only be on screen if the message was opened, which is what makes the
# open check below able to fail. See the comment there.
printf 'From: rosa@example.org\nSubject: the roof survey\nDate: Tue, 2 Jan 2024 10:00:00 +0000\n\nThe survey is attached. Page four is the section on the north roof and page five lists what the surveyor could not reach from the ladder. The closing line names the ridge tiles.\n' \
  > "$work/mail/cur/2.host:2,S"
printf 'From: bank@example.com\nSubject: your statement\nDate: Wed, 3 Jan 2024 09:00:00 +0000\n\nYour statement is ready.\n' \
  > "$work/mail/new/1.host"
printf 'From: me@example.org\nSubject: sent thing\nDate: Mon, 1 Jan 2024 08:00:00 +0000\n\nSent body.\n' \
  > "$work/mail/.Sent/cur/3.host:2,S"
# A third one, for the draft probe. It has its own because the probes run in
# order against ONE mailbox: by the time that one runs, the archive and delete
# probes have emptied the inbox, and a reply needs something to reply to.
printf 'From: gutters@example.org\nSubject: the gutter quote\nDate: Thu, 4 Jan 2024 11:00:00 +0000\n\nQuote attached.\n' \
  > "$work/mail/cur/4.host:2,S"
# And a fourth, for the keyboard probe, by the same rule.
printf 'From: fences@example.org\nSubject: the fence estimate\nDate: Fri, 5 Jan 2024 12:00:00 +0000\n\nEstimate attached.\n' \
  > "$work/mail/cur/5.host:2,S"

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
  // The writes. A maildir this client cannot write to offers no Compose: a row
  // that "archives" a file which is back at the next start is the sample's
  // trick, and only the sample gets to play it.
  compose: !!document.querySelector("#mail-compose"),
  // The empty-mailbox sentence must name the PLACE it looked. It said "No
  // account is connected" until 5 September - a thing this system has no way to
  // connect - so a reader went hunting for a setting instead of a directory.
  namesPlace: /no mailbox at \S/.test(text),
  saysAccount: /account|konto/i.test(text),
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
# Compose stays absent even though the mailbox now keeps writes, and the two are
# different questions: a maildir keeps a draft, an archive and a delete, but
# sending needs an account and Arlen has no account surface, so starting a
# message from nothing is the one entry that is not offered (`mail-app.md`).
say "and a mailbox with nowhere to send offers no Compose" \
  "$(printf '%s' "$got" | grep -q '"compose":false' && echo 1 || echo 0)" "$got"

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
  // With a message open the header shows the writes, because a maildir keeps
  // every one of them: the read mark is a rename, archive is a rename, delete is
  // a rename or an unlink. They were absent while they were pretences.
  writes: [...document.querySelectorAll("button")].some((b) =>
    /^(Archive|Delete|Reply|Forward)$/.test(b.getAttribute("aria-label") || "")),
  text: text.slice(-300),
});
JS

opened=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/open.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-mailbox-open.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

say "and a row it listed is a row that opens" \
  "$(printf '%s' "$opened" | grep -q '"opened":true' && echo 1 || echo 0)" "$opened"
say "and an open message offers the archive, delete, reply and forward it can keep" \
  "$(printf '%s' "$opened" | grep -q '"writes":true' && echo 1 || echo 0)" "$opened"

# THE WRITES, AND THE PROOF IS THE DISK. Every check above this reads the window,
# which is exactly the wrong instrument here: the defect these four commands were
# built to remove is a surface that reports a write nobody kept. So the probe
# presses the controls and the assertions afterwards look at the maildir - a
# message that reads as archived and is still in the inbox is the failure, and no
# amount of reading the screen would show it.
cat > "$work/write.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const rowFor = (t) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === t)[0];
const press = (label) => {
  const b = [...document.querySelectorAll("button")]
    .find((x) => (x.getAttribute("aria-label") || "") === label);
  if (b) b.click();
  return !!b;
};
// Open the unread one, which marks it read.
for (let i = 0; i < 60; i++) { if (rowFor("your statement")) break; await wait(250); }
const cell = rowFor("your statement");
if (!cell) return JSON.stringify({ listed: false });
cell.closest("button, [role=option], [role=row], li, tr").click();
await wait(1500);
const archived = press("Archive");
await wait(1500);
// Then open the read one and delete it.
const other = rowFor("the roof survey");
if (other) other.closest("button, [role=option], [role=row], li, tr").click();
await wait(1200);
const deleted = press("Delete");
await wait(1500);
return JSON.stringify({ listed: true, archived, deleted });
JS

wrote=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/write.js" SHOOT_INJECT_SETTLE=6 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-writes.png" "" 14 2>&1 \
  | sed -n 's/^inject result: //p')

say "the controls are there to press" \
  "$(printf '%s' "$wrote" | grep -q '"archived":true' \
     && printf '%s' "$wrote" | grep -q '"deleted":true' && echo 1 || echo 0)" "$wrote"

# Opening a message marks it read, and a maildir says so in the filename: out of
# new/ and carrying S. Checked wherever it ended up, since it was archived after.
seen_name=$(find "$work/mail" -name '1.host*' -printf '%P\n' 2>/dev/null | head -1)
say "reading a message leaves the read mark on the disk, not just on the screen" \
  "$(case "$seen_name" in *new/*) echo 0;; *:2,*S*) echo 1;; *) echo 0;; esac)" \
  "the file is now [$seen_name]"

say "and archive moves the file into the archive folder" \
  "$(case "$seen_name" in .Archive/*) echo 1;; *) echo 0;; esac)" \
  "the file is now [$seen_name]"

deleted_name=$(find "$work/mail/.Trash" -name '2.host*' -printf '%P\n' 2>/dev/null | head -1)
say "and delete moves the file into the trash rather than off the disk" \
  "$([ -n "$deleted_name" ] && echo 1 || echo 0)" \
  "the trash holds [$deleted_name]"

# THE FOURTH WRITE, and the only way to reach it live. Compose is absent on a
# mailbox with nowhere to send, so nothing starts a message from nothing - but
# Reply answers one that is in front of you, and what it opens saves to Drafts.
# The mailbox has no drafts folder, which is the interesting half: `mail_draft_save`
# makes one, the single place these writes create a directory, because a draft has
# nowhere else to be.
cat > "$work/draft.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const rowFor = (t) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === t)[0];
for (let i = 0; i < 60; i++) { if (rowFor("the gutter quote")) break; await wait(250); }
const cell = rowFor("the gutter quote");
if (!cell) return JSON.stringify({ listed: false });
cell.closest("button, [role=option], [role=row], li, tr").click();
await wait(1800);
const reply = [...document.querySelectorAll("button")]
  .find((b) => (b.getAttribute("aria-label") || "") === "Reply");
if (!reply) return JSON.stringify({ listed: true, replied: false });
reply.click();
await wait(1200);
const body = document.querySelector("#compose-body");
const save = document.querySelector("#compose-save-draft");
if (!body || !save) return JSON.stringify({ listed: true, replied: true, composer: false });
// Through the DOM setter so Svelte's binding sees it, the way the kit's own
// tests drive an input.
const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")
  || Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
set.set.call(body, "the ladder did not reach");
body.dispatchEvent(new Event("input", { bubbles: true }));
await wait(400);
save.click();
await wait(2000);
return JSON.stringify({
  listed: true,
  replied: true,
  composer: true,
  text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200),
});
JS

drafted=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/draft.js" SHOOT_INJECT_SETTLE=6 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-draft.png" "" 14 2>&1 \
  | sed -n 's/^inject result: //p')

say "reply opens a composer on a mailbox that cannot send" \
  "$(printf '%s' "$drafted" | grep -q '"composer":true' && echo 1 || echo 0)" "$drafted"

draft_file=$(find "$work/mail" -path '*Drafts*' -name '*:2,*' -print -quit 2>/dev/null)
say "and saving the reply writes a draft into a drafts folder it had to make" \
  "$([ -n "$draft_file" ] && echo 1 || echo 0)" "the drafts folder holds [$draft_file]"

# THE KEYBOARD, which reaches the same writes down a different path: its own
# handler, its own gate (typing in the search box must stay typing) and its own
# idea of which row is meant (the list's anchor, not the header's selection). A
# fault in any of those is invisible from the button path, and `e` is the one the
# ruling names.
cat > "$work/key.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const rowFor = (t) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === t)[0];
for (let i = 0; i < 60; i++) { if (rowFor("the fence estimate")) break; await wait(250); }
const cell = rowFor("the fence estimate");
if (!cell) return JSON.stringify({ listed: false });
const row = cell.closest("button, [role=option], [role=row], li, tr");
row.click();
await wait(1500);
row.focus();
row.dispatchEvent(new KeyboardEvent("keydown", { key: "e", bubbles: true }));
await wait(2000);
return JSON.stringify({ listed: true, text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 160) });
JS

keyed=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/key.js" SHOOT_INJECT_SETTLE=6 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-keyboard.png" "" 14 2>&1 \
  | sed -n 's/^inject result: //p')

keyed_name=$(find "$work/mail" -name '5.host*' -printf '%P\n' 2>/dev/null | head -1)
say "the keyboard reaches the same writes, and e archives the anchored row" \
  "$(case "$keyed_name" in .Archive/*) echo 1;; *) echo 0;; esac)" \
  "the file is now [$keyed_name] - $keyed"

# A WRITE THE MAILBOX REFUSES. Every check above is about a write that lands;
# this is the other half, and it is the one the whole feature exists to get
# right: a surface that reports an archive nobody kept is exactly the pretence
# these commands replaced. The archive folder is made unwritable, so the rename
# fails for a reason the app cannot fix, and the demand is that NOTHING moves -
# not the file, and not the row.
chmod 500 "$work/mail/.Archive/cur"
cat > "$work/refused.js" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const rowFor = (t) => [...document.querySelectorAll("*")]
  .filter((e) => e.children.length === 0 && (e.textContent || "").trim() === t)[0];
for (let i = 0; i < 60; i++) { if (rowFor("the gutter quote")) break; await wait(250); }
const cell = rowFor("the gutter quote");
if (!cell) return JSON.stringify({ listed: false });
cell.closest("button, [role=option], [role=row], li, tr").click();
await wait(1500);
const b = [...document.querySelectorAll("button")]
  .find((x) => (x.getAttribute("aria-label") || "") === "Archive");
if (!b) return JSON.stringify({ listed: true, pressed: false });
b.click();
await wait(2500);
// Is the message still where it was, as far as the window is concerned?
return JSON.stringify({
  listed: true,
  pressed: true,
  stillListed: !!rowFor("the gutter quote"),
});
JS

refused=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/mail" SHOOT_INJECT="$work/refused.js" SHOOT_INJECT_SETTLE=6 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-refused-write.png" "" 14 2>&1 \
  | sed -n 's/^inject result: //p')
chmod 700 "$work/mail/.Archive/cur"

refused_name=$(find "$work/mail" -name '4.host*' -printf '%P\n' 2>/dev/null | head -1)
# PRESSED FIRST. Both demands below are satisfied by a run where the control was
# never found - nothing moves if nothing was clicked - so the press is asserted
# on its own rather than assumed by the two checks that rest on it.
say "the refused case actually got as far as pressing Archive" \
  "$(printf '%s' "$refused" | grep -q '"pressed":true' && echo 1 || echo 0)" "$refused"

say "a write the mailbox refuses leaves the message where it was" \
  "$(case "$refused_name" in .Archive/*) echo 0;; "") echo 0;; *) echo 1;; esac)" \
  "the file is now [$refused_name]"

say "and the list does not show it as archived" \
  "$(printf '%s' "$refused" | grep -q '"stillListed":true' && echo 1 || echo 0)" "$refused"

say "and the draft on disk carries what was typed and the subject replied to" \
  "$([ -n "$draft_file" ] && grep -q "the ladder did not reach" "$draft_file" \
     && grep -q "^Subject: Re: the gutter quote" "$draft_file" && echo 1 || echo 0)" \
  "$([ -n "$draft_file" ] && head -6 "$draft_file" | tr '\n' '|')"

# THE case. With no maildir the app must show an unconnected mailbox, not the
# sample one - three plausible senders are indistinguishable from real mail.
empty=$(SHOOT_APP_ENV="ARLEN_MAILDIR=$work/nothing-here" SHOOT_INJECT="$work/probe.js" SHOOT_INJECT_SETTLE=3 \
  "$here/shoot-app.sh" "$app" "$here/out/mail-no-mailbox.png" "" 10 2>&1 \
  | sed -n 's/^inject result: //p')

say "with no mailbox it shows none, rather than the sample one" \
  "$(printf '%s' "$empty" | grep -q '"fixture":false' && echo 1 || echo 0)" "$empty"

say "and says so rather than rendering an empty frame" \
  "$(printf '%s' "$empty" | grep -qE '"text":"[^"]{10,}' && echo 1 || echo 0)" "$empty"
say "and what it says is where it looked, so the reader knows where to put one" \
  "$(printf '%s' "$empty" | grep -q '"namesPlace":true' && echo 1 || echo 0)" "$empty"
say "and does not name an account, which nothing on this machine can connect" \
  "$(printf '%s' "$empty" | grep -q '"saysAccount":false' && echo 1 || echo 0)" "$empty"

[ "$fail" = 0 ] && echo "the mailbox reads this machine's maildir, and says so when there is none"
exit "$fail"
