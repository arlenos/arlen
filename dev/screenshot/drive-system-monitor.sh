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
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const rows = () => [...document.querySelectorAll("[role=row]")]
  .filter(r => !r.querySelector("[role=columnheader]"));
const top = () => rows().slice(0, 3).map(r => (r.textContent||"").replace(/\s+/g," ").trim().slice(0,30));
// Wait for a MEASURED CPU column before capturing the order. CPU is a delta, so
// the first poll of a run has none and every row reads a dash; with every value
// tied the default CPU sort can coincidentally equal the memory sort, and this
// probe then reports "clicking did nothing" about a table with nothing to sort.
// That is how it went red on 18 August, after another probe left the refresh
// rate at 10s and the second sample had not arrived.
for (let i = 0; i < 40; i++) {
  const cpu = rows()[0]?.querySelectorAll(".cell.num")[0]?.textContent?.trim();
  if (cpu && cpu !== "-") break;
  await wait(500);
}
const before = top();
const mem = [...document.querySelectorAll("[role=columnheader]")]
  .find(h => /memory/i.test(h.textContent||""));
if (!mem) return "no memory column";
(mem.querySelector("button") || mem).click();
await wait(1000);
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
# The other half - that the DATA keeps arriving while the order is held - is
# REPORTED here and asserted in `freeze.test.ts` instead, deliberately.
#
# As an assertion it was flaky by construction: it required one process's CPU to
# CHANGE within about seven seconds, which is a claim about how busy this machine
# happens to be, not about the code. It read 7.3/7.6/10.0/7.2 on one run and
# 6.2 five times on the next, and the second is not a defect - it is a quiet
# laptop. Asserting it would have made this driver fail for the weather.
#
# The unit test settles it properly and deterministically: `pinnedOrder` returns
# the SAME OBJECT it was handed, so the row on screen is this poll's row rather
# than a copy taken when the key went down. Identity, not hope.
printf '%s\n' "       (data during the freeze: $(printf '%s' "$got" | sed -n 's/.*"samples":\[\([^]]*\)\].*/\1/p'))"

# The filter box, which existed and had never been pressed. Names come from each
# row's `aria-label` and NOT from the name cell: the cell also carries the icon
# badge letter, so its textContent reads "c claude" - a string the interface
# never shows anyone, and typing it matches nothing. The first cut of this probe
# did exactly that and reported the filter as broken.
#
# The child-name clause (a tab title surfacing its browser row) is NOT checked
# here and cannot be: this machine's children carry their parent's name, so the
# two cases are the same string on screen. `freeze.test.ts` builds that case.
cat > "$probes/p-filter.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(3000);
const names = () => [...document.querySelectorAll("[role=row][data-pid]")]
  .map(r => r.getAttribute("aria-label")).filter(Boolean);
const before = names().length;
const withKids = [...document.querySelectorAll("[role=row][data-pid]")]
  .find(r => r.querySelector("button.twist"));
const parentName = withKids ? withKids.getAttribute("aria-label") : names()[0];
const box = [...document.querySelectorAll("input")].find(i => i.offsetParent !== null);
if (!box) return JSON.stringify({ error: "no filter box" });
const type = async (v) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set.call(box, v);
  box.dispatchEvent(new Event("input", { bubbles: true }));
  await wait(700);
};
await type("zzzznomatch");
const noneCount = names().length;
await type(parentName);
const hit = names();
await type("");
const after = names();
return JSON.stringify({ before, parentName, noneCount,
  hitCount: hit.length, hitHasIt: hit.some(n => n.includes(parentName)),
  narrowed: hit.length < before,
  // "Restored" cannot be an exact count: this runs over ten seconds of a live
  // machine and the driver itself starts and reaps processes, so `after ===
  // before` fails whenever anything on the box exits. It went red on 18 August
  // for that reason and the filter was fine. What the clause MEANS is that
  // clearing the box brings back rows the filter had hidden, which is checked
  // by a row that did NOT match the query being present again.
  restored: after.length > hit.length && after.some(n => !n.includes(parentName)),
  afterCount: after.length });
JS
got=$(drive "$probes/p-filter.js" sysmon-filter.png)
say "typing in the filter narrows the list to what matches" \
  "$(printf '%s' "$got" | grep -q '"noneCount":0' \
     && printf '%s' "$got" | grep -q '"hitHasIt":true' \
     && printf '%s' "$got" | grep -q '"narrowed":true' \
     && printf '%s' "$got" | grep -q '"restored":true' && echo 1 || echo 0)" "$got"

# The memory-pressure meter, which needs the Performance tab AND its Memory
# device - it lives behind two clicks, which is why it went unlooked-at.
#
# Matching on the whole line rather than a fragment: the first cut of this probe
# used `[^.]*pressure[^.]*` and the regex cut at the decimal point in "36.2", so
# it reported "0 GB in use" and looked like a broken figure. The app was right.
cat > "$probes/p-pressure.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const tab = [...document.querySelectorAll("button")]
  .find(b => /performance|leistung/i.test(b.textContent || ""));
if (!tab) return JSON.stringify({ error: "no performance tab" });
tab.click();
await wait(2500);
const mem = [...document.querySelectorAll("button,li,[role=option],[role=tab]")]
  .find(e => /^\s*(memory|arbeitsspeicher|speicher)\b/i.test((e.textContent || "").trim()));
if (mem) { mem.click(); await wait(2000); }
const text = (document.body.innerText || "").replace(/\s+/g, " ").trim();
const line = (text.match(/[0-9][^-]*GB in use[^|]{0,80}/) || [null])[0];
return JSON.stringify({ line: line ? line.slice(0, 110) : null,
  hasMeter: /Memory pressure: (none|some waiting|thrashing|not measured)/.test(text) });
JS
got=$(drive "$probes/p-pressure.js" sysmon-pressure.png)
say "the memory pane says how full it is AND whether anything is waiting on it" \
  "$(printf '%s' "$got" | grep -q '"hasMeter":true' \
     && printf '%s' "$got" | grep -q 'GB in use' && echo 1 || echo 0)" "$got"

# The CPU pane's load line. Worth a case of its own because of HOW it was broken:
# `LoadAverage` shipped `per_core` while the frontend read `perCore`, since serde
# applies `rename_all` per struct and a nested one does not inherit it. The types
# both said `perCore` and svelte-check was happy - TypeScript cannot check what a
# Rust process actually puts on the wire. The line simply did not appear, and
# nothing but pressing the app said so.
cat > "$probes/p-load.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const tab = [...document.querySelectorAll("button")]
  .find(b => /performance|leistung/i.test(b.textContent || ""));
if (!tab) return JSON.stringify({ error: "no performance tab" });
tab.click();
await wait(3500);
const text = (document.body.innerText || "").replace(/\s+/g, " ").trim();
// Anchored on the two ENDS rather than on what sits between them: the clock and
// the temperature landed in that gap later, and a regex that spelled out the
// middle went null the moment they did - reporting a working line as missing.
const m = text.match(/(logical processors|logische Prozessoren)[\s\S]{0,90}?(per core|pro Kern)\)/);
return JSON.stringify({ line: m ? m[0].slice(0, 130) : null,
  hasPerCore: /per core|pro Kern/.test(text),
  hasClock: /MHz/.test(text),
  // The sensor label travels with the figure, so a bare number cannot pass as a
  // die temperature. Reported, not asserted: plenty of machines have no sensor.
  temp: (text.match(/(Tctl|Tdie|Package[^,]*|temp\d)\s*[\d.,]+\s*°C/) || [null])[0] });
JS
got=$(drive "$probes/p-load.js" sysmon-load.png)
say "the CPU pane gives the load against this machine's core count" \
  "$(printf '%s' "$got" | grep -q '"hasPerCore":true' \
     && printf '%s' "$got" | grep -q '"line":"' && echo 1 || echo 0)" "$got"

# THE GUARDRAIL. Stopping a daemon warns first (plan (d)1). The target is chosen
# with care: this probe CLICKS Stop, so if the guardrail were broken the click
# would land as a real stop. `systemd` (pid 1) cannot be killed by this user, so
# a broken guardrail here produces a refused kill and a red probe rather than a
# dead session. Never a live daemon that WOULD die.
cat > "$probes/p-guardrail.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
// [role=row], not `tbody tr` - the warning at the top of this file, which I
// then walked straight into: `tbody tr` finds nothing and an empty list looks
// exactly like a machine with no systemd on it.
const rows = [...document.querySelectorAll("[role=row]")]
  .filter(r => !r.querySelector("[role=columnheader]"));
const target = rows.find(r => /(^|\s)systemd(\s|$)/i.test(
  (r.querySelector(".cell.name")?.textContent || "").trim()));
if (!target) return JSON.stringify({ skipped: "no systemd row on this machine" });
target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 200, clientY: 200 }));
await wait(400);
const menu = document.querySelector('[role="menu"]');
if (!menu) return JSON.stringify({ error: "no row menu" });
const items = [...menu.querySelectorAll('[role="menuitem"]')];
// The plain Stop item, not Force Quit, matched on the whole label so the German
// "Beenden" inside "Sofort beenden" cannot pick the wrong one.
const stop = items.find(b => /^(stop|beenden)$/i.test((b.textContent || "").trim()));
if (!stop) return JSON.stringify({ error: "no stop item", items: items.map(b => b.textContent.trim()) });
const before = stop.textContent.trim();
stop.click();
await wait(300);
// The SAME element, re-read. The first cut asked for `[role=menuitem].danger`,
// which Force Quit already is - so it reported "armed" on a label that had not
// changed at all, and passed for a reason unrelated to the guardrail.
const after = stop.textContent.trim();
// Read before dismissing: the menu staying open IS the behaviour (the warning
// must be readable), and Escape would make that unmeasurable.
const stillOpen = document.body.contains(menu);
// Escape rather than a second click: the point is that the first press did NOT
// act, and pressing again would be asking pid 1 to exit.
document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
return JSON.stringify({ before, after, stillOpen,
  // Named the consequence, not merely different: "Stop?" would also be longer.
  armed: /system service|Systemdienst/.test(after), namesIt: /systemd/.test(after) });
JS
got=$(drive "$probes/p-guardrail.js" sysmon-guardrail.png)
if printf '%s' "$got" | grep -q '"skipped"'; then
  echo "  --   the daemon guardrail (no systemd row here): $got"
else
  say "stopping a system service asks before it acts" \
    "$(printf '%s' "$got" | grep -q '"armed":true' \
       && printf '%s' "$got" | grep -q '"stillOpen":true' && echo 1 || echo 0)" "$got"
fi


# WHAT A PROCESS HOLDS OPEN, and the two answers that must stay apart. The pane
# used to invent this: three paths built from the process name, plus a hardcoded
# `tcp 140.82.121.4:443 ESTABLISHED` (a real GitHub address) for anything with
# traffic. Both cases below are needed - a reader that returns an empty list for
# a process it could not read looks identical to a working one on our own rows.
cat > "$probes/p-openfiles.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const rows = [...document.querySelectorAll("[role=row][data-pid]")];
const name = r => (r.querySelector(".cell.name")?.textContent || "").trim();
const read = async (row) => {
  row.click();
  await wait(1200);
  const pane = document.querySelector("aside");
  [...pane.querySelectorAll("button")].find(b => /open files|dateien/i.test(b.textContent || ""))?.click();
  await wait(1500);
  // `.files` itself, not the pane: the pane's footer sits OUTSIDE the tab body
  // and reads the same on every tab, so a tail-of-innerText check reports the
  // footer no matter which tab is showing.
  return (pane.querySelector(".files")?.innerText || "NO FILES SECTION").replace(/\s+/g, " ").trim();
};
// A process of ours (the app's own webview) and one that is not (pid 1, root).
const mine = rows.find(r => /WebKitWebProcess|arlen-system-monitor/.test(name(r)));
const theirs = rows.find(r => /^systemd$/.test(name(r)));
if (!mine || !theirs) return JSON.stringify({ skipped: "needed both an own and a foreign process" });
const ours = await read(mine);
const foreign = await read(theirs);
return JSON.stringify({
  // Real fds look like real paths, and the invented address is gone.
  realPaths: /\/(dev|proc|memfd|usr|home|tmp)/.test(ours),
  invented: /140\.82\.121\.4/.test(ours),
  // The foreign one must say it could not look - NOT "no open files", which
  // would be a false all-clear on the screen about what programs can reach.
  saysUnmeasured: /not measured/i.test(foreign),
  claimsEmpty: /no open files|keine offenen/i.test(foreign),
  ours: ours.slice(0, 120), foreign: foreign.slice(0, 90) });
JS
got=$(drive "$probes/p-openfiles.js" sysmon-openfiles.png)
if printf '%s' "$got" | grep -q '"skipped"'; then
  echo "  --   what a process holds open: $got"
else
  say "the open-files pane reads real fds, and says so when it cannot" \
    "$(printf '%s' "$got" | grep -q '"realPaths":true' \
       && printf '%s' "$got" | grep -q '"invented":false' \
       && printf '%s' "$got" | grep -q '"saysUnmeasured":true' \
       && printf '%s' "$got" | grep -q '"claimsEmpty":false' && echo 1 || echo 0)" "$got"
fi


# THE STATISTICS AND MEMORY TABS, which were computed from the row: threads as
# memory divided by 40, ppid as `1200 + pid % 40`, context switches as
# `1000 + pid * 137`. Numbers with the shape of measurements.
cat > "$probes/p-stats.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const rows = [...document.querySelectorAll("[role=row][data-pid]")];
const name = r => (r.querySelector(".cell.name")?.textContent || "").trim();
const read = async (row) => {
  row.click();
  await wait(1200);
  const pane = document.querySelector("aside");
  const tab = t => [...pane.querySelectorAll("button")].find(b => b.textContent.trim() === t);
  tab("Statistics")?.click(); await wait(1200);
  const stats = [...pane.querySelectorAll(".stat")].map(d => d.innerText.replace(/\s+/g, " ").trim());
  tab("Memory")?.click(); await wait(800);
  const mem = [...pane.querySelectorAll(".stat")].map(d => d.innerText.replace(/\s+/g, " ").trim());
  return { stats, mem };
};
const mine = rows.find(r => /WebKitWebProcess|arlen-system-monitor/.test(name(r)));
const theirs = rows.find(r => /^systemd$/.test(name(r)));
if (!mine || !theirs) return JSON.stringify({ skipped: "needed both an own and a foreign process" });
const ours = await read(mine);
const foreign = await read(theirs);
const num = (rows, label) => {
  const line = rows.find(r => r.startsWith(label));
  return line ? line.slice(label.length).replace(/[^\d.-]/g, "") : "";
};
return JSON.stringify({
  // A webview has many threads and pid 1 has one; a formula from memory cannot
  // produce that pair.
  ourThreads: num(ours.stats, "Threads"),
  initThreads: num(foreign.stats, "Threads"),
  // pid 1's parent is 0, which no `1200 + pid % 40` can yield.
  initParent: num(foreign.stats, "Parent process"),
  // PSS must be below RSS: it divides shared pages by their sharers. Equal
  // values would mean one was copied from the other.
  rss: parseFloat(num(ours.mem, "Resident (RSS)")),
  pss: parseFloat(num(ours.mem, "Proportional (PSS)")),
  // smaps_rollup needs ptrace-read, so a foreign process has no memory figures
  // and must show a dash rather than a borrowed number.
  foreignMemBlank: foreign.mem.every(r => /-$/.test(r)),
  ourStats: ours.stats.join(" | ").slice(0, 130) });
JS
got=$(drive "$probes/p-stats.js" sysmon-stats.png)
if printf '%s' "$got" | grep -q '"skipped"'; then
  echo "  --   per-process statistics: $got"
else
  ourT=$(printf '%s' "$got" | sed -n 's/.*"ourThreads":"\([0-9]*\)".*/\1/p')
  initT=$(printf '%s' "$got" | sed -n 's/.*"initThreads":"\([0-9]*\)".*/\1/p')
  initP=$(printf '%s' "$got" | sed -n 's/.*"initParent":"\([0-9-]*\)".*/\1/p')
  rss=$(printf '%s' "$got" | sed -n 's/.*"rss":\([0-9.]*\).*/\1/p')
  pss=$(printf '%s' "$got" | sed -n 's/.*"pss":\([0-9.]*\).*/\1/p')
  ok=0
  [ -n "$ourT" ] && [ "$ourT" -gt 1 ] && [ "$initT" = 1 ] && [ "$initP" = 0 ] \
    && printf '%s' "$got" | grep -q '"foreignMemBlank":true' \
    && awk "BEGIN{exit !($pss > 0 && $pss < $rss)}" && ok=1
  say "the statistics and memory tabs read the kernel, and stay blank when they cannot" "$ok" "$got"
fi


# THE GLOBAL REFRESH RATE, which the plan names beside freeze-the-refresh. The
# failure mode for a control like this is being cosmetic: the select changes, the
# stored value changes, and the timer keeps its old period. So this MEASURES the
# cadence at both ends of the range instead of asserting the widget moved.
cat > "$probes/p-rate.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
await wait(2500);
const sel = document.querySelector("select.rate-select");
if (!sel) return JSON.stringify({ error: "no rate control" });
const set = async (v) => {
  sel.value = v;
  sel.dispatchEvent(new Event("change", { bubbles: true }));
  await wait(400);
};
// How often the top row's text changes over `secs`. CPU figures move every poll
// on any real machine, so this counts polls without needing a clock inside the
// app.
const changes = async (secs) => {
  let last = null, n = 0;
  for (let i = 0; i < secs * 4; i++) {
    await wait(250);
    const v = document.querySelectorAll("[role=row][data-pid]")[0]?.innerText || "";
    if (last !== null && v !== last) n++;
    last = v;
  }
  return n;
};
await set("500");
const fast = await changes(5);
await set("10000");
const slow = await changes(5);
const stored = localStorage.getItem("arlen.system-monitor.refreshMs");
// Put it back. The rate PERSISTS, so leaving it at 10s made the next probe's app
// start with no second sample yet - every row read a dash where the sort probe
// expected figures, and that probe went red with nothing wrong in the app. A
// probe that changes remembered state has to change it back.
await set("2000");
return JSON.stringify({ opts: [...sel.options].map(o => o.textContent.trim()),
  fast, slow, stored, restored: localStorage.getItem("arlen.system-monitor.refreshMs") });
JS
got=$(drive "$probes/p-rate.js" sysmon-rate.png)
fast=$(printf '%s' "$got" | sed -n 's/.*"fast":\([0-9]*\).*/\1/p')
slow=$(printf '%s' "$got" | sed -n 's/.*"slow":\([0-9]*\).*/\1/p')
ok=0
# Five seconds at 500ms is up to ten polls and at 10s is at most one, so the two
# cannot be confused by a slow machine or a quiet moment.
[ -n "$fast" ] && [ -n "$slow" ] && [ "$fast" -ge 4 ] && [ "$slow" -le 1 ] \
  && printf '%s' "$got" | grep -q '"stored":"10000"' && ok=1
say "the refresh-rate control changes how often the machine is actually read" "$ok" "$got"


[ "$fail" = 0 ] && echo "a process list that sorts, a detail that names a pid, graphs that draw, and a kill that asks before it acts"
exit "$fail"
