// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-focus-survives-state.py`.
//
// The two positive cases are the real ones, reduced: the greeter's password
// field as it stood before 7 September, and the screenshot editor's swatch. Both
// were found by driving a surface; this asserts the static reading finds them
// too, which is the whole reason for having it beside the probe.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "check-focus-survives-state.py"), "utf8");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

function run(css, name = "Thing.svelte") {
  const root = mint("focus-survives-");
  try {
    const dir = join(root, "apps", "demo", "src", "lib");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, name), `<div class="field"></div>\n<style>\n${css}\n</style>\n`);
    mkdirSync(join(root, "dev", "scripts"), { recursive: true });
    const copy = join(root, "dev", "scripts", "check-focus-survives-state.py");
    writeFileSync(copy, source);
    const r = spawnSync("python3", [copy], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

console.log("the gate catches a focus indication another state can take away");

// 1. The greeter: focus changes only border-color, and the error state sets it.
{
  const { code, out } = run(`
  .field { border: 1px solid grey; }
  .field:focus-within { border-color: white; }
  .field.error { border-color: red; }
`);
  ok("a focus rule a state fully overwrites is caught", code === 1);
  ok("and the finding names both rules", out.includes(":focus-within") && out.includes(".field.error"));
}

// 2. The swatch: selection takes the outline and nothing draws focus.
{
  const { code, out } = run(`
  .swatch { width: 1rem; }
  .swatch.active { outline: 2px solid blue; outline-offset: 1px; }
`);
  ok("a state claiming the outline with no focus rule is caught", code === 1);
  ok("and the finding says the UA ring is suppressed", out.includes("browser's own ring"));
}

// 3. The fix both got: focus on a channel of its own.
{
  const { code } = run(`
  .field:focus-within { border-color: white; box-shadow: 0 0 0 2px blue; }
  .field.error { border-color: red; }
`);
  ok("focus keeping a channel the state does not touch passes", code === 0);
}

// 4. A state that touches something else entirely.
{
  const { code } = run(`
  .field:focus-within { outline: 2px solid blue; }
  .field.error { background: red; }
`);
  ok("a state that changes something else passes", code === 0);
}

// 5. A state rule with no outline and no focus rule is not this check's business.
{
  const { code } = run(`
  .swatch.active { border-color: blue; }
`);
  ok("a state that claims no outline passes", code === 0);
}

// 6. `outline: none` on a state is not a claim on the channel.
{
  const { code } = run(`
  .swatch.active { outline: none; }
`);
  ok("a state clearing the outline passes", code === 0);
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
