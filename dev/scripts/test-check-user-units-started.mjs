// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the user-unit start check.
//
// The red case is what shipped twice: a user unit with `[Install]
// WantedBy=default.target` and nothing that acts on it. That is not a
// misconfiguration systemd complains about - it is a unit that never runs, with
// no line in the boot log to notice.
//
// The green cases are the two real ways a user unit gets started, and they have
// to BOTH pass, because the tree uses both: the supervisor's table for the
// daemons whose peers need to name them, and a wants-link for the verify probe,
// which must not appear in shipped release code.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync, symlinkSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-user-units-started.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

const UNIT = "[Unit]\nDescription=x\n\n[Service]\nExecStart=/bin/true\n\n[Install]\nWantedBy=default.target\n";

// Builds a tree with one user unit, optionally in the table and/or wants-linked.
function run({ inTable = false, linked = false, units = ["arlen-thing.service"] } = {}) {
  const dir = mint("userstart-");
  const u = join(dir, "dev/mkosi/mkosi.extra/usr/lib/systemd/user");
  mkdirSync(u, { recursive: true });
  for (const name of units) writeFileSync(join(u, name), UNIT);

  if (linked) {
    mkdirSync(join(u, "default.target.wants"), { recursive: true });
    for (const name of units) {
      symlinkSync(`../${name}`, join(u, "default.target.wants", name));
    }
  }

  const rs = join(dir, "sdk/permissions/src/unit_identity.rs");
  mkdirSync(dirname(rs), { recursive: true });
  const rows = inTable ? units.map((n) => `    ("${n}", "thing"),\n`).join("") : "";
  writeFileSync(rs, `const USER_UNIT_APP_IDS: &[(&str, &str)] = &[\n${rows}];\n`);

  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("user units are started:");

{
  const r = run({});
  check("a unit nothing starts is caught", r.code === 1);
  check("and the message names it", r.out.includes("arlen-thing.service"));
  check("and says [Install] is not the enable", r.out.includes("does not enable"));
}
{
  const r = run({ inTable: true });
  check("the supervisor's table counts as started", r.code === 0);
}
{
  const r = run({ linked: true });
  check("a default.target.wants link counts as started", r.code === 0);
}
{
  // Both routes at once must not be treated as a conflict - the graph daemon is
  // in the table and could reasonably gain a link too.
  const r = run({ inTable: true, linked: true });
  check("both routes together is fine", r.code === 0);
}
{
  // One covered and one stranded: the pass must not be decided by the first unit
  // it happens to read.
  const r = run({ linked: true, units: ["a.service", "b.service"] });
  const partial = run({ inTable: true, units: ["a.service", "b.service"] });
  check("all units are checked, not just one", r.code === 0 && partial.code === 0);
}
{
  const dir = mint("userstart-empty-");
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  check("a tree with no user units refuses rather than passing", r.status === 2);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
