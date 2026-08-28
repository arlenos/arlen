// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The drift this stops: a daemon lands, its unit declares a bus name, nobody adds
// the pair, and the served-object sweep stays green while covering one surface
// less than the day before. A gate that quietly shrinks is the empty-tree pass in
// another costume, so this one gets its control on the same day it is written.
//
// Run: node dev/scripts/test-check-bus-names-covered.mjs

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-bus-names-covered.py");
const LIST = "dev/scripts/served-objects.tsv";
const UNIT = "daemons/probe/dist/arlen-probed.service";

const failures = [];

/** Run a MODIFIED copy of the gate against the real tree.
 *
 * The cases below judge the caller half, which reads the actual shipped set and
 * the actual crates - a fixture tree cannot express that without becoming a second
 * copy of the repo. So the script is mutated instead and pointed at the real root,
 * which the gate already accepts as an argument.
 */
function runMutated(mutate) {
  const dir = mint("bus-gate-");
  const path = join(dir, "check.py");
  writeFileSync(path, mutate(readFileSync(GATE, "utf8")));
  const r = spawnSync("python3", [path, ROOT], { encoding: "utf8", cwd: ROOT });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

/** Assert a plain condition, for the cases that run the gate rather than a tree. */
function assertCase(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
}

function check(name, files, expect) {
  const dir = mint("arlen-busnames-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

const unit = "[Service]\nType=dbus\nBusName=org.arlen.Probe1\n";

console.log("check-bus-names-covered:");

check(
  "a declared name missing from the list is caught",
  { [UNIT]: unit, [LIST]: "# nothing here\n" },
  (code, out) => code === 1 && out.includes("org.arlen.Probe1"),
);

check(
  "the same name carried as a pair passes",
  {
    [UNIT]: unit,
    [LIST]: "arlen-probed\torg.arlen.Probe1\t/org/arlen/Probe1\n",
  },
  (code) => code === 0,
);

// An exclusion is a reason a person reads, not a way to make a name go away, so
// it counts as covered - and an EMPTY reason must not.
check(
  "an exclusion with a reason counts as covered",
  {
    [UNIT]: unit,
    [LIST]: "!exclude\torg.arlen.Probe1\troot-owned on the system bus\n",
  },
  (code) => code === 0,
);

check(
  "an exclusion with no reason does not count",
  { [UNIT]: unit, [LIST]: "!exclude\torg.arlen.Probe1\t\n" },
  (code, out) => code === 1 && out.includes("org.arlen.Probe1"),
);

// The scoping that took two tries to get right: `dev/mkosi/mkosi.tools` is a
// vendored distro toolchain carrying systemd's own units. Demanding the list
// account for logind would be demanding it account for the operating system.
check(
  "a vendored distro unit is not ours to cover",
  {
    [UNIT]: unit,
    "dev/mkosi/mkosi.tools/usr/lib/systemd/system/systemd-logind.service":
      "[Service]\nBusName=org.freedesktop.login1\n",
    [LIST]: "arlen-probed\torg.arlen.Probe1\t/org/arlen/Probe1\n",
  },
  (code, out) => code === 0 && !out.includes("login1"),
);

check(
  "a tree with no declaring unit is a moved layout, not a pass",
  { [LIST]: "# nothing\n" },
  (code, out) => code === 1 && out.includes("layout moved"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
// The caller's half, added 13 Aug: a shipped component dialling a name whose owner
// is not installed. The controls are the two ways that can go wrong - a real
// dangling caller must fail, and a carried one whose owner started shipping must
// fail too, because a carried gap that closed reads as coverage.
{
  const r = runMutated((s) =>
    s.replace('    "org.arlen.Connections1": (', '    "org.arlen.__none__": (')
  );
  assertCase("a dangling caller fails once it is no longer carried",
        r.code === 1 && r.out.includes("org.arlen.Connections1"));
}
{
  const r = runMutated((s) =>
    s.replace('    "org.arlen.Accounts1": (', '    "org.arlen.Clock1": (')
  );
  assertCase("a carried name whose owner ships is caught",
        r.code === 1 && r.out.includes("Clock1"));
}

console.log("a declared name must be paired or excused, and the distro's own units are not ours");
