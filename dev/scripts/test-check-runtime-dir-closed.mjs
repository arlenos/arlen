// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the `/run/arlen` mode gate.
//
// The gate guards a security property that has no other symptom: if the directory
// becomes user-writable, the undo signer keeps working and quietly stops being sure
// who answered. So what has to be shown is not that the gate can print OK, but that
// each way of losing the property turns it red - a widened mode, a non-root owner,
// and the declaration disappearing altogether.

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync, cpSync, rmSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-runtime-dir-closed.py");
const UNIT_REL = "dev/mkosi/mkosi.extra/usr/lib/systemd/system";
const UNIT = "arlen-event-bus.service";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** Run the gate against a COPY of the unit tree with `mutate` applied to the unit. */
function run(mutate) {
  const dir = mkdtempSync(join(tmpdir(), "runtime-dir-"));
  const units = join(dir, UNIT_REL);
  mkdirSync(units, { recursive: true });
  cpSync(join(ROOT, UNIT_REL), units, { recursive: true });
  const path = join(units, UNIT);
  const next = mutate(readFileSync(path, "utf8"));
  if (next === null) rmSync(path);
  else writeFileSync(path, next);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("runtime dir closed:");

{
  const r = run((s) => s);
  check("the tree as it stands passes", r.code === 0);
  check("and names the unit that creates it", r.out.includes("arlen-event-bus.service"));
}
{
  // The one that matters: group/other write is exactly what lets a user bind a
  // socket there and be trusted as the broker.
  const r = run((s) => s.replace("RuntimeDirectoryMode=0755", "RuntimeDirectoryMode=0777"));
  check("a world-writable mode is caught", r.code === 1);
  check("and says why it matters", r.out.includes("bind a socket"));
}
{
  // Group write alone, because 0775 looks harmless next to 0777.
  const r = run((s) => s.replace("RuntimeDirectoryMode=0755", "RuntimeDirectoryMode=0775"));
  check("group write alone is caught too", r.code === 1);
}
{
  // A non-root owner: the directory is then owned by whatever that account is,
  // and the gate should say so rather than assume it is a service account.
  const r = run((s) => s.replace("RuntimeDirectoryMode=0755", "User=tim\nRuntimeDirectoryMode=0755"));
  check("a non-root owner is reported", r.code === 1 && r.out.includes("User=tim"));
}
{
  // The declaration vanishing is the quietest failure of all: nothing is wrong
  // with any unit, the directory is simply whatever created it.
  const r = run(() => null);
  check("losing the declaration entirely is caught", r.code === 1);
  check("and says the check would rest on air", r.out.includes("rests on air"));
}
{
  // An unparsable mode must refuse rather than pass. A mode nobody can read is
  // precisely where a reader stops checking.
  const r = run((s) => s.replace("RuntimeDirectoryMode=0755", "RuntimeDirectoryMode=rwxr-xr-x"));
  check("a mode that is not octal is refused, not assumed safe", r.code === 1);
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe runtime-dir gate holds");
process.exit(failures ? 1 : 0);
