// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the socket-server table.
//
// The gate is half hand-kept, so what has to be shown is that the hand-kept half
// cannot drift quietly: an entry for a socket nobody references any more, a socket
// referenced with no entry, and a carried violation that has since been fixed all
// have to fail. The first version of this check reported OK on the one violation it
// was written for - three separate blind spots in how it matched socket names - so a
// green run means nothing unless the failing shapes are exercised.

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-socket-servers.py");
const SRC = readFileSync(GATE, "utf8");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** Run a modified copy of the gate against the real tree. */
function run(mutate) {
  const dir = mkdtempSync(join(tmpdir(), "socket-gate-"));
  const path = join(dir, "check.py");
  writeFileSync(path, mutate(SRC));
  const r = spawnSync("python3", [path, ROOT], { encoding: "utf8", cwd: ROOT });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("socket servers:");

{
  const r = run((s) => s);
  check("the tree as it stands passes", r.code === 0);
  check("and says how many sockets it covered", /OK: \d+ socket/.test(r.out));
}
{
  // An entry for a socket the tree no longer mentions: the table would otherwise
  // sit there looking like coverage of something that is gone.
  //
  // Anchored on `knowledge.sock`, the one entry that cannot go away while there
  // is a knowledge daemon. It used to anchor on `installd.sock`, which was itself
  // a stale entry - installd binds no socket, the About page was the only thing
  // that ever named one - so removing it on 15 Aug took this control down with
  // it. Anchor a control on the thing least likely to be the next defect.
  const r = run((s) =>
    s.replace('    "knowledge.sock": "arlen-graph-daemon",',
      '    "knowledge.sock": "arlen-graph-daemon",\n    "no-such-thing.sock": "arlen-graph-daemon",'),
  );
  check("a table entry for a vanished socket is caught", r.code === 1);
  check("and names it", r.out.includes("no-such-thing.sock"));
}
{
  // A socket the tree dials with no entry at all - the drift that matters most,
  // because it is what a NEW daemon looks like.
  const r = run((s) => s.replace('    "knowledge.sock": "arlen-graph-daemon",\n', ""));
  check("a referenced socket with no entry is caught", r.code === 1);
  check("and names it", r.out.includes("knowledge.sock"));
}
{
  // A carried violation that has been resolved must be dropped, not left: an entry
  // saying "known, unshipped" about something now shipped reads as coverage.
  //
  // The case SYNTHESISES its entry rather than borrowing the real one. It used to
  // mutate the table around `modulesd.sock`, the only carried entry the tree had
  // ever held - so when modulesd shipped on 15 Aug and `KNOWN` went empty, this
  // case lost its subject and went red while nothing was wrong. A control that
  // only works while a particular defect exists stops working the day it is
  // fixed, which is the wrong way round.
  const r = run((s) =>
    s.replace("KNOWN: dict[str, str] = {}", 'KNOWN = {"knowledge.sock": "carried for the control"}'),
  );
  check("a carried violation whose server now ships is caught", r.code === 1);
}

// The table's other half lives in the boot harness: CI cannot check the VALUES,
// because the server never names its socket, so the run does it - each daemon says
// what it bound. These two cases are that check's controls.
function bootFaults(mutate) {
  const py = `
import sys, json
sys.path.insert(0, ${JSON.stringify(join(ROOT, "dev/vm"))})
import verify
real = verify._socket_table()
table = ${mutate}
verify._socket_table = lambda: table
print(json.dumps(verify.socket_table_faults(sys.stdin.read())))
`;
  const serial =
    "arlen-graph-daemon[537]: INFO graph daemon listening socket=\"/run/arlen/knowledge.sock\"\n" +
    "arlen-powerd[656]: INFO power-daemon listening socket=\"/run/user/1000/arlen/power.sock\"\n";
  const r = spawnSync("python3", ["-c", py], { input: serial, encoding: "utf8", cwd: ROOT });
  if (r.status !== 0) {
    console.log(r.stderr);
    return null;
  }
  return JSON.parse(r.stdout);
}

{
  const f = bootFaults("real");
  check("a boot matching the table is clean", Array.isArray(f) && f.length === 0);
}
{
  const f = bootFaults('{**real, "knowledge.sock": "arlen-notifyd"}');
  check("a wrong server in the table is caught by the boot",
        f?.some((l) => l.includes("knowledge.sock") && l.includes("arlen-graph-daemon")));
}
{
  const f = bootFaults('{k: v for k, v in real.items() if k != "power.sock"}');
  check("a socket bound on the boot and absent from the table is caught",
        f?.some((l) => l.includes("power.sock")));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe socket table holds");
process.exit(failures ? 1 : 0);
