// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The scope gate had no positive control, and it earned one the day it turned
// out to have been quietly right for a while: both its acknowledged cross-app
// calls had been fixed by each app growing its own command, and neither entry
// noticed. The staleness half below is the guard added for that, and it is the
// case most worth pinning - a check that reports a defect is easy to trust, one
// that reports a stale EXCUSE is the thing nobody writes a test for.
//
// Run: node dev/scripts/test-check-invoke-scope.mjs

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-invoke-scope.py");

const failures = [];

// The gate needs at least one #[tauri::command] under apps/ or it reports that
// it needs updating and exits 2 - a guard against being pointed at a tree whose
// layout moved. Every fixture therefore carries a defining app.
const OWNER =
  "#[tauri::command]\npub fn shell_thing() -> u32 { 1 }\n" +
  "#[tauri::command]\npub fn own_thing() -> u32 { 2 }\n";

function check(name, files, expect, { acknowledged } = {}) {
  const dir = mint("arlen-scope-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // ACKNOWLEDGED is hardcoded in the gate and is empty, so the staleness guard
  // cannot be reached from a fixture tree alone. Rather than leave that half
  // unproven - which is how it got into this state - the test runs a COPY of the
  // gate with an entry planted. It is the same file otherwise, so the logic under
  // test is the real one.
  let gate = GATE;
  if (acknowledged) {
    const src = readFileSync(GATE, "utf8").replace(
      "ACKNOWLEDGED: dict[str, str] = {}",
      `ACKNOWLEDGED: dict[str, str] = {${JSON.stringify(acknowledged)}: "planted"}`,
    );
    gate = join(dir, "gate.py");
    writeFileSync(gate, src);
  }
  const r = spawnSync("python3", [gate, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-invoke-scope:");

// The defect the gate exists for: a Tauri command is compiled into one binary,
// so invoking another app's command is rejected at runtime while grep finds the
// name and everything looks wired.
check(
  "a call into another app's command is caught",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn own_thing() -> u32 { 2 }\n",
    "apps/other/src/lib/x.ts": 'await invoke("shell_thing");\n',
  },
  (code, out) => code === 1 && out.includes("shell_thing"),
);

check(
  "the same call passes when the app defines the command itself",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn shell_thing() -> u32 { 9 }\n",
    "apps/other/src/lib/x.ts": 'await invoke("shell_thing");\n',
  },
  (code) => code === 0,
);

// The wrapper hop, added 12 Aug. Before it, this call was not merely unreported -
// it was never asked the question, because the literal never sits at `invoke`.
check(
  "a cross-app call made through a wrapper is caught too",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn own_thing() -> u32 { 2 }\n",
    "apps/other/src/lib/x.ts":
      "async function send(cmd: string, args?: unknown) { await invoke(cmd, args); }\n" +
      'await send("shell_thing", {});\n',
  },
  (code, out) => code === 1 && out.includes("shell_thing"),
);

// Both halves of the staleness guard. An excuse for a call that no longer
// happens, and an excuse for one the app has since taken ownership of - the
// second is the state the real list was found in.
check(
  "an acknowledgement for a call that no longer happens is caught",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn own_thing() -> u32 { 2 }\n",
    "apps/other/src/lib/x.ts": 'await invoke("own_thing");\n',
  },
  (code, out) => code === 1 && out.includes("no longer invokes"),
  { acknowledged: "other::shell_thing" },
);

check(
  "an acknowledgement the app has since taken ownership of is caught",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn shell_thing() -> u32 { 9 }\n",
    "apps/other/src/lib/x.ts": 'await invoke("shell_thing");\n',
  },
  (code, out) => code === 1 && out.includes("now defines"),
  { acknowledged: "other::shell_thing" },
);

// And the case the guard must NOT fire on, or it would report every entry that
// is doing its job.
check(
  "a live acknowledgement stays quiet",
  {
    "apps/shell/src-tauri/src/lib.rs": OWNER,
    "apps/other/src-tauri/src/lib.rs": "#[tauri::command]\npub fn own_thing() -> u32 { 2 }\n",
    "apps/other/src/lib/x.ts": 'await invoke("shell_thing");\n',
  },
  (code, out) => code === 0 && out.includes("planted"),
  { acknowledged: "other::shell_thing" },
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("a cross-app call is caught through a wrapper as well as directly, and a stale excuse is caught too");
