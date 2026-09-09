#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the clock with its daemon running, and again without.
#
# WHY THIS EXISTS. The clock app and the clock daemon have never been in the same
# room. The daemon has unit tests and a two-daemon drive with the calendar; the
# app has been photographed alone, where it correctly says the service is not
# running. Neither exercise answers the question the app exists for: with the
# service up, does the app see it.
#
# BOTH STATES ARE CASES HERE, and that is deliberate. "Service missing" is not an
# error state to be tolerated, it is a thing the app has to say clearly - a clock
# that silently shows an alarm list nobody is keeping is worse than one that says
# so. So this asserts the refusal when the daemon is absent AND its disappearance
# when the daemon is there.
#
# The whole run sits inside `dbus-run-session`, which is what puts the app and
# the daemon on one private bus: tauri-driver inherits this script's environment
# and the app inherits tauri-driver's, so there is no knob to pass and no
# developer session to disturb.
#
# Run: dev/screenshot/drive-clock.sh [path-to-arlen-clock-app]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/fresh.sh
. "$root/dev/screenshot/lib/fresh.sh"
app="${1:-$root/target/release/arlen-clock-app}"
clockd="$root/target/debug/arlen-clockd"
fail=0

[ -x "$app" ] || { echo "no clock app at $app"; exit 2; }
[ -x "$clockd" ] || { echo "no clock daemon at $clockd - cargo build it first"; exit 2; }
require_fresh "$clockd" "$root/daemons/clock/src" || exit 2

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

probe=$(mktemp)
cat > "$probe" <<'JS'
const tabs = [...document.querySelectorAll("[role=tab], nav button")]
  .map((b) => b.innerText.trim()).filter(Boolean).join("|");
const body = document.body.innerText.replace(/\s+/g, " ");
return `tabs=${JSON.stringify(tabs)} body=${JSON.stringify(body.slice(0, 300))}`;
JS

echo "clock:"

# Without the daemon first, in a session of its own so the developer's own clock
# is neither read nor disturbed.
run="$(mktemp -d)"
alone=$(env XDG_STATE_HOME="$run/state" XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run" HOME="$run" \
  dbus-run-session -- env SHOOT_INJECT="$probe" \
  "$here/shoot-app.sh" "$app" "$here/out/clock-no-service.png" 2>&1 | sed -n 's/^inject result: //p')

say "the five faces of the clock are all there" \
  "$(printf '%s' "$alone" | grep -q "Alarms|Timers|Focus|Stopwatch|World" && echo 1 || echo 0)" "$alone"

# The sentence that makes an empty alarm list honest. Without it the app shows
# the same nothing whether the service is down or you simply have no alarms.
say "with no service it says the alarms are not being kept" \
  "$(printf '%s' "$alone" | grep -q "clock service is not running" && echo 1 || echo 0)" "$alone"

# Now with the daemon, on one private bus.
run2="$(mktemp -d)"
withd=$(env XDG_STATE_HOME="$run2/state" XDG_DATA_HOME="$run2/data" XDG_RUNTIME_DIR="$run2" HOME="$run2" \
  dbus-run-session -- bash -c '
    "'"$clockd"'" >"'"$run2"'/clockd.log" 2>&1 &
    pid=$!
    for _ in $(seq 1 40); do
      busctl --user list 2>/dev/null | grep -q org.arlen.Clock1 && break
      sleep 0.25
    done
    SHOOT_INJECT="'"$probe"'" "'"$here"'/shoot-app.sh" "'"$app"'" "'"$here"'/out/clock.png" 2>&1
    kill "$pid" 2>/dev/null
  ' | sed -n 's/^inject result: //p')

# The point of the whole script: the refusal is gone because the thing it refused
# on behalf of is there. A sentence that stays up with the service running is a
# sentence nobody will believe when it matters.
say "with the service running it stops saying that" \
  "$(case "$withd" in ""|REFUSED:*) echo 0;; *) printf '%s' "$withd" | grep -q "clock service is not running" && echo 0 || echo 1;; esac)" "$withd"

# Paired with evidence the app rendered at all, because "the sentence is absent"
# is also true of a window that never came up.
say "and the app is still a clock rather than a blank window" \
  "$(printf '%s' "$withd" | grep -q "Alarms|Timers|Focus|Stopwatch|World" && echo 1 || echo 0)" "$withd"

# GERMAN. Every sentence in this window is one the app writes about itself - which
# service is missing, that alarms do not wake a sleeping machine, what adding one
# gets you - and that is the prose that gets written once in English. Six apps
# have had a defect only the German render showed. `dev/screenshot/README.md` has
# the recipe and the two traps; the release binary reads `locale.toml`, since the
# `?locale=` hook is compiled out.
run3="$(mktemp -d)"
mkdir -p "$run3/config/arlen"
printf '[locale]\nui = "de"\n' > "$run3/config/arlen/locale.toml"
de=$(env XDG_STATE_HOME="$run3/state" XDG_DATA_HOME="$run3/data" XDG_CONFIG_HOME="$run3/config" \
  XDG_RUNTIME_DIR="$run3" HOME="$run3" \
  dbus-run-session -- env SHOOT_INJECT="$probe" \
  "$here/shoot-app.sh" "$app" "$here/out/clock-de.png" 2>&1 | sed -n 's/^inject result: //p')

# The tab strip is the app's own vocabulary and the first thing a reader meets.
say "the five faces are named in German" \
  "$(printf '%s' "$de" | grep -qE "Wecker|Kurzzeit|Fokus|Stoppuhr|Welt" \
     && ! printf '%s' "$de" | grep -q "Stopwatch" && echo 1 || echo 0)" "$de"
# And the sentence under them. Matched on a noun rather than a verb phrase:
# German splits the verb, so "läuft nicht" is not what a correct window says.
say "and the missing service is named in German too" \
  "$(printf '%s' "$de" | grep -qE "Uhrendienst|Weckdienst|Dienst" \
     && ! printf '%s' "$de" | grep -q "clock service is not running" && echo 1 || echo 0)" "$de"

rm -rf "$run" "$run2" "$run3" "$probe"
[ "$fail" = 0 ] && echo "the clock knows whether anything is keeping its time, and says so in the reader's language"
exit "$fail"
