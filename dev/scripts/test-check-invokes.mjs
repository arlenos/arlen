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
import { spawnSync } from "node:child_process";

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
  // Both streams on every path. Reading `execFileSync`'s return value catches
  // stdout alone, so a case asserting on something the gate writes to stderr while
  // still exiting 0 would silently compare against an empty string - and the sync
  // call additionally echoes the child's stderr here, printing a wall of red above
  // an EXPECTED failure. Found twice in sibling gate tests before being fixed here.
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
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

// Both drift directions on the inventory, using the real one: `apps/knowledge`
// carries entries, and an app named `knowledge` in a throwaway tree is measured
// against them. The direction that bit was the second one - the gate simply
// stopped counting a fixed command and the entry sat there forever, so the total
// read as debt that someone had already paid.
check(
  "an inventory entry whose command now exists is reported",
  tree({
    "apps/knowledge/package.json": "{}",
    "apps/knowledge/src/lib/x.ts": 'await invoke("knowledge_library");\n',
    "apps/knowledge/src-tauri/src/lib.rs": HOST.replace(/open_thing/g, "knowledge_library"),
  }),
  (code, out) => code !== 0 && out.includes("knowledge_library") && out.includes("now registers it"),
);

check(
  "an inventory entry with neither a call nor a command stays quiet",
  // Carried, still missing, still invoked: the ordinary state of the inventory,
  // which must not fail the check or the count would be unusable.
  tree({
    "apps/knowledge/package.json": "{}",
    "apps/knowledge/src/lib/x.ts": 'await invoke("knowledge_library");\n',
    "apps/knowledge/src-tauri/src/lib.rs": "// no host commands here\n",
  }),
  (code) => code === 0,
);

// The shape that inflated the uncalled count: the name is chosen at runtime and
// assigned, so the literal never sits inside the `invoke(` call. Settings' module
// store does this for real, and both of its commands were reported as called by
// nothing while being in daily use.
check(
  "a command reached through a variable is not reported as uncalled",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'const cmd = flag ? "open_thing" : "open_other";\nawait invoke(cmd, { id });\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && out.includes("0 registered command(s)"),
);

// And the boundary that keeps it safe: the same literals must NOT satisfy the
// missing-command check, or a discriminant string in an assignment would become
// an invoked command and fail a gate over nothing.
check(
  "a variable-borne name is not treated as a call that needs a command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'const cmd = kind === "builtin" ? "open_thing" : "open_thing";\nawait invoke(cmd);\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && !out.includes("builtin"),
);

check(
  "an app with no host at all does not crash the gate",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": "export const x = 1;\n",
  }),
  (code) => code === 0,
);

// A template literal can hold a document rather than code. The text editor ships
// two demo files that way, one of them showing example Arlen code with an
// `invoke` in it - and the scanner read that sample as a call this binary makes,
// so it sat on the missing-command list for weeks as work nobody could finish.
check(
  "an invoke inside a template literal is sample text, not a call site",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "const DOC = `example:\n  await invoke(\"not_a_real_command\");\n`;\nexport default DOC;\n",
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && !out.includes("not_a_real_command"),
);

// The other direction, so the blanking cannot quietly swallow real calls: a
// genuine invoke beside a template literal is still found.
check(
  "a real call next to a template literal is still seen",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "const DOC = `await invoke(\"decoy\")`;\nawait invoke(\"open_missing\");\n",
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing") && !out.includes("decoy"),
);

console.log(failures.length ? "\nsome cases regressed" : "\nboth directions hold");
process.exit(failures.length ? 1 : 0);
