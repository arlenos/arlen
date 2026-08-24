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

# One event dated TODAY, written by the script rather than pinned in the fixture
# above. The agenda starts at the first day that has something on it, so without
# this the first heading is whatever date the fixture names and a reader cannot
# tell it apart from now. A hardcoded date would also make the case rot the day
# after it was written.
today=$(date +%Y%m%d)
cat > "$fix/arlen/calendars/today.ics" <<ICS
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Arlen//drive//EN
BEGIN:VEVENT
UID:today@drive
DTSTART:${today}T113000Z
DTEND:${today}T120000Z
SUMMARY:An event on the current day
END:VEVENT
END:VCALENDAR
ICS

# A RULE THE ENGINE REFUSES, dated today so it lands in the same first screen.
# `rrule` models FREQ, INTERVAL, weekly BYDAY, COUNT and UNTIL and refuses the
# rest, so this monthly-by-monthday event comes back as ONE date - and every
# later occurrence is missing from the reader's agenda. Until 21 August the only
# sign of that was a `title` attribute, which is not a statement to somebody
# reading at a glance or driving from the keyboard.
cat > "$fix/arlen/calendars/monthly.ics" <<ICS
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Arlen//drive//EN
BEGIN:VEVENT
UID:rent@drive
DTSTART:${today}T090000Z
SUMMARY:A rule this calendar does not model
RRULE:FREQ=MONTHLY;BYMONTHDAY=1,15
END:VEVENT
END:VCALENDAR
ICS

# A WEEK SOMEBODY CALLED OFF. The rule expands correctly and the file then says
# "not that one" with an EXDATE; until 21 August that line was dropped on the
# floor and the agenda showed a meeting that had been cancelled. Dated from today
# so both the kept and the excluded week are inside the window.
skip=$(date -d "+7 days" +%Y%m%d 2>/dev/null || date -v+7d +%Y%m%d)
keep=$(date -d "+14 days" +%Y%m%d 2>/dev/null || date -v+14d +%Y%m%d)
cat > "$fix/arlen/calendars/cancelled.ics" <<ICS
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Arlen//drive//EN
BEGIN:VEVENT
UID:offweek@drive
DTSTART:${today}T150000Z
RRULE:FREQ=WEEKLY;COUNT=3
EXDATE:${skip}T150000Z
SUMMARY:A meeting with one week called off
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

# A wide slice on purpose. The window is short and the events are many, and the
# probe reads the whole text rather than a screenful: when a sentence was added
# above the list, a 400-character slice silently cut the last three events off
# and three cases failed for a reason that had nothing to do with them.
cat > "$fix/p-agenda.js" <<'JS'
await new Promise(r => setTimeout(r, 1200));
// The client defaults to the week grid now; these cases assert the agenda's
// sentences, so the probe walks over to it first ("Agenda" in both languages).
const seg = document.getElementById("cal-view");
const b = seg && [...seg.querySelectorAll("button")].find(x => /Agenda/.test(x.textContent || ""));
if (b) { b.click(); await new Promise(r => setTimeout(r, 500)); }

return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 1200);
JS
got=$(drive "$fix/p-agenda.js" "$fix" calendar-agenda.png)

# The day the reader is standing on is marked as such.
# Case-insensitive, and the fixture's own title deliberately avoids the word:
# the marker is styled uppercase, so a case-sensitive check would be asserting a
# CSS rule, and a title containing "today" would pass without any marker at all.
say "today is marked as today" \
  "$(printf '%s' "$got" | grep -q "An event on the current day" \
     && printf '%s' "$got" | grep -qi "today" && echo 1 || echo 0)" "$got"

# The caveat is IN THE ROW. The core has always known this row was not worked
# out (`expanded: false`); the window kept it in a tooltip.
say "a repetition the calendar cannot work out says so in the row" \
  "$(printf '%s' "$got" | grep -q "A rule this calendar does not model" \
     && printf '%s' "$got" | grep -q "only this date" && echo 1 || echo 0)" "$got"

# Its own probe, with no slice. The count below is over the WHOLE agenda and the
# 1200-character reader above already cut three cases off once; asserting a count
# against a truncated string measures the slice, not the app.
cat > "$fix/p-all.js" <<'JS'
await new Promise(r => setTimeout(r, 1200));
// The client defaults to the week grid now; these cases assert the agenda's
// sentences, so the probe walks over to it first ("Agenda" in both languages).
const seg = document.getElementById("cal-view");
const b = seg && [...seg.querySelectorAll("button")].find(x => /Agenda/.test(x.textContent || ""));
if (b) { b.click(); await new Promise(r => setTimeout(r, 500)); }

return (document.body.innerText || "").replace(/\s+/g, " ").trim();
JS
all=$(drive "$fix/p-all.js" "$fix" calendar-exdate.png)

# Three weekly occurrences, one called off: the title must appear exactly twice.
say "a week the file calls off is not on the agenda" \
  "$(printf '%s' "$all" | grep -o "A meeting with one week called off" | wc -l | grep -qx 2 \
     && echo 1 || echo 0)" "$(printf '%s' "$all" | grep -o "A meeting with one week called off" | wc -l) occurrence(s)"

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

# Nothing starts the calendar daemon here, and that is the point: the app reads
# the files itself, shows the same agenda, and has to SAY that no reminders are
# being set. An agenda that looked identical either way would let somebody
# believe their reminders were armed when nothing was arming them.
say "with no service running it says the reminders are not being set" \
  "$(printf '%s' "$got" | grep -q "no reminders are being set" && echo 1 || echo 0)" "$got"

say "a repeating event says it repeats" \
  "$(printf '%s' "$got" | grep -q "Repeats" && echo 1 || echo 0)" "$got"

# An empty directory and an absent one are different states, and the absent one
# has to name the path rather than telling the reader to put files "somewhere".
empty="$fix/empty-home"
mkdir -p "$empty"
cat > "$fix/p-empty.js" <<'JS'
await new Promise(r => setTimeout(r, 1000));
// The client defaults to the week grid now; these cases assert the agenda's
// sentences, so the probe walks over to it first ("Agenda" in both languages).
const seg = document.getElementById("cal-view");
const b = seg && [...seg.querySelectorAll("button")].find(x => /Agenda/.test(x.textContent || ""));
if (b) { b.click(); await new Promise(r => setTimeout(r, 500)); }

return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 700);
JS
got=$(drive "$fix/p-empty.js" "$empty" calendar-no-files.png)
# Opening the app CREATES the directory (the watcher cannot watch a path that is
# not there), so what a first-run reader meets is an empty one - and it still has
# to say where to put files.
say "with nothing to show it names the path to put files in" \
  "$(printf '%s' "$got" | grep -q "arlen/calendars" && echo 1 || echo 0)" "$got"

# A file written WHILE the window is open. Without the watcher the agenda is
# whatever the directory held at mount, and a calendar showing yesterday's answer
# with no sign it is stale is the quiet kind of wrong. The file lands two seconds
# in, from a background shell, because the probe cannot touch the filesystem.
cat > "$fix/p-live.js" <<'JS'
await new Promise(r => setTimeout(r, 5000));
// The client defaults to the week grid now; these cases assert the agenda's
// sentences, so the probe walks over to it first ("Agenda" in both languages).
const seg = document.getElementById("cal-view");
const b = seg && [...seg.querySelectorAll("button")].find(x => /Agenda/.test(x.textContent || ""));
if (b) { b.click(); await new Promise(r => setTimeout(r, 500)); }

return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 1200);
JS
( sleep 2; cat > "$fix/arlen/calendars/added.ics" <<'ICS'
BEGIN:VCALENDAR
BEGIN:VEVENT
UID:added@arlen
SUMMARY:Added while the window was open
DTSTART;TZID=Europe/Vienna:20260819T170000
END:VEVENT
END:VCALENDAR
ICS
) &
got=$(drive "$fix/p-live.js" "$fix" calendar-live.png)
wait
say "a file written while the window is open appears without a restart" \
  "$(printf '%s' "$got" | grep -q "Added while the window was open" && echo 1 || echo 0)" "$got"

# Opened ON a file, the way a double-click in the file manager arrives. That file
# is the whole agenda: mixing it with the directory would bury what the person
# actually opened.
cat > "$fix/second.ics" <<'ICS'
BEGIN:VCALENDAR
BEGIN:VEVENT
UID:only@arlen
SUMMARY:The only event in this file
DTSTART;TZID=Europe/Vienna:20260819T110000
END:VEVENT
END:VCALENDAR
ICS
got=$(XDG_DATA_HOME="$fix" SHOOT_APP_ARGS="$fix/second.ics" SHOOT_INJECT="$fix/p-agenda.js" \
  "$here/shoot-app.sh" "$app" "$here/out/calendar-one-file.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "opened on a file, it shows that file and not the whole directory" \
  "$(printf '%s' "$got" | grep -q "The only event in this file" \
     && ! printf '%s' "$got" | grep -q "Morning standup" && echo 1 || echo 0)" "$got"

# KEEPING IT. Opening a file reads it where it lies, deliberately - so until
# there was a way to say "keep this one", the calendar directory was empty on
# every machine, the agenda was empty for everyone, and the reminder daemon had
# nothing to watch. That is why neither the app nor the daemon was on the image.
# This is the way in, so it is driven: press the button, and the file must be in
# the directory afterwards AND the view must switch from the-file to the-folder.
cat > "$fix/p-keep.js" <<'JS'
for (let i = 0; i < 40; i++) {
  await new Promise((r) => setTimeout(r, 100));
  if (document.querySelector(".keep button")) break;
}
const btn = document.querySelector(".keep button");
if (!btn) return "no keep button";
btn.click();
// The button going away IS the state change: keeping it drops the launched file
// and re-reads the directory, so a view still offering to keep is one that did
// not switch.
for (let i = 0; i < 50; i++) {
  await new Promise((r) => setTimeout(r, 100));
  if (!document.querySelector(".keep button")) break;
}
// Folder mode lands on the week grid; the kept event's day may be outside it,
// so the assertion reads the agenda ("Agenda" in both languages).
const seg = document.getElementById("cal-view");
const b = seg && [...seg.querySelectorAll("button")].find(x => /Agenda/.test(x.textContent || ""));
if (b) { b.click(); await new Promise(r => setTimeout(r, 500)); }
return `stillOffering=${document.querySelector(".keep button") ? 1 : 0} `
  + `body=${JSON.stringify(document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 700))}`;
JS
keepdir="$fix/keep"
rm -rf "$keepdir" && mkdir -p "$keepdir"
cat > "$keepdir/invitation.ics" <<'ICS'
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//arlen//drive//EN
BEGIN:VEVENT
UID:kept@drive
DTSTAMP:20260820T050000Z
DTSTART:20260821T090000Z
DTEND:20260821T100000Z
SUMMARY:An invitation somebody opened
END:VEVENT
END:VCALENDAR
ICS
got=$(XDG_DATA_HOME="$keepdir/data" SHOOT_APP_ARGS="$keepdir/invitation.ics" SHOOT_INJECT="$fix/p-keep.js" \
  "$here/shoot-app.sh" "$app" "$here/out/calendar-keep.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# The file is on the machine now. This is the assertion the image steps rest on:
# without it, staging the daemon is staging a process that watches an empty
# folder forever.
say "keeping an opened invitation puts it in the calendar directory" \
  "$([ -f "$keepdir/data/arlen/calendars/invitation.ics" ] && echo 1 || echo 0)" \
  "$(ls -la "$keepdir/data/arlen/calendars/" 2>&1 | tail -3) || $got"

say "and the view stops being about that one file" \
  "$(printf '%s' "$got" | grep -q "stillOffering=0" && echo 1 || echo 0)" "$got"

# The event survived the move: a copy that lands but does not parse is the same
# empty calendar with more steps.
say "and the event is still there, read from the folder" \
  "$(printf '%s' "$got" | grep -q "An invitation somebody opened" && echo 1 || echo 0)" "$got"

# THE OTHER DIRECTION IS NOT DRIVEN HERE, and the reason is the harness rather
# than the app. With the daemon started on a private bus (`dbus-run-session`),
# the app still reported the service as absent: tauri-driver launches the binary
# itself, and the bus address this script exports does not reach it. So the
# service-up path is proved at the wire instead - `dev/scripts/drive-calendar-clock.sh`
# starts both daemons and reads the alarm out of the clock's own state - and what
# is not yet proved in pixels is that the sentence below DISAPPEARS when the
# service answers. Said here rather than left as a missing case.

# German. Six other apps had a defect that only the German render showed - a
# column sized to an English word, a heading that never adopted the catalogue -
# so this is a case rather than something someone remembers to look at. The
# release binary takes its language from `locale.toml`, not from a URL: the
# `?locale=` hook is compiled out of a production build.
cfg="$fix/config-de"
mkdir -p "$cfg/arlen"
printf '[locale]\nui = "de"\n' > "$cfg/arlen/locale.toml"
got=$(XDG_DATA_HOME="$fix" XDG_CONFIG_HOME="$cfg" SHOOT_INJECT="$fix/p-agenda.js" \
  "$here/shoot-app.sh" "$app" "$here/out/calendar-agenda-de.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "and the caveat is German too" \
  "$(printf '%s' "$got" | grep -q "nur dieses Datum" && echo 1 || echo 0)" "$got"

say "the German build says the German words, dates included" \
  "$(printf '%s' "$got" | grep -q "Mittwoch, 19. August" \
     && printf '%s' "$got" | grep -q "Ganztägig" \
     && printf '%s' "$got" | grep -q "Jede Woche" \
     && printf '%s' "$got" | grep -q "Mo, Di, Mi, Do, Fr" && echo 1 || echo 0)" "$got"
# The titles come from the FILE and stay as written: translating someone's own
# event would be a worse bug than leaving it.
say "and leaves the events' own titles alone" \
  "$(printf '%s' "$got" | grep -q "Morning standup" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "the file's own traps all came through the way RFC 5545 says"
exit "$fail"
