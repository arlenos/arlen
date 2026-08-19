#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the calendar over a real .ics file and read what came back.
#
# WHY A FIXTURE FILE RATHER THAN A MOCK. Every claim this app makes is a claim
# about iCalendar, and iCalendar is where the traps are: a folded line, an
# escaped comma, an all-day entry with no time of day, a time in UTC that is not
# the reader's time. A mock would answer whatever the app expects. A file
# answers what RFC 5545 says.
#
# The fixture is generated here rather than committed because it has to be read
# alongside what each case is FOR, and a file sitting in the tree loses that.
#
# Run: dev/screenshot/drive-calendar.sh [path-to-arlen-calendar-app]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl and the run then reports on whatever dev
# server holds that port.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-calendar-app}"
fix="$HOME/.cache/arlen-drive-calendar"
fail=0

[ -x "$app" ] || { echo "no calendar binary at $app"; exit 2; }
rm -rf "$fix"
mkdir -p "$fix/arlen/calendars" "$here/out"

# One file, four traps. The folded SUMMARY continues on a line starting with two
# spaces; one of those spaces is the fold marker and one is a real space.
cat > "$fix/arlen/calendars/work.ics" <<'ICS'
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Arlen//drive//EN
BEGIN:VTIMEZONE
TZID:Europe/Vienna
END:VTIMEZONE
BEGIN:VEVENT
UID:standup@arlen
SUMMARY:Morning standup
DTSTART;TZID=Europe/Vienna:20260819T090000
DTEND;TZID=Europe/Vienna:20260819T091500
LOCATION:Room 2
RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR
END:VEVENT
BEGIN:VEVENT
UID:review@arlen
SUMMARY:Design review with a title long enough that a writer
  would fold the line here
DTSTART;TZID=Europe/Vienna:20260819T140000
DURATION:PT1H30M
END:VEVENT
BEGIN:VEVENT
UID:holiday@arlen
SUMMARY:Public holiday
DTSTART;VALUE=DATE:20260820
END:VEVENT
BEGIN:VEVENT
UID:call@arlen
SUMMARY:Call with Lisbon\, then lunch
DTSTART:20260820T160000Z
END:VEVENT
END:VCALENDAR
ICS

say() {  # say <name> <ok> <got>
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

drive() {  # drive <probe-js> <data-home> <out-png>
  printf '%s' "$(XDG_DATA_HOME="$2" SHOOT_INJECT="$1" \
    "$here/shoot-app.sh" "$app" "$here/out/$3" 2>&1 \
    | sed -n 's/^inject result: //p')"
}

echo "calendar:"

cat > "$fix/p-agenda.js" <<'JS'
await new Promise(r => setTimeout(r, 1200));
return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 400);
JS
got=$(drive "$fix/p-agenda.js" "$fix" calendar-agenda.png)

say "the events in the file are shown, grouped under their own day" \
  "$(printf '%s' "$got" | grep -q "Wednesday, August 19" \
     && printf '%s' "$got" | grep -q "Thursday, August 20" && echo 1 || echo 0)" "$got"

# The fold marker is not part of the value. A reader that skips unfolding shows
# half a title and treats the rest as an unknown property.
say "a folded title is rejoined into one sentence" \
  "$(printf '%s' "$got" | grep -q "a writer would fold the line here" && echo 1 || echo 0)" "$got"

say "an escaped comma comes back as a comma" \
  "$(printf '%s' "$got" | grep -q "Call with Lisbon, then lunch" && echo 1 || echo 0)" "$got"

# DURATION and DTEND are two ways of saying the same thing and both have to
# arrive as an end time.
say "a DURATION becomes an end time" \
  "$(printf '%s' "$got" | grep -q "14:00–15:30" && echo 1 || echo 0)" "$got"

say "an all-day entry says so rather than claiming midnight" \
  "$(printf '%s' "$got" | grep -q "All day Public holiday" && echo 1 || echo 0)" "$got"

# The one the screenshot caught: a UTC time is not the reader's clock, and
# showing it bare beside local times says it is.
say "a time written in UTC is marked as UTC" \
  "$(printf '%s' "$got" | grep -qE "16:00 Call with Lisbon, then lunch UTC|UTC" && echo 1 || echo 0)" "$got"

say "a repeating event says it repeats" \
  "$(printf '%s' "$got" | grep -q "Repeats" && echo 1 || echo 0)" "$got"

# An empty directory and an absent one are different states, and the absent one
# has to name the path rather than telling the reader to put files "somewhere".
empty="$fix/empty-home"
mkdir -p "$empty"
cat > "$fix/p-empty.js" <<'JS'
await new Promise(r => setTimeout(r, 1000));
return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 300);
JS
got=$(drive "$fix/p-empty.js" "$empty" calendar-no-files.png)
say "with no calendar directory it names the path to put files in" \
  "$(printf '%s' "$got" | grep -q "arlen/calendars" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "the file's own traps all came through the way RFC 5545 says"
exit "$fail"
