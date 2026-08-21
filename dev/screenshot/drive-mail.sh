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
  "$(printf '%s' "$out" | grep -q "pay at evil-collector.example before Friday" && echo 0 || echo 1)" "$out"

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

say "a header that asks to report back is named" \
  "$(printf '%s' "$out" | grep -q "Disposition-Notification-To" && echo 1 || echo 0)" "$out"

# The absence is stated, not silent. A reader told nothing takes the text part
# for the whole message, which is how the half that lies goes unread.
say "and the html part's absence is said out loud" \
  "$(printf '%s' "$out" | grep -q "also has an HTML part" && echo 1 || echo 0)" "$out"

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

say "launched with no message, it says where one comes from" \
  "$(printf '%s' "$bare" | grep -qE "file manager|Dateiverwaltung" && echo 1 || echo 0)" "$bare"

[ "$fail" = 0 ] && echo "the window says what the message is doing, including the half it will not show, and an empty one says where to get a message"
exit "$fail"
