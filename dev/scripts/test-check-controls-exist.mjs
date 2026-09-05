// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the every-check-has-a-control gate see a check with none?
//
// It has to hold for itself, and it found two the hand sweep that prompted it had
// missed - I looked at `check-*.py` and `check-*.sh` and forgot that some checks
// are `.mjs`. That is the argument for the gate in one line: the property is easy
// to hold and easy to lose, and a person enumerating file types by hand loses it.
//
// Over a fixture, like every control here: the gates run concurrently, so one that
// writes into `dev/scripts` would be visible to its neighbours mid-run.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-controls-exist.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// A fixture `dev/scripts` holding exactly the files named.
function gateOver(files) {
  const dir = mint("arlen-controls-exist-");
  try {
    const scripts = path.join(dir, "dev", "scripts");
    mkdirSync(scripts, { recursive: true });
    for (const f of files) writeFileSync(path.join(scripts, f), "// probe\n", "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("controls exist:");

{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver(["check-thing.py", "test-check-thing.mjs"]);
  check("a check with a control passes", r.code === 0, r.out.trim().split("\n").pop());
  // Pinned: an empty scripts directory also exits 0 with "0 check(s)", which is
  // how this would go vacuous.
  check("and the gate actually saw the check", r.out.includes("1 check(s)"),
        r.out.trim().split("\n").pop());
}

{
  const r = gateOver(["check-thing.py"]);
  check("a check with no control is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names it", r.out.includes("check-thing.py"));
}

{
  // The three suffixes a check comes in. `.mjs` is the one the hand sweep missed,
  // so it is the one that most needs asserting.
  const r = gateOver(["check-a.py", "check-b.sh", "check-c.mjs"]);
  check("a check in any of the three suffixes is counted", r.code === 1 &&
        r.out.includes("check-a.py") && r.out.includes("check-b.sh") && r.out.includes("check-c.mjs"),
        r.out.trim().split("\n")[0]);
}

{
  // A control may be either suffix; neither is preferred and both must satisfy it.
  const r = gateOver(["check-thing.sh", "test-check-thing.py"]);
  check("a python control satisfies a shell check", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a check with nothing proving it can still fail");
