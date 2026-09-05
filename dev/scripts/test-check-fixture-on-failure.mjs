// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the fixture-on-failure gate still catch a store answering a failed read
// with invented data?
//
// It had no control until 5 September, and it guards the defect this project has
// paid for most often: a catch written while the backend did not exist, still
// there after it landed, so a read that failed in a real session renders design
// data as fact. Its own header lists what that cost - a capture picker offering
// two windows that did not exist, a grants list saying an app had no access it
// could not read. A gate that quietly stopped matching would look exactly like a
// tree with none of that in it.
//
// Over a fixture tree, never this one: the gates run concurrently (see
// `.githooks/pre-commit`), so a control that writes into `apps/` is visible to
// every neighbour while it runs. The gate takes its root as `argv[1]`.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-fixture-on-failure.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// Run the gate over a throwaway tree holding one store file.
function gateOver(body) {
  const dir = mint("arlen-fixture-gate-");
  try {
    const stores = path.join(dir, "apps", "clock", "src", "lib", "stores");
    mkdirSync(stores, { recursive: true });
    writeFileSync(path.join(stores, "probe.ts"), body, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("fixture on failure:");

// The real tree, read-only. Every case below is only meaningful if this is green.
{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

// THE DEFECT, in the shape the header describes: the catch answers with design
// data, so a failed read renders as fact.
{
  const r = gateOver(
    `import { writable } from "svelte/store";\n` +
    `import { invoke } from "@tauri-apps/api/core";\n` +
    `const FIXTURE = [{ id: "made-up" }];\n` +
    `export const rows = writable([]);\n` +
    `export async function load() {\n` +
    `  try {\n    rows.set(await invoke("clock_state"));\n  } catch {\n` +
    `    rows.set(FIXTURE);\n  }\n}\n`,
  );
  check("a catch that answers with a fixture is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file", r.out.includes("probe.ts"), r.out.trim().split("\n")[0]);
}

// The honest shape: the catch records the failure and shows nothing invented.
{
  const r = gateOver(
    `import { writable } from "svelte/store";\n` +
    `import { invoke } from "@tauri-apps/api/core";\n` +
    `export const rows = writable([]);\n` +
    `export const failed = writable(false);\n` +
    `export async function load() {\n` +
    `  try {\n    rows.set(await invoke("clock_state"));\n    failed.set(false);\n` +
    `  } catch {\n    failed.set(true);\n  }\n}\n`,
  );
  check("a catch that records the failure instead passes", r.code === 0, r.out.trim().split("\n")[0]);
  // Pinned EXACTLY, because a run that scanned nothing also exits 0 and prints
  // "0 catch block(s) across 0 frontend file(s)" - which is the shape a vacuous
  // pass takes here, and a looser pattern would accept it.
  check("and the gate actually read the fixture file",
        r.out.includes("1 catch block(s) across 1 frontend file(s)"),
        r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches a fixture answering a failed read and passes a recorded failure");
