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
sleep 6
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
say "a reminder reaches the clock" \
  "$(printf '%s' "$log" | grep -qE "set=1.*reminders registered|reminders registered.*set=1" && echo 1 || echo 0)" "$log"

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
say "and only once, however often the store is re-read" \
  "$([ "$(printf '%s\n' "$log" | grep -c 'announced an upcoming meeting')" = 1 ] && echo 1 || echo 0)" "$log"

[ "$fail" = 0 ] && echo "the two daemons spoke, and an alarm exists that nobody set by hand"
exit "$fail"
