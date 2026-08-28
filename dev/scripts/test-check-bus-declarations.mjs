// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-bus-declarations: plant the defect it exists
// for and watch it fail. A gate that has only ever been run against a tree that
// already passes cannot be told apart from one that always passes.

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-bus-declarations.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// The gate also refuses an UNBOUNDED entry that names a component shipping no
// profile, so the table cannot rot into excusing something that left. That means
// a fixture tree has to carry a profile for every excused name, or every case
// fails for a reason unrelated to what it is testing. `excused: false` opts out,
// which is how the stale-entry direction is tested.
const EXCUSED = "knowledge";
const EXCUSED_PROFILE = `[info]\napp_id = "${EXCUSED}"\n\n[event_bus]\npublish = ["project.*"]\n`;

function tree(profiles, { excused = true } = {}) {
  const dir = mint("bus-decl-");
  mkdirSync(join(dir, PROFILES), { recursive: true });
  if (excused) writeFileSync(join(dir, PROFILES, `${EXCUSED}.toml`), EXCUSED_PROFILE);
  for (const [name, body] of Object.entries(profiles)) {
    writeFileSync(join(dir, PROFILES, `${name}.toml`), body);
  }
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const BOTH = '[info]\napp_id = "a"\n\n[event_bus]\npublish = ["x.*"]\nsubscribe = []\n';
const ONLY_PUBLISH = '[info]\napp_id = "b"\n\n[event_bus]\npublish = ["x.*"]\n';
const NO_BUS = '[info]\napp_id = "c"\n\n[graph]\nread = ["system.File.id"]\n';

// The defect: on the bus, says what it sends, silent about what it hears. That
// silence is read as unbounded, which is the whole reason for the check.
{
  const d = tree({ b: ONLY_PUBLISH });
  const r = run(d);
  check("a publisher that never declares subscribe is caught", r.code === 1);
  check("and the message names the missing half", r.out.includes("`subscribe`"));
  cleanup(d);
}

// An EMPTY list is an answer, not an omission - the distinction the whole tier
// split rests on. A gate that rejected it would push people back to silence.
{
  const d = tree({ a: BOTH });
  check("declaring subscribe = [] passes", run(d).code === 0);
  cleanup(d);
}

// Not every profile is a bus participant, and one that never mentions the bus
// must not be asked to declare its traffic.
{
  const d = tree({ c: NO_BUS });
  check("a profile with no event_bus section is not a subject", run(d).code === 0);
  cleanup(d);
}

// Pointed at a tree with no profiles at all, "found nothing wrong" would be a
// lie about a scan that read nothing.
{
  const d = mint("bus-decl-empty-");
  check("an absent profile directory is an error, not a pass", run(d).code === 2);
  cleanup(d);
}

// The reverse direction, which is what keeps the excuse list honest: a name
// excused from declaring `subscribe` that ships no profile at all is a stale
// entry, and saying so is the difference between a list and a graveyard.
{
  const d = tree({ a: BOTH }, { excused: false });
  const r = run(d);
  check("an excuse for a component that ships nothing is caught", r.code === 1);
  check("and the message says to delete the entry", r.out.includes("delete the entry"));
  cleanup(d);
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
