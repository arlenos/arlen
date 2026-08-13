// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the unit-identity drift check: plant each way the
// hand-kept table can fall out of step with the shipped units, and watch it refuse.
//
// The check exists because the table is hand-kept ON PURPOSE - the cgroup route is
// sound precisely because the kernel guarantees the unit name while we choose the
// app_id - and a hand-kept table drifts quietly in the worst direction: a new
// daemon ships, nothing maps it, and it authenticates as nobody, which from
// outside looks exactly like a daemon refused for a good reason.

import { mkdtempSync, writeFileSync, mkdirSync, cpSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-unit-identity.py");
const UNITS = "dev/mkosi/mkosi.extra/usr/lib/systemd/system";
const RESOLVER = "sdk/permissions/src/unit_identity.rs";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// Run the check against a COPY of the tree, so a planted defect never touches the
// real one. The check resolves its paths from its own location, so it travels.
function withTree(mutate) {
  const dir = mkdtempSync(join(tmpdir(), "unit-identity-"));
  mkdirSync(join(dir, UNITS), { recursive: true });
  mkdirSync(join(dir, dirname(RESOLVER)), { recursive: true });
  mkdirSync(join(dir, "dev/scripts"), { recursive: true });
  cpSync(join(ROOT, UNITS), join(dir, UNITS), { recursive: true });
  cpSync(join(ROOT, RESOLVER), join(dir, RESOLVER));
  cpSync(CHECK, join(dir, "dev/scripts/check-unit-identity.py"));
  mutate(dir);
  const r = spawnSync("python3", [join(dir, "dev/scripts/check-unit-identity.py")], {
    encoding: "utf8",
  });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// The tree as it stands passes, so a failure below is the planted defect and not
// a pre-existing one.
{
  const r = withTree(() => {});
  check("the tree as it stands passes", r.code === 0);
  check("and it says how many units are named", r.out.includes("named by the cgroup resolver"));
}

// A new system daemon ships and nobody maps it. This is the quiet one.
{
  const r = withTree((dir) => {
    writeFileSync(
      join(dir, UNITS, "arlen-newthing.service"),
      "[Service]\nExecStart=/usr/bin/arlen-newthing\n",
    );
  });
  check("a shipped unit with no entry is refused", r.code === 1);
  check("and the message names the unit", r.out.includes("arlen-newthing.service"));
}

// An entry for a unit that no longer ships: coverage that cannot fire.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace(
        '("arlen-timeline.service", "timeline"),',
        '("arlen-timeline.service", "timeline"),\n    ("arlen-gone.service", "gone"),',
      ),
    );
  });
  check("an entry for a unit that does not ship is refused", r.code === 1);
  check("and the message says it cannot fire", r.out.includes("cannot fire"));
}

// Both excused and mapped - the state the check caught on its very first run,
// when this file's author had done exactly that to arlen-llama.service.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace(
        '("arlen-timeline.service", "timeline"),',
        '("arlen-timeline.service", "timeline"),\n    ("arlen-llama.service", "llama"),',
      ),
    );
  });
  check("a unit both excused and mapped is refused", r.code === 1);
  check("and the message says the excuse outlived its reason", r.out.includes("outlived"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery drift is caught");
process.exit(failures ? 1 : 0);
