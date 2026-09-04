#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open a message that is trying something, and read what the window says about it.
#
# WHY THE FIXTURES ARE HOSTILE. Every claim this app makes is a claim about a
# message somebody else wrote, and the interesting messages are the ones written
# to be misread. So the file opened here is a phishing shape - the text part
# names one host, the HTML part names another, and a header asks to be told when
# it was read - because a mail window that renders that as an ordinary invoice is
# the defect, and it is a defect a screenshot of a well-formed message cannot
# show.
#
# THE LOAD-BEARING CASE IS THE ONE ABOUT ABSENCE. `mail-app.md` section 3 makes
# not rendering the HTML part an architectural constraint rather than a
# preference, so this asserts that the HTML part's own sentence never reaches the
# screen - while the divergence notice, which names the WORDS that differ, does.
# Those two are easy to conflate and only one of them is safe.
#
# Run: dev/screenshot/drive-mail.sh [path-to-arlen-mail-app]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-mail-app}"
fix="$HOME/.cache/arlen-drive-mail"
fail=0

[ -x "$app" ] || { echo "no mail binary at $app"; exit 2; }
rm -rf "$fix"; mkdir -p "$fix"

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

# A message whose two halves disagree about where the money goes, carrying a read
# receipt. Written as a literal file rather than through a library so the message
# the window opens is the message this script describes.
# Appended line by line rather than one quoted format string spanning several
# lines. A trailing backslash inside single quotes is NOT a continuation - it is
# a literal backslash - so the old version put a stray `\` line inside the text
# part, and the window faithfully showed it. The fixture was malformed and the
# screenshot was right.
: > "$fix/invoice.eml"
{
  printf 'From: billing@example.com\r\n'
  printf 'To: you@example.com, treasury@example.com\r\n'
  printf 'Cc: audit@example.com\r\n'
  printf 'Subject: Your invoice is ready\r\n'
  printf 'Date: Tue, 19 Aug 2026 09:00:00 +0000\r\n'
  printf 'MIME-Version: 1.0\r\n'
  printf 'Disposition-Notification-To: watcher@example.com\r\n'
  printf 'Content-Type: multipart/alternative; boundary=b\r\n\r\n'
  printf -- '--b\r\nContent-Type: text/plain\r\n\r\n'
  printf 'Please pay at example.com before Friday.\r\n'
  printf -- '--b\r\nContent-Type: text/html\r\n\r\n'
  printf '<p>Please pay at evil-collector.example before Friday.</p>\r\n'
  printf -- '--b--\r\n'
} >> "$fix/invoice.eml"

# And one that contradicts itself about its own format, which the rules refuse.
: > "$fix/ambiguous.eml"
{
  printf 'From: someone@example.com\r\n'
  printf 'Subject: Which am I\r\n'
  printf 'Content-Type: text/plain\r\n'
  printf 'Content-Type: text/html\r\n\r\n'
  printf 'Pick one.\r\n'
} >> "$fix/ambiguous.eml"

cat > "$fix/probe.js" <<'JS'
await new Promise((r) => setTimeout(r, 2000));
const body = document.body.innerText.replace(/\s+/g, " ").trim();
// The body BLOCK on its own, so "what the reader is shown as the message" can be
// asserted separately from every notice around it.
const shown = (document.querySelector(".body")?.innerText ?? "").replace(/\s+/g, " ").trim();
return `shown=${JSON.stringify(shown.slice(0, 160))} body=${JSON.stringify(body.slice(0, 520))}`;
JS

echo "mail:"

out=$(SHOOT_APP_ARGS="$fix/invoice.eml" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/mail.png" 2>&1 | sed -n 's/^inject result: //p')

# Positive first: every assertion about absence below is also satisfied by a
# window that never opened the file.
say "the message is open and named" \
  "$(printf '%s' "$out" | grep -q "Your invoice is ready" && echo 1 || echo 0)" "$out"

# The caveat is ON the sender line. A display name is whatever the sender typed,
# and this is the field a reader trusts hardest.
say "the sender is shown as a claim rather than as identity" \
  "$(printf '%s' "$out" | grep -q "not verified" && echo 1 || echo 0)" "$out"

# THE case. The HTML part's own sentence must never reach the screen; the words
# that differ, which the divergence notice names, must.
say "the html part's sentence is not on the screen" \
  "$(case "$out" in ""|REFUSED:*) echo 0;; *) printf '%s' "$out" | grep -q "pay at evil-collector.example before Friday" && echo 0 || echo 1;; esac)" "$out"

# The words are named WHOLE. This used to check for "collector", which is a
# fragment of `evil-collector.example` produced by splitting on punctuation, so
# the assertion passed on the noise it was meant to catch.
say "but the reader is told the two parts disagree, in the differing words" \
  "$(printf '%s' "$out" | grep -q "versions of this message differ" \
     && printf '%s' "$out" | grep -q "evil-collector.example" && echo 1 || echo 0)" "$out"

# What the reader is shown AS the message is the text part, verbatim. A window
# that showed neither would pass the case above.
say "and what it shows as the message is the text part" \
  "$(printf '%s' "$out" | grep -q 'shown="Please pay at example.com before Friday.' && echo 1 || echo 0)" "$out"

# The date is the reader's, not the machine's. This window printed
# `2026-08-19T09:00:00Z` until 21 August, in every language.
say "the date is written the way a reader writes one" \
  "$(printf '%s' "$out" | grep -q "August 19, 2026" \
     && ! printf '%s' "$out" | grep -q "2026-08-19T09" && echo 1 || echo 0)" "$out"

say "a header that asks to report back is named" \
  "$(printf '%s' "$out" | grep -q "Disposition-Notification-To" && echo 1 || echo 0)" "$out"

# The absence is stated, not silent. A reader told nothing takes the text part
# for the whole message, which is how the half that lies goes unread.
say "and the html part's absence is said out loud" \
  "$(printf '%s' "$out" | grep -q "also has an HTML part" && echo 1 || echo 0)" "$out"

# AN INVITATION, which is the seam with the calendar (section 4). The load-bearing
# part is that the window says the part is there AND says nobody read it: this app
# does not do iTIP, so a sentence that sounded like it had understood the meeting
# would be a claim it cannot back. The calendar part is deliberately NOT marked as
# an attachment here, which is the ordinary shape and the reason it was invisible
# before - the "carries N files" line never mentioned it.
: > "$fix/invite.eml"
{
  printf 'From: ada@example.org\r\n'
  printf 'Subject: Lunch on Friday\r\n'
  printf 'Date: Fri, 21 Aug 2026 09:15:00 +0200\r\n'
  printf 'MIME-Version: 1.0\r\n'
  printf 'Content-Type: multipart/alternative; boundary=b\r\n\r\n'
  printf -- '--b\r\nContent-Type: text/plain\r\n\r\n'
  printf 'Lunch on Friday?\r\n'
  printf -- '--b\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\n'
  printf 'BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Lunch\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n'
  printf -- '--b--\r\n'
} >> "$fix/invite.eml"

inv=$(SHOOT_APP_ARGS="$fix/invite.eml" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/mail-invitation.png" 2>&1 | sed -n 's/^inject result: //p')

say "an invitation in a message is named" \
  "$(printf '%s' "$inv" | grep -q "carries an invitation" && echo 1 || echo 0)" "$inv"

# The second half of the same sentence, and the one that keeps it honest.
say "and the window says it did not read it" \
  "$(printf '%s' "$inv" | grep -q "Nothing here has read it" && echo 1 || echo 0)" "$inv"

# ONE PART, ONE MENTION. The parser returns a text/calendar part as an attachment
# whether or not the sender marked it as one, so the window used to say "carries
# an invitation" and "carries one file, not opened: a file the sender did not
# name" about the same bytes.
say "the invitation is not listed a second time as an unnamed file" \
  "$(case "$inv" in ""|REFUSED:*) echo 0;; *) printf '%s' "$inv" | grep -q "carries one file" && echo 0 || echo 1;; esac)" "$inv"

# The protocol token stays out of the window: REQUEST is for the machine.
say "the method is not printed raw at the reader" \
  "$(case "$inv" in ""|REFUSED:*) echo 0;; *) printf '%s' "$inv" | grep -q "REQUEST" && echo 0 || echo 1;; esac)" "$inv"

# A SEALED MESSAGE. PGP and S/MIME messages have no readable text part, so the
# window said "this message has no text part" over two attachments named things
# like `encrypted.asc` - which describes an empty message rather than a sealed
# one. Nothing here decrypts anything; the point is that it stops implying it
# read the message.
: > "$fix/sealed.eml"
{
  printf 'From: ada@example.org\r\n'
  printf 'Subject: Secret\r\n'
  printf 'MIME-Version: 1.0\r\n'
  printf 'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary=b\r\n\r\n'
  printf -- '--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n'
  printf -- '--b\r\nContent-Type: application/octet-stream; name=encrypted.asc\r\n\r\n'
  printf -- '-----BEGIN PGP MESSAGE-----\r\n-----END PGP MESSAGE-----\r\n'
  printf -- '--b--\r\n'
} >> "$fix/sealed.eml"

sealed=$(SHOOT_APP_ARGS="$fix/sealed.eml" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/mail-sealed.png" 2>&1 | sed -n 's/^inject result: //p')

say "an encrypted message says it is encrypted" \
  "$(printf '%s' "$sealed" | grep -q "encrypted with PGP" && echo 1 || echo 0)" "$sealed"

# Its envelope is not an enclosure. The window said "This message carries 2
# files, not opened" over the PGP version part and the ciphertext.
say "and does not offer its envelope as files somebody sent" \
  "$(case "$sealed" in ""|REFUSED:*) echo 0;; *) printf '%s' "$sealed" | grep -q "carries 2 files" && echo 0 || echo 1;; esac)" "$sealed"

say "and does not report itself as a message with no text" \
  "$(case "$sealed" in ""|REFUSED:*) echo 0;; *) printf '%s' "$sealed" | grep -q "no text part" && echo 0 || echo 1;; esac)" "$sealed"

amb=$(SHOOT_APP_ARGS="$fix/ambiguous.eml" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/mail-ambiguous.png" 2>&1 | sed -n 's/^inject result: //p')

# Section 2's whole point, reaching a person: a message that says two
# contradictory things about its own format is not interpreted, and the reader is
# told which header did it rather than being handed a guess.
say "a message that contradicts itself about its format is refused, not guessed at" \
  "$(printf '%s' "$amb" | grep -q "contradicts itself" \
     && printf '%s' "$amb" | grep -q "Content-Type" && echo 1 || echo 0)" "$amb"

# LAUNCHED WITH NOTHING, which is what the launcher gives a person. Nothing here
# opened that state, and it is where three apps drifted into three different
# sentences: this window cannot open a message itself, so naming a file extension
# and no route reads as an instruction it gives you no way to follow.
cat > "$fix/p-bare.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200);
JS

bare=$(SHOOT_INJECT="$fix/p-bare.js" \
  "$here/shoot-app.sh" "$app" "$here/out/mail-no-file.png" 2>&1 | sed -n 's/^inject result: //p')

# The client shape: with no maildir and no launched file, the one sentence names
# the one fact, and the fact is a PLACE. It said "No account is connected" until
# 5 September, which named a thing this system does not have - no setting, no
# command, no route anywhere is an account - so a reader went looking for one
# instead of at the directory that would have worked. It used to add that a
# message opened from Files still shows here, which is the surface narrating a
# row the reader sees when it exists (design-system.md 6.7).
say "launched with no message, it names the place it looked for mail" \
  "$(printf '%s' "$bare" | grep -q "Maildir" && echo 1 || echo 0)" "$bare"

# And the word that sent the reader nowhere is gone, in both languages.
say "and does not offer an account this machine has no way to connect" \
  "$(printf '%s' "$bare" | grep -qiE "account|konto" && echo 0 || echo 1)" "$bare"

[ "$fail" = 0 ] && echo "the window says what the message is doing, including the half it will not show, and an empty one says where to get a message"
exit "$fail"
