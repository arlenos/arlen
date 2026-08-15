// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the emitter-declaration check.
//
// The red case is the one that happened: a daemon that emits, is in the image,
// and has no profile - which no boot in a VM can surface, because the trigger is
// hardware the guest does not have. The green cases pin the two ways a component
// is legitimately quiet here: it already has a profile, or it is not shipped yet
// and says so.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-emitters-declared.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";
const IDENTITY = "sdk/permissions/src/identity.rs";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// One resolver arm, in the shape the check reads out of identity.rs.
const ARMS = `
        "/usr/lib/arlen/libexec/arlen-powerd" => {
            return Ok("powerd".to_string());
        }
`;
const EMITS = `
async fn publish(emitter: &E) {
    emitter.emit("power.state", bytes).await
}
`;
const PROFILE = '[info]\napp_id = "powerd"\n\n[event_bus]\npublish = ["power.state"]\nsubscribe = []\n';

// The gate's own NOT_SHIPPED table, injected for the carried case below rather
// than borrowed from it. That table named `daemons/modulesd` until modulesd
// shipped on 15 Aug and it went empty, at which point the case that exercises
// carrying had no subject and went red while nothing was wrong. A control tied to
// a live entry expires the day the entry is resolved.
const CARRIED = 'NOT_SHIPPED: dict[str, str] = {"daemons/example": "not in the image yet"}';

function run(files, mutate = (s) => s) {
  const dir = mkdtempSync(join(tmpdir(), "emitters-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const gate = join(dir, "check.py");
  writeFileSync(gate, mutate(readFileSync(GATE, "utf8")));
  const r = spawnSync("python3", [gate, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("emitters declared:");

{
  // The defect: emits, resolvable, no profile. A profile has to exist for the
  // check to have read anything at all, so an unrelated one stands in.
  const r = run({
    [IDENTITY]: ARMS,
    "daemons/power-daemon/src/main.rs": EMITS,
    [`${PROFILES}/somethingelse.toml`]: '[info]\napp_id = "somethingelse"\n',
  });
  check(
    "an emitter with no profile is caught",
    r.code === 1 && r.out.includes("powerd.toml"),
  );
}
{
  const r = run({
    [IDENTITY]: ARMS,
    "daemons/power-daemon/src/main.rs": EMITS,
    [`${PROFILES}/powerd.toml`]: PROFILE,
  });
  check("the same emitter with its profile passes", r.code === 0);
}
{
  // The directory name is `power-daemon` and the id is `powerd`. A check that
  // looked for `power-daemon.toml` would report a profile that exists as
  // missing, and send somebody to write a second one nothing loads.
  const r = run({
    [IDENTITY]: ARMS,
    "daemons/power-daemon/src/main.rs": EMITS,
    [`${PROFILES}/power-daemon.toml`]: PROFILE,
  });
  check(
    "a profile named after the directory rather than the id is not accepted",
    r.code === 1,
  );
}
{
  const r = run(
    {
      [IDENTITY]: ARMS,
      "daemons/example/src/main.rs": 'fn f() { emitter.emit("example.thing", b).await }\n',
      [`${PROFILES}/powerd.toml`]: PROFILE,
    },
    (s) => s.replace(/NOT_SHIPPED: dict\[str, str\] = \{[\s\S]*?\}/, CARRIED),
  );
  check("an emitter that is not shipped is carried, not failed", r.code === 0);
  check("and says it needs one the day it ships", r.out.includes("the day it ships"));
}
{
  const r = run({ "README.md": "nothing emits here\n" });
  check("a tree with no emitter refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
