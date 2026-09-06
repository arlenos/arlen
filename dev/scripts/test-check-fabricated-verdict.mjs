// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-fabricated-verdict.py`.
//
// The positive case is the real one, reduced: the add-provider dialog's Test
// button as it stood before 7 September. The negative cases are the shapes that
// look like it and are not, because a gate that cannot tell them apart gets
// switched off - and the ordinary handler (call, then set the result) is by far
// the most common function in this tree.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "check-fabricated-verdict.py"), "utf8");

let failed = 0;
function ok(name, cond, detail = "") {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) {
    failed++;
    if (detail) console.log(`       ${detail.trim().split("\n")[0]}`);
  }
}

// The gate reads its own tree, never this one: the fixture is minted per case and
// removed after, so a run cannot leave a file behind for the next check to read.
function run(script, name = "Thing.svelte") {
  const root = mint("fabricated-verdict-");
  try {
    const dir = join(root, "apps", "demo", "src", "lib");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, name), `<script lang="ts">\n${script}\n</script>\n<div></div>\n`);
    mkdirSync(join(root, "dev", "scripts"), { recursive: true });
    const copy = join(root, "dev", "scripts", "check-fabricated-verdict.py");
    writeFileSync(copy, source);
    const r = spawnSync("python3", [copy, root], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

console.log("the gate catches a verdict stated without asking");

// 1. The real one: two assignments, no call.
{
  const { code, out } = run(`
  let test = $state({ kind: "idle" });
  function runTest() {
    test = { kind: "testing" };
    test = { kind: "ok" };
  }
`);
  ok("a handler that states a result and calls nothing is caught", code === 1, out);
  ok("and the finding names the handler", out.includes("runTest"), out);
  ok("and the gate actually read the fixture", /^1 file\(s\) checked/m.test(out), out);
}

// 2. The ordinary case, and the one that must never be a finding: it asks first.
{
  const { code } = run(`
  let test = $state({ kind: "idle" });
  async function runTest() {
    const r = await invoke("provider_test", { id });
    test = r.ok ? { kind: "ok" } : { kind: "network" };
  }
`);
  ok("a handler that asks first is not a finding", code === 0);
}

// 3. An affirmative literal that is not a claim about the world.
{
  const { code } = run(`
  let mode = $state("off");
  function pick() {
    mode = "ok";
  }
`);
  ok("an affirmative on a name that is not a verdict is not a finding", code === 0);
}

// 4. Prose about a fabrication must not read as one - this file's own header
//    would otherwise trip its own gate.
{
  const { code } = run(`
  function note() {
    // This used to set status = "ok" without calling anything.
    return 1;
  }
`);
  ok("the same words in a comment are not a finding", code === 0);
}

// 5. The documented scope: a fabricated FAILURE is wrong too and is not this
//    gate's, because it is the safe direction and nobody types a key into it.
{
  const { code } = run(`
  let status = $state("idle");
  function check() {
    status = "unreachable";
  }
`);
  ok("a fabricated failure is out of scope, as documented", code === 0);
}

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
console.log("both directions hold");
