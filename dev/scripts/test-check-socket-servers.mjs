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
  const r = run((s) =>
    s.replace('    "installd.sock": "installd",',
      '    "installd.sock": "installd",\n    "no-such-thing.sock": "arlen-graph-daemon",'),
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
  const r = run((s) =>
    s.replace('    "modulesd.sock": "arlen-modulesd",', '    "modulesd.sock": "arlen-graph-daemon",'),
  );
  check("a carried violation whose server now ships is caught", r.code === 1);
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe socket table holds");
process.exit(failures ? 1 : 0);
