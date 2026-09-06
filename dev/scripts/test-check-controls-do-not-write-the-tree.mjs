// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-controls-do-not-write-the-tree.py`.
//
// The positive case is the real one: the shape two of this directory's controls
// had on 7 September, where the path to a repository file goes through a `const`
// before the write. That hop is the whole test - a first cut of the check looked
// only at the write's own argument, found nothing in either real case, and would
// have shipped green.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-controls-do-not-write-the-tree.py");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

function run(body, name = "test-demo.mjs") {
  const root = mint("controls-write-");
  try {
    const dir = join(root, "dev", "scripts");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, name), body);
    const r = spawnSync("python3", [check, root], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

const HEAD = `const here = dirname(fileURLToPath(import.meta.url));\nconst root = join(here, "..", "..");\n`;

console.log("the gate catches a control that edits the repository");

// 1. The real shape: the path goes through a const first.
{
  const { code, out } = run(`${HEAD}
const path = join(root, "apps", "screenshot", "src-tauri", "src", "lib.rs");
const original = readFileSync(path, "utf8");
writeFileSync(path, original.replace("a", "b"));
`);
  ok("a write to a repo path held in a variable is caught", code === 1);
  ok("and the finding names the call", out.includes("writeFileSync"));
}

// 2. Written straight through, no hop.
{
  const { code } = run(`${HEAD}
writeFileSync(join(root, "apps", "x.ts"), "");
`);
  ok("a write built inline is caught too", code === 1);
}

// 3. The fix: a minted tree, and the check pointed at it.
{
  const { code } = run(`${HEAD}
const tree = mint("demo-");
writeFileSync(join(tree, "apps", "x.ts"), "");
cleanup(tree);
`);
  ok("a write into a minted tree passes", code === 0);
}

// 4. Reading the repository is what most of these do.
{
  const { code } = run(`${HEAD}
const source = readFileSync(join(root, "dev", "scripts", "check-x.py"), "utf8");
`);
  ok("reading the repository passes", code === 0);
}

// 5. A control with no repository root at all.
{
  const { code } = run(`const tree = mint("demo-");\nwriteFileSync(join(tree, "a.ts"), "");\n`);
  ok("a control that never names the root passes", code === 0);
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
