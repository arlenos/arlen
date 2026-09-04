#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the printing panel against the real CUPS on this machine.
#
# WHY THIS EXISTS. Printing was carried for a week as "five built operations no
# surface reaches". It is now eight of eight wired - `printers_list`,
# `printers_default`, `print_queue`, `printers_set_default`,
# `printers_set_options`, `printers_test_page`, `print_job_cancel`,
# `print_job_retry` - and nobody had opened the page to see what it does with
# them. A wired command and a working panel are different claims.
#
# THE ONE THING WORTH DRIVING HERE IS THE DISTINCTION. "You have no printers"
# and "I could not reach the print service" are the same empty list on screen and
# completely different facts: the first is a machine with nothing set up, the
# second is a machine whose CUPS is down while its printers sit there. This host
# runs CUPS with no destinations added, which is exactly the first case - so the
# panel must say the FIRST sentence and not the second, and a panel that says
# "not known" here is one that cannot tell them apart.
#
# Run: dev/screenshot/drive-printers.sh [path-to-arlen-settings]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-settings}"
fail=0

[ -x "$app" ] || { echo "no settings binary at $app"; exit 2; }

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

echo "printers:"

# What CUPS on this host actually reports, so the assertions below are about this
# machine rather than about a string somebody typed. `lpstat -r` answers whether
# the scheduler is running at all.
running=0
lpstat -r 2>/dev/null | grep -q "is running" && running=1
count=$(lpstat -p 2>/dev/null | grep -c "^printer ")

probe=$(mktemp)
cat > "$probe" <<'JS'
// Navigate by CLICKING, and the sidebar entries are BUTTONS rather than links -
// only the appearance sub-pages are `<a href>`. The first cut of this looked for
// an anchor, found none, and returned before navigating; two cases then passed
// against the landing page, one of them a negative assertion that was true
// because the printers page had never been opened.
const nav = [...document.querySelectorAll("button")].find((b) => b.innerText.trim() === "Printers");
if (!nav) return "no printers entry in the sidebar";
nav.click();
for (let i = 0; i < 60; i++) {
  await new Promise((r) => setTimeout(r, 100));
  if (/printer/i.test(document.body.innerText)) break;
}
// Give the CUPS read time to land: the empty state and the not-yet-read state
// look identical, and sampling the first paint would report the wrong one.
await new Promise((r) => setTimeout(r, 1500));
const body = document.body.innerText.replace(/\s+/g, " ").trim();
const rows = document.querySelectorAll("[class*=printer], .row, li").length;
// Something only the printers page says, so "we got there" is a fact rather
// than an inference from the word "printer" being in the sidebar.
const onPrinters = /default printer|test page|print queue|no printers|printers/i.test(
  document.querySelector("main")?.innerText ?? "",
);
// The main pane only: the sidebar is 200 characters of nav that pushed the
// queue section past every slice I tried, and a case that fails because the
// evidence was truncated is a case that teaches nothing.
const main = (document.querySelector("main")?.innerText ?? "").replace(/\s+/g, " ").trim();
return `onPrinters=${onPrinters} rows=${rows} main=${JSON.stringify(main.slice(0, 400))}`;
JS

work="$(mktemp -d)"
out=$(env XDG_CONFIG_HOME="$work/config" XDG_STATE_HOME="$work/state" XDG_DATA_HOME="$work/data" \
  SHOOT_INJECT="$probe" "$here/shoot-app.sh" "$app" "$here/out/printers.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# POSITIVE FIRST, and it is not decoration. The case below it is a NEGATIVE
# assertion - the panel must not say the service was unreachable - and a negative
# is also satisfied by a page that never loaded. This one has to prove we are on
# the printers page before the absence of a sentence means anything.
say "the printing panel opens" \
  "$(printf '%s' "$out" | grep -q "onPrinters=true" && echo 1 || echo 0)" "$out"

if [ "$running" = 1 ] && [ "$count" = 0 ]; then
  # THE case, and the only one this host can answer. CUPS is up and has nothing
  # in it, so the panel must say so rather than reporting the service as
  # unreachable - which is what an implementation that treats every empty answer
  # as a failed read would say, and it is not a distinction a screenshot shows.
  # THE case, and it found a real bug the first time it ran honestly. The
  # sentence is quoted from the catalogue rather than paraphrased: my first cut
  # grepped for "could not be reached" while the page says "Cannot reach the
  # print service right now", so the assertion passed against a page that was
  # both unreached and wrong.
  say "with a running print service and no printers it says you have none" \
    "$(case "$out" in ""|REFUSED:*) echo 0;; *) printf '%s' "$out" | grep -q "Cannot reach the print service" && echo 0 || echo 1;; esac)" "$out"
  say "and it says so in the words for an empty machine" \
    "$(printf '%s' "$out" | grep -q "No printers set up" && echo 1 || echo 0)" "$out"
elif [ "$running" = 0 ]; then
  # The other half, on a host where CUPS is down. Asserted the same way round so
  # the case is real wherever it runs rather than being skipped into silence.
  say "with the print service down it says it could not be reached" \
    "$(printf '%s' "$out" | grep -q "Cannot reach the print service" && echo 1 || echo 0)" "$out"
else
  echo "  --   the empty-versus-unreachable case: this host has $count printer(s), so neither empty state is the one on screen"
fi

# Evidence the panel reached CUPS at all rather than rendering its own chrome
# over a failed read: the queue section has to have answered something.
say "and it says something about the queue as well as the printers" \
  "$(printf '%s' "$out" | grep -qiE "queue|job|nothing (is )?waiting|no jobs" && echo 1 || echo 0)" "$out"

rm -rf "$work" "$probe" 2>/dev/null
[ "$fail" = 0 ] && echo "the printing panel tells an empty queue from an unreachable one"
exit "$fail"
