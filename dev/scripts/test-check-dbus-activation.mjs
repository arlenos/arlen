// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-dbus-activation. The defect it exists for shipped
// once already: `org.arlen.InstallDaemon1` had no `SystemdService=`, so activating
// it would have run the install daemon - the one that writes as root - outside the
// sandbox its own unit declares, while looking identical to the confined path.

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "../..");
const GATE = join(HERE, "check-dbus-activation.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// A tree with one activation file and, optionally, the unit it points at.
const EXEC = "/usr/lib/arlen/libexec/arlen-thing";

function tree({ pointer = "arlen-thing.service", withUnit = true, unitExec = EXEC }) {
  const dir = mint("dbus-activation-");
  const dist = join(dir, "daemons/thing/dist");
  mkdirSync(dist, { recursive: true });
  writeFileSync(
    join(dist, "org.arlen.Thing1.service"),
    "[D-BUS Service]\nName=org.arlen.Thing1\n" +
      `Exec=${EXEC}\n` +
      (pointer === null ? "" : `SystemdService=${pointer}\n`)
  );
  if (withUnit) {
    writeFileSync(join(dist, "arlen-thing.service"), `[Service]\nExecStart=${unitExec}\n`);
  }
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

check("the repository as it stands passes", run(ROOT).code === 0);

// The defect that shipped: activation runs the bare Exec.
{
  const d = tree({ pointer: null });
  const r = run(d);
  check("an activation file with no SystemdService is caught", r.code === 1);
  check("and the message says the daemon starts unconfined", r.out.includes("OUTSIDE the sandbox"));
  cleanup(d);
}

// A pointer at nothing is as quiet as no pointer, and fails later - when a user
// first needs the daemon rather than when the file was written.
{
  const d = tree({ pointer: "arlen-typo.service" });
  const r = run(d);
  check("a pointer at a unit that does not exist is caught", r.code === 1);
  check("and the message names the missing unit", r.out.includes("arlen-typo.service"));
  cleanup(d);
}

// Two lines describing one daemon, disagreeing. Invisible while systemd answers,
// which is exactly why it needs a check rather than a reader.
{
  const d = tree({ unitExec: "/usr/bin/arlen-thing-old" });
  const r = run(d);
  check("an Exec that differs from the unit's ExecStart is caught", r.code === 1);
  check("and the message gives both binaries", r.out.includes("arlen-thing-old") && r.out.includes(EXEC));
  cleanup(d);
}

// A hardened unit decorates its ExecStart with `+`, `!` or `-`; that is the same
// binary and must not read as a disagreement.
{
  const d = tree({ unitExec: `+${EXEC}` });
  check("a prefixed ExecStart is not a disagreement", run(d).code === 0);
  cleanup(d);
}

// The intended arrangement stays quiet, or the gate forbids what it exists to require.
{
  const d = tree({});
  check("a file pointing at a unit that exists is not a finding", run(d).code === 0);
  cleanup(d);
}

// A tree with no activation files has not been checked, and saying so beats a
// cheerful pass - the failure mode this project has hit twice.
{
  const dir = mint("dbus-activation-empty-");
  check("a tree with no activation files is an error, not a pass", run(dir).code === 2);
  cleanup(dir);
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
