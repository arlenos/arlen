#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run the calendar daemon against a real clock daemon and read what happened.
#
# WHY THIS EXISTS. `calendar-app.md` section 4 spans two processes: the calendar
# derives a trigger, the clock arms it, and neither half proves the other. Every
# piece under this is unit-tested - the parse, the recurrence, the derivation,
# the plan, the reach gate - and all of them passing is compatible with the two
# daemons never speaking, which is exactly what happened the first time
# (`Clock1` admitted only the clock app, so every registration would have been
# refused in silence).
#
# So this starts both on a private bus with one calendar file and asks whether an
# alarm arrived. It is the one question the unit tests cannot answer.
#
# Run: dev/scripts/drive-calendar-clock.sh
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
fail=0

# DEBUG binaries, and they have to be CURRENT. The first run of the announcement
# case failed against a daemon built an hour earlier: the code was right, the
# binary predated it, and the output looked exactly like a feature that does not
# work. `cargo build` the daemon you changed before reading anything here.
clock="$root/target/debug/arlen-clockd"
cal="$root/target/debug/arlen-calendard"
bus="$root/target/debug/event-bus"
for bin in "$clock" "$cal" "$bus"; do
  [ -x "$bin" ] || { echo "no binary at $bin - cargo build the daemon first"; exit 2; }
done

# A DEBUG build deliberately: the dev app ids (`dev.arlen-calendard`) resolve
# only there, which is what lets a build-tree binary reach a daemon a release
# build would refuse.
run="$(mktemp -d)"
trap 'rm -rf "$run"' EXIT
mkdir -p "$run/data/arlen/calendars" "$run/state" "$run/config"

# A second meeting a few minutes out, so it falls inside the announcement lead
# while the one above stays a day away. Two windows, two different promises: the
# clock gets tomorrow's reminder, the bus hears about the one starting now.
soon_date=$(date -u -d "+4 minutes" +%Y%m%d)
soon_time=$(date -u -d "+4 minutes" +%H%M%S)

# And a third whose alarm comes due WHILE THIS RUNS, so the last link in the
# chain is exercised rather than inferred. Everything above proves a reminder
# was derived, handed over and stored; none of it proves the clock ever reaches
# the moment and rings.
#
# SEVENTY-FIVE SECONDS, AND NOT TWELVE, WHICH IS WHAT I TRIED FIRST. A reminder
# is a wall-clock time to the minute: the calendar registers `%H:%M` and the
# clock resolves that to `HH:MM:00`, then refuses a dated alarm whose moment has
# passed - correctly, since ringing for a meeting that already happened is
# worse than silence. So an alarm twelve seconds out is armed for the START of
# the minute it is already inside, which is behind `now` the instant it lands,
# and it can never fire. The case failed, and it was the fixture that was wrong.
#
# Truncation to the minute costs at most 59 seconds, so 75 leaves the armed
# moment at least 16 seconds ahead of the registration no matter which second
# the run starts on. That is what makes the window below as wide as it is.
ring_date=$(date -u -d "+75 seconds" +%Y%m%d)
ring_time=$(date -u -d "+75 seconds" +%H%M%S)

# One event, one alarm, tomorrow, so the trigger is always ahead of now.
tomorrow=$(date -u -d "+1 day" +%Y%m%d)
cat > "$run/data/arlen/calendars/work.ics" <<ICS
BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:standup@drive
SUMMARY:Morning standup
DTSTART:${tomorrow}T090000Z
DTEND:${tomorrow}T091500Z
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT15M
END:VALARM
END:VEVENT
BEGIN:VEVENT
UID:soon@drive
SUMMARY:Starting shortly
LOCATION:Room 3
DTSTART:${soon_date}T${soon_time}Z
END:VEVENT
BEGIN:VEVENT
UID:imminent@drive
SUMMARY:Rings during this run
DTSTART:${ring_date}T${ring_time}Z
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT0S
END:VALARM
END:VEVENT
END:VCALENDAR
ICS

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

echo "calendar and clock:"

# Both daemons under one private session bus, with every path pointed at the
# temp root so a run cannot touch the developer's own alarms or calendars.
cat > "$run/run.sh" <<'SH'
set -u
# The bus first: the calendar announces on its first pass, and a bus that is not
# up yet would turn a real announcement into a warning about a missing bus.
ARLEN_RUNTIME_DIR="$RUN" XDG_RUNTIME_DIR="$RUN" "$BUS" >"$RUN/bus.log" 2>&1 &
bus_pid=$!
sleep 1
"$CLOCKD" >"$RUN/clock.log" 2>&1 &
clock_pid=$!
sleep 2
"$CALD" >"$RUN/calendar.log" 2>&1 &
cal_pid=$!
# Long enough for the imminent alarm to come due inside the run. The rest of
# the assertions are satisfied within a second or two; this one is the reason
# the window is this wide, and why this script takes a minute and a half rather
# than ten seconds. It is a script somebody runs, not a per-commit gate.
sleep 100
kill "$cal_pid" "$clock_pid" "$bus_pid" 2>/dev/null
wait 2>/dev/null
SH
CLOCKD="$clock" CALD="$cal" BUS="$bus" RUN="$run" \
  HOME="$run" XDG_DATA_HOME="$run/data" XDG_STATE_HOME="$run/state" \
  XDG_CONFIG_HOME="$run/config" XDG_RUNTIME_DIR="$run" RUST_LOG=info \
  dbus-run-session -- bash "$run/run.sh" >/dev/null 2>&1

# Stripped of terminal formatting first: tracing writes its fields with ANSI
# escapes between the name and the value, so a plain `set=1` never matches even
# when the line says exactly that. The first run of this script failed on
# nothing but colour.
strip_ansi() { sed -e 's/\x1b\[[0-9;]*m//g'; }
log=$(strip_ansi < "$run/calendar.log" 2>/dev/null)
# Whitespace squeezed out: the clock pretty-prints its store, so a pattern
# written as it appears in the type does not match the file.
state=$(tr -d ' \n' < "$run/state/arlen/clock/state.json" 2>/dev/null)

say "the calendar daemon takes its bus name" \
  "$(printf '%s' "$log" | grep -q "serving org.arlen.Calendar1" && echo 1 || echo 0)" "$log"

# The whole point. Anything less than this - a refused call, a clock that never
# came up, a derivation that found nothing - shows up as a missing line.
# Any number above zero, not exactly one: the fixture has grown a second alarm
# since this was written, and a case that counts what the fixture happens to
# hold is a case that goes red when the fixture is edited rather than when the
# daemons stop talking.
say "a reminder reaches the clock" \
  "$(printf '%s' "$log" | grep -qE "set=[1-9].*reminders registered|reminders registered.*set=[1-9]" && echo 1 || echo 0)" "$log"

# The failure this drive was written after: the clock refusing the calendar
# outright. Named separately so the reason is in the output rather than inferred
# from an absence.
say "the clock did not refuse it" \
  "$(printf '%s' "$log" | grep -q "could not register reminders" && echo 0 || echo 1)" "$log"

# The clock's OWN store, not the calendar's word for it. The calendar logging a
# successful call and the clock holding the alarm are different claims, and the
# second is the one that makes an alarm ring.
say "the clock wrote the alarm into its own state" \
  "$(printf '%s' "$state" | grep -q '"id":"calendar:' && echo 1 || echo 0)" "$state"

# What makes it removable by the calendar and nobody else, and what marks it as
# an alarm no person set.
say "the alarm carries the mark of who registered it" \
  "$(printf '%s' "$state" | grep -q 'source.*calendar' && echo 1 || echo 0)" "$state"

# The field Clock1 could not express before: without it the reminder would be
# armed for the next matching wall-clock time rather than the meeting's day.
say "the alarm belongs to a day rather than to a time of day" \
  "$(printf '%s' "$state" | grep -q '"on_date":"20' && echo 1 || echo 0)" "$state"

# The other promise. `meeting-prep` has triggered on this event since it was
# written and nothing ever emitted it, so this line is the difference between a
# behaviour that exists and one that can fire. The daemon says it only after the
# write to the bus returned, so a bus that refused the connection shows up as a
# warning instead.
say "a meeting about to start is announced on the bus" \
  "$(printf '%s' "$log" | grep -q "announced an upcoming meeting" && echo 1 || echo 0)" "$log"

# And it is said once. The store is re-read on a timer; without the memory this
# one meeting would wake the agent at every pass.
#
# Counted PER MEETING, by uid, rather than over the whole log. The promise is
# that one meeting is announced once, not that the daemon speaks once - and with
# the window now wide enough for the imminent alarm, a second meeting enters the
# lead window and is rightly announced too. Counting every line would read that
# correct behaviour as a repeat.
say "and only once, however often the store is re-read" \
  "$([ "$(printf '%s\n' "$log" | grep -c 'uid=soon@drive')" = 1 ] && echo 1 || echo 0)" "$log"

# THE LAST LINK. Everything above proves a reminder was derived, handed over,
# stored and marked; none of it proves the clock ever reaches the moment. An
# alarm armed for a time that arrives and passes in silence is the whole feature
# failing at the last step, and it would have read as eight green cases.
#
# Read from the CLOCK's own log, not the calendar's: the calendar's part ended
# when the registration returned.
clocklog=$(strip_ansi < "$run/clock.log" 2>/dev/null)
say "the alarm comes due and the clock rings it" \
  "$(printf '%s' "$clocklog" | grep -q "announcing" && echo 1 || echo 0)" "$clocklog"

# And it rings the right one. `announcing` alone would also hold for the
# stopwatch, a timer, or a focus session ending - anything the clock says out
# loud - so the meeting's own title has to be in what it announced.
say "and what it rings carries the meeting's own title" \
  "$(printf '%s' "$clocklog" | grep -q "Rings during this run" && echo 1 || echo 0)" "$clocklog"

[ "$fail" = 0 ] && echo "the two daemons spoke, and an alarm exists that nobody set by hand"
exit "$fail"
