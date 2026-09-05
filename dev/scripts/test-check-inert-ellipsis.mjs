// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the inert-ellipsis gate see a rule that declares one and cannot honour it?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses matter as much as the hit. A file with both properties in
// DIFFERENT rules is correct and common - a flex row whose child ellipses is the
// standard fix - so a grep would report every such component and be turned off
// within a day. That is why the gate parses rules.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-inert-ellipsis.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component) {
  const dir = mint("arlen-inert-ellipsis-");
  try {
    const at = path.join(dir, "apps", "demo", "src");
    mkdirSync(at, { recursive: true });
    writeFileSync(path.join(at, "Thing.svelte"), component, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("inert ellipsis:");

{
  const r = gateOver(`<span class="x">hi</span>
<style>
  .x {
    display: inline-flex;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>`);
  check("a flex box declaring an ellipsis is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the selector", r.out.includes(".x"), r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<span class="g">hi</span>
<style>
  .g { display: grid; overflow: hidden; text-overflow: ellipsis; }
</style>`);
  check("grid counts as well as flex", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  // THE FIX, and it must pass: the container keeps the layout, the child ellipses.
  const r = gateOver(`<span class="row"><span class="t">hi</span></span>
<style>
  .row { display: flex; overflow: hidden; white-space: nowrap; }
  .t { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>`);
  check("the two-element fix passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 component(s)"),
        r.out.trim().split("\n").pop());
}

{
  // A plain text box with an ellipsis is the ordinary case and must not be named.
  const r = gateOver(`<div class="t">hi</div>
<style>
  .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>`);
  check("a block box with an ellipsis is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // Inside a media query, which is where half this tree's narrow-width rules live.
  const r = gateOver(`<span class="x">hi</span>
<style>
  @media (max-width: 40rem) {
    .x { display: flex; text-overflow: ellipsis; overflow: hidden; }
  }
</style>`);
  check("a rule inside a media query is seen too", r.code === 1, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds an ellipsis a flex or grid box cannot honour");
