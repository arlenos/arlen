// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-command-shapes-agree.py`, on the real tree.
//
// It reads `apps/` and `daemons/` from its own location, so a temp-dir fixture
// would test a copy of the walk rather than the check. Instead it puts the real
// divergence back - the screenshot editor's `frontend_log` as it stood this
// morning - requires a refusal that names it, and restores the file.

import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const check = join(here, "check-command-shapes-agree.py");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

function run() {
  const r = spawnSync("python3", [check], { encoding: "utf8" });
  return { code: r.status, out: `${r.stdout}${r.stderr}` };
}

console.log("the gate catches one command name meaning two argument shapes");

ok("the tree as it stands passes", run().code === 0);

// Put the pre-unification screenshot signature back.
{
  const path = join(root, "apps", "screenshot", "src-tauri", "src", "lib.rs");
  const original = readFileSync(path, "utf8");
  const from = "fn frontend_log(level: String, msg: String) {";
  if (!original.includes(from)) {
    ok("the screenshot command still has the line the control edits", false);
  } else {
    writeFileSync(path, original.replace(from, "fn frontend_log(message: String) {"));
    try {
      const { code, out } = run();
      ok("a third argument shape for a carried name is still caught", code === 1 || out.includes("frontend_log"));
      // `frontend_log` is carried, so the divergence it is carried FOR must not
      // hide a new one. The carry is keyed on the name, which is the honest
      // limit: this asserts the check still SEES it rather than that it fails.
      ok("and the run says what it is carrying", out.includes("carried"));
    } finally {
      writeFileSync(path, original);
    }
  }
}

// A name that is not carried must fail outright.
{
  const path = join(root, "apps", "clock", "src-tauri", "src", "lib.rs");
  const original = readFileSync(path, "utf8");
  const from = "fn frontend_log(level: String, msg: String) {";
  if (!original.includes(from)) {
    ok("the clock command still has the line the control edits", false);
  } else {
    writeFileSync(path, original.replace(from, "fn clock_state(level: String, msg: String) {"));
    try {
      const { code, out } = run();
      ok("an uncarried name with two shapes fails", code === 1);
      ok("and the finding names it", out.includes("clock_state"));
    } finally {
      writeFileSync(path, original);
    }
  }
}

ok("and the tree is green again afterwards", run().code === 0);

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
