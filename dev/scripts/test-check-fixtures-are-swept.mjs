// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-fixtures-are-swept.py`. The fault is a fixture no table
// names; the near-miss it must not report is a fixture named in a row that also
// carries a selector, which is the shape a state behind a click needs.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const gate = path.join(here, "check-fixtures-are-swept.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failed += 1;
    console.log(`  FAIL ${name}`);
    if (detail) console.log(`       ${detail}`);
  }
}

function gateOver(fixtures, tableRow) {
  const dir = mint("arlen-fixtures-swept-");
  try {
    const hosts = path.join(dir, "dev", "screenshot", "hosts");
    mkdirSync(hosts, { recursive: true });
    for (const f of fixtures) writeFileSync(path.join(hosts, `${f}.js`), "// EXPECT: x\n", "utf8");
    writeFileSync(
      path.join(dir, "dev", "screenshot", "sweep-render-all.sh"),
      `SURFACES=(\n  "demo ${tableRow}"\n)\n`,
      "utf8",
    );
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("fixtures-are-swept:");

{
  const r = gateOver(["demo-refuses"], "/@@demo-refuses");
  check("a fixture the table names passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(["demo-refuses", "demo-forgotten"], "/@@demo-refuses");
  check("a fixture nothing names is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file", r.out.includes("demo-forgotten.js"));
}

{
  // Behind a click: selector first, fixture after. Must not read as unnamed.
  const r = gateOver(["demo-refuses"], "/::#open@@demo-refuses");
  check("a row with a selector still names its fixture", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver([], "/@@nothing");
  check("no fixtures at all is a refusal, not a pass", r.code === 1, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("every fixture is named by something that runs it");
