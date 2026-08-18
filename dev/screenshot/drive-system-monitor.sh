#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the system monitor through what `system-monitor-plan.md` names: a real
# process list with sort and kill, graphs, per-process detail.
#
# WHY A SCRIPT. The board described this app as "six commands and 115 lines of
# Rust", which counts `src-tauri` - a Tauri shim - and not the 895-line core
# behind it. A line count cannot tell a sortable table from a static one, and it
# cannot tell whether Force Quit ends anything. Pressing can.
#
# THE KILL CASE ends a process this script starts, and nothing else. It is worth
# the care: the app requires TWO presses (the first turns the label into a
# question), so a single-click probe leaves the process alive and reads exactly
# like a control that lies. That cost me a half-written finding on 18 August.
#
# Run: dev/screenshot/drive-system-monitor.sh [path-to-arlen-system-monitor]
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves the
# binary pointing at devUrl and the run reports on whatever serves that port.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-system-monitor}"
probes="$HOME/.cache/arlen-drive-sysmon"
fail=0

[ -x "$app" ] || { echo "no system-monitor binary at $app"; exit 2; }
mkdir -p "$probes" "$here/out"

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

drive() {  # drive <probe-js> <out-png>
  printf '%s' "$(SHOOT_INJECT="$1" "$here/shoot-app.sh" "$app" "$here/out/$2" 2>&1 \
    | sed -n 's/^inject result: //p')"
}

echo "system monitor:"

# The table is a [role=row] grid, not a <table>. Aiming at `tbody tr` finds
# nothing, which reads as an empty process list.
cat > "$probes/p-list.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const rows = [...document.querySelectorAll("[role=row]")]
  .filter(r => !r.querySelector("[role=columnheader]"));
return JSON.stringify({
  rows: rows.length,
  heads: [...document.querySelectorAll("[role=columnheader]")]
    .map(h => (h.textContent||"").replace(/\s+/g," ").trim().slice(0,18)),
  sample: rows.slice(0, 3).map(r => (r.textContent||"").replace(/\s+/g," ").trim().slice(0,34)),
});
JS
got=$(drive "$probes/p-list.js" sysmon.png)
say "lists real processes off this machine, with columns" \
  "$(printf '%s' "$got" | grep -qE '"rows":[1-9][0-9]+' \
     && printf '%s' "$got" | grep -q "Name" && echo 1 || echo 0)" "$got"

cat > "$probes/p-sort.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const rows = () => [...document.querySelectorAll("[role=row]")]
  .filter(r => !r.querySelector("[role=columnheader]"));
const top = () => rows().slice(0, 3).map(r => (r.textContent||"").replace(/\s+/g," ").trim().slice(0,30));
const before = top();
const mem = [...document.querySelectorAll("[role=columnheader]")]
  .find(h => /memory/i.test(h.textContent||""));
if (!mem) return "no memory column";
(mem.querySelector("button") || mem).click();
await new Promise(r => setTimeout(r, 1000));
return JSON.stringify({ before, after: top() });
JS
got=$(drive "$probes/p-sort.js" sysmon-sort.png)
say "clicking a column actually reorders the list" \
  "$(printf '%s' "$got" | grep -q '"before"' \
     && [ "$(printf '%s' "$got" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(1 if d["before"]!=d["after"] else 0)' 2>/dev/null)" = 1 ] \
     && echo 1 || echo 0)" "$got"

cat > "$probes/p-detail.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const row = [...document.querySelectorAll("[role=row]")]
  .filter(r => !r.querySelector("[role=columnheader]"))[0];
if (!row) return "no rows";
// The ROW, not the first button inside it. `row.querySelector("button")` used to
// find nothing on a flat list and fall through to the row, which is the only
// reason it worked; the moment the landing view became app-grouped the first
// button became the expander, whose handler stopPropagation()s, so the probe
// toggled a twisty and reported "no detail pane" as if selection had broken.
row.click();
await new Promise(r => setTimeout(r, 1000));
const pane = document.querySelector("[class*=detail], [class*=Detail], aside");
return pane ? (pane.textContent||"").replace(/\s+/g," ").trim().slice(0,140) : "no detail pane";
JS
got=$(drive "$probes/p-detail.js" sysmon-detail.png)
say "a row opens a per-process detail with its pid" \
  "$(printf '%s' "$got" | grep -qi "PID" && echo 1 || echo 0)" "$got"

cat > "$probes/p-perf.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const tab = [...document.querySelectorAll("button, [role=tab]")]
  .find(t => /performance/i.test(t.textContent||""));
if (!tab) return "no performance tab";
tab.click();
await new Promise(r => setTimeout(r, 1800));
return JSON.stringify({
  canvases: [...document.querySelectorAll("canvas")].map(c => [c.width, c.height]).slice(0,4),
  text: (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,120),
});
JS
got=$(drive "$probes/p-perf.js" sysmon-perf.png)
say "the performance tab draws graphs with live figures" \
  "$(printf '%s' "$got" | grep -q '"canvases":\[\[' \
     && printf '%s' "$got" | grep -qi "cpu" && echo 1 || echo 0)" "$got"

# A process of this script's own making, named distinctly enough for the app's
# filter to find exactly it.
setsid bash -c 'exec -a arlnprobe sleep 300' >/dev/null 2>&1 &
sleep 1
victim=$(pgrep -f "arlnprobe" | head -1)
if [ -z "$victim" ]; then
  say "ends the process it says it ended" 0 "could not start a target process"
else
  cat > "$probes/p-kill.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const box = document.querySelector('input[type=search], input[placeholder*="ilter"], input');
if (!box) return "no filter box";
Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set
  .call(box, "arlnprobe");
box.dispatchEvent(new Event("input", { bubbles: true }));
await new Promise(r => setTimeout(r, 1200));
const row = [...document.querySelectorAll("[role=row]")]
  .filter(r => /arlnprobe/.test(r.textContent||"") && !r.querySelector("[role=columnheader]"))[0];
if (!row) return "the filter found no row for the target";
(row.querySelector("button") || row).click();
await new Promise(r => setTimeout(r, 1000));
const quit = [...document.querySelectorAll("button")]
  .find(b => /force quit|beenden/i.test(b.textContent||""));
if (!quit) return "no Force Quit control";
// TWO presses: the first turns the label into a question and returns.
const first = quit.textContent.trim();
quit.click(); await new Promise(r => setTimeout(r, 500));
const asking = quit.textContent.trim();
quit.click(); await new Promise(r => setTimeout(r, 1500));
return JSON.stringify({ first, asking });
JS
  got=$(drive "$probes/p-kill.js" sysmon-kill.png)
  sleep 2
  gone=$(ps -p "$victim" >/dev/null 2>&1 && echo no || echo yes)
  say "force quit ends the process, and asks first" \
    "$([ "$gone" = yes ] && printf '%s' "$got" | grep -q '"asking"' && echo 1 || echo 0)" \
    "$got (target $victim gone: $gone)"
  kill "$victim" 2>/dev/null
fi

# FREEZE-THE-REFRESH, and both halves, because either alone would pass for the
# wrong implementation: an order that holds while the numbers also stop is just a
# stopped poll wearing the feature's name, and numbers that move while the order
# does too is no freeze at all. So this pins the ids AND watches one process's
# CPU change underneath them.
cat > "$probes/p-freeze.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(3000);
const rows = () => [...document.querySelectorAll("[role=row][data-pid]")];
const ids = () => rows().map(r => r.getAttribute("data-pid")).slice(0, 12);
// Read the CPU cell BY PID, so the reading follows the process rather than a
// screen position - which would be circular here.
const cpuOf = pid => {
  const r = rows().find(x => x.getAttribute("data-pid") === pid);
  const c = r && r.querySelector(".cell.num");
  return c ? c.textContent.trim() : null;
};
const grid = document.querySelector("[role=grid]");
const busiest = ids()[0];
window.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift", bubbles: true }));
await wait(600);
const frozenAttr = grid && grid.getAttribute("data-frozen");
const pinned = ids();
const samples = [];
for (let i = 0; i < 5; i++) { samples.push(cpuOf(busiest)); await wait(1500); }
const stillPinned = ids();
window.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true }));
await wait(2000);
return JSON.stringify({ frozenAttr, held: JSON.stringify(pinned) === JSON.stringify(stillPinned),
  thawedAttr: grid && grid.getAttribute("data-frozen"),
  samples, moved: new Set(samples.filter(Boolean)).size > 1 });
JS
got=$(drive "$probes/p-freeze.js" sysmon-freeze.png)
say "holding the modifier stops the rows reordering" \
  "$(printf '%s' "$got" | grep -q '"frozenAttr":"yes"' \
     && printf '%s' "$got" | grep -q '"held":true' \
     && printf '%s' "$got" | grep -q '"thawedAttr":"no"' && echo 1 || echo 0)" "$got"
say "and the figures keep arriving while it is held" \
  "$(printf '%s' "$got" | grep -q '"moved":true' && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "a process list that sorts, a detail that names a pid, graphs that draw, and a kill that asks before it acts"
exit "$fail"
