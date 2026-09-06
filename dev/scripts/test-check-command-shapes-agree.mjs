// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-command-shapes-agree.py`.
//
// It builds a small tree and points the check at it with a root argument. The
// first cut edited the REPOSITORY instead - putting the real divergence back,
// running, restoring - which passes when run alone and fails at random under the
// pre-commit hook, where the gates run concurrently and another check can read
// the tree mid-edit. It did exactly that within the hour of being written.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-command-shapes-agree.py");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

/// `apps/<name>/src-tauri/src/lib.rs` for each entry, then run the check on it.
function run(apps) {
  const root = mint("command-shapes-");
  try {
    for (const [app, body] of Object.entries(apps)) {
      const dir = join(root, "apps", app, "src-tauri", "src");
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, "lib.rs"), body);
    }
    const r = spawnSync("python3", [check, root], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

const cmd = (sig) => `#[tauri::command]\nfn ${sig} {\n    let _ = 1;\n}\n`;

console.log("the gate catches one command name meaning two argument shapes");

// 1. The real divergence, reduced: the same name with and without a level. Under
//    a name that is NOT carried - `frontend_log` is, at two shapes, so writing
//    the case with its own name would have asserted nothing. The first run of
//    this control did exactly that and passed for the wrong reason.
{
  const { code, out } = run({
    files: cmd("thing_log(level: String, msg: String)"),
    screenshot: cmd("thing_log(message: String)"),
  });
  ok("two argument shapes for one name is caught", code === 1);
  ok("and the finding names both", out.includes("thing_log") && out.includes("message"));
}

// 2. Agreement passes, including Tauri's injected arguments, which never travel.
{
  const { code } = run({
    files: cmd("frontend_log(window: tauri::Window, level: String, msg: String)"),
    screenshot: cmd("frontend_log(level: String, msg: String)"),
  });
  ok("the same shape either side of an injected argument passes", code === 0);
}

// 3. One component registering it alone is not a divergence.
{
  const { code } = run({ files: cmd("frontend_log(message: String)") });
  ok("a name registered once passes", code === 0);
}

// 4. Different names are not this check's business.
{
  const { code } = run({
    files: cmd("frontend_log(level: String, msg: String)"),
    screenshot: cmd("log_frontend(message: String)"),
  });
  ok("two names for one job are not compared", code === 0);
}

// 5. A carried name is allowed exactly as many shapes as its entry records, so a
//    THIRD one under an already-carried name still fails. The tree's own
//    `frontend_log` carry is two.
{
  const { code, out } = run({
    files: cmd("frontend_log(level: String, msg: String)"),
    store: cmd("frontend_log(level: String, message: String)"),
    screenshot: cmd("frontend_log(message: String)"),
  });
  ok("a third shape under a carried name is caught", code === 1);
  ok("and it is the carried name that is named", out.includes("frontend_log"));
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
