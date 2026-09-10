// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-sweep-tables-agree.py`. The fault is a surface the
// render sweep looks at and the accessibility audit does not; the near-misses it
// must not report are the two ways the tables legitimately differ - a pinned
// locale on one side, and rows written in a different order.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const gate = path.join(here, "check-sweep-tables-agree.py");

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

function gateOver(renderRows, axeRows) {
  const dir = mint("arlen-sweep-tables-");
  try {
    const d = path.join(dir, "dev", "screenshot");
    mkdirSync(d, { recursive: true });
    const body = (rows) => `SURFACES=(\n${rows.map((r) => `  "${r}"`).join("\n")}\n)\n`;
    writeFileSync(path.join(d, "sweep-render-all.sh"), body(renderRows), "utf8");
    writeFileSync(path.join(d, "sweep-axe.sh"), body(axeRows), "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("sweep-tables-agree:");

{
  const r = gateOver(["demo /|/one"], ["demo /|/one"]);
  check("two tables naming the same surfaces pass", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(["demo /|/one|/two"], ["demo /|/one"]);
  check("a surface the audit never sees is caught", r.code === 1);
  check("and the finding names it", r.out.includes("demo /two"));
}

{
  // The axe table pins a locale on fixture rows and the render table often does
  // not. Same surface, and reporting it would make the gate unusable.
  const r = gateOver(["demo /@@refuses"], ["demo /?locale=de@@refuses"]);
  check("a pinned locale is the same surface", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(["demo /|/one"], ["demo /one|/"]);
  check("row order does not matter", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(["demo /", "other /"], ["demo /"]);
  check("a whole app the audit skips is caught", r.code === 1);
  check("and it says the app has no row at all", r.out.includes("names no row"));
}

{
  const r = gateOver(["demo /::#open@@refuses"], ["demo /::#other@@refuses"]);
  check("a different click is a different surface", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  const dir = mint("arlen-sweep-tables-empty-");
  let code = 0;
  try {
    mkdirSync(path.join(dir, "dev", "screenshot"), { recursive: true });
    try {
      execFileSync("python3", [gate, dir], { encoding: "utf8" });
    } catch (e) {
      code = e.status ?? 1;
    }
  } finally {
    cleanup(dir);
  }
  check("a tree with no tables is a refusal, not a pass", code === 1);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("both sweeps look at the same surfaces");
