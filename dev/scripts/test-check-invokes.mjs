// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the invoke-exists gate must catch, and what it must leave alone.
//
// This gate decides whether a control on screen has anything behind it: a Tauri
// command is reachable only inside the binary that registers it, so an app
// invoking a name its own host does not register throws on every press, and
// whatever the catch does is what the user gets. That makes both wrong answers
// expensive - a miss ships a dead button, a false alarm trains people to ignore
// the count - so both directions are pinned here.
//
// NOT covered, deliberately: the known-missing inventory and its
// stale-entry guard. Those key off a hardcoded per-app table, so a fixture for
// them would pin this test to today's inventory and break every time the count
// legitimately changes. The guard is real (`check-invoke-exists.py` reports an
// entry whose call is gone); it just cannot be fixture-tested without coupling.
//
// Run: node dev/scripts/test-check-invokes.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-invoke-exists.py");

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-invoke-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  try {
    return { code: 0, out: execFileSync("python3", [GATE, dir], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

// Registration, not annotation. The gate reads the `generate_handler!` list
// because that is what makes a command reachable; a `#[tauri::command]` nobody
// registers is exactly the dead call this gate is for. My first version of this
// fixture annotated without registering and the gate correctly reported it - the
// test was wrong, not the gate.
const HOST = `#[tauri::command]
pub fn open_thing() -> Result<(), String> { Ok(()) }

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![open_thing])
        .run(tauri::generate_context!())
        .expect("run");
}
`;

// Annotated but never registered: still dead, and the gate must say so.
const HOST_UNREGISTERED = `#[tauri::command]
pub fn open_thing() -> Result<(), String> { Ok(()) }
`;

check(
  "a call whose host registers it passes",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code) => code === 0,
);

check(
  "a call with no handler fails and names the command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_missing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing"),
);

check(
  "a call registered in ANOTHER app's host still fails",
  // The binary boundary this gate exists for, and the one that had `topbar_items`
  // looking like a missing producer when the producer was written all along.
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": "// nothing registered here\n",
    "apps/other/package.json": "{}",
    "apps/other/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_thing"),
);

check(
  "a command annotated but never registered is still reported",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST_UNREGISTERED,
  }),
  (code, out) => code !== 0 && out.includes("open_thing"),
);

check(
  "an app with no host at all does not crash the gate",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": "export const x = 1;\n",
  }),
  (code) => code === 0,
);

console.log(failures.length ? "\nsome cases regressed" : "\nboth directions hold");
process.exit(failures.length ? 1 : 0);
