// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-fixture-answers-whole.py`, on the real tree.
//
// Unusual among the controls here in that it does NOT mint a fixture repository:
// the check reads Rust structs through the same resolver `check-invoke-shape`
// uses, and standing that whole tree up in a temp dir would be testing the copy
// rather than the check. So it puts the day's two REAL instances back into the
// real host fixtures, requires a refusal naming each, and restores them.

import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const check = join(here, "check-fixture-answers-whole.py");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

function run() {
  const r = spawnSync("python3", [check], { encoding: "utf8" });
  return { code: r.status, out: `${r.stdout}${r.stderr}` };
}

/// Put a partial answer back into a real fixture, run, restore.
function withPartial(file, from, to, fn) {
  const path = join(root, "dev", "screenshot", "hosts", file);
  const original = readFileSync(path, "utf8");
  if (!original.includes(from)) {
    ok(`${file} still contains the line the control edits`, false);
    return;
  }
  writeFileSync(path, original.replace(from, to));
  try {
    fn(run());
  } finally {
    writeFileSync(path, original);
  }
}

console.log("the gate catches a fixture that answers less than the shape it promises");

ok("the tree as it stands passes", run().code === 0);

// The mail instance: a Message with the seven fields the picture needed. `to` is
// the field to remove, not `refusal` - the first cut of this control took the
// optional one and stopped catching anything the moment the check learned that
// `Option` fields are not required.
withPartial(
  "mail-refuses-archive.js",
  '          to: ["you@example.org"],',
  "          zz_removed: [],",
  ({ code, out }) => {
    ok("a Message missing a field is caught", code === 1);
    ok("and the finding names the command and the field", out.includes("mail_open") && out.includes("to"));
  },
);

// The screenshot instance: an output the surface keys its dropdown on.
withPartial(
  "screenshot-refuses-save.js",
  "[{ index: 0, name: \"eDP-1\"",
  "[{ name: \"eDP-1\"",
  ({ code, out }) => {
    ok("an output without its index is caught", code === 1);
    ok("and the finding names it", out.includes("list_outputs") && out.includes("index"));
  },
);

ok("and the tree is green again afterwards", run().code === 0);

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
