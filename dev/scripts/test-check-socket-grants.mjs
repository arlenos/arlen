// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the socket-grant table.
//
// The gate compares two hand-kept lists, so what has to be shown is that each
// way they can come apart is refused: a served socket the launcher has no row
// for, a row for a socket nothing serves, and a socket quietly opened to every
// app. Each case runs the real gate against a changed copy of the real table.

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-socket-grants.py");
const TABLE = readFileSync(join(ROOT, "daemons/arlen-run/src/sockets.rs"), "utf8");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** Run the gate against a changed copy of the launcher's table. */
function run(mutate) {
  const dir = mint("socket-grants-");
  const path = join(dir, "sockets.rs");
  const changed = mutate(TABLE);
  writeFileSync(path, changed);
  const r = spawnSync("python3", [GATE, ROOT, path], { encoding: "utf8", cwd: ROOT });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}`, same: changed === TABLE };
}

console.log("socket grants:");

{
  const r = run((s) => s);
  check("the tree as it stands passes", r.code === 0);
  check("and says how many sockets it covered", /OK: \d+ sockets/.test(r.out));
}
{
  const r = run((s) => s.replace('    ("knowledge.sock", Grant::Graph),\n', ""));
  check("the mutation landed", !r.same);
  check("a served socket with no row is caught", r.code === 1);
  check("and names it", r.out.includes("knowledge.sock: in the inventory"));
}
{
  const r = run((s) =>
    s.replace('    ("knowledge.sock", Grant::Graph),', '    ("knowledge.sock", Grant::Graph),\n    ("no-such-thing.sock", Grant::Nobody),'),
  );
  check("a row for a socket nothing serves is caught", r.code === 1);
  check("and names it", r.out.includes("no-such-thing.sock"));
}
{
  const r = run((s) => s.replace('("audit-read.sock", Grant::Nobody)', '("audit-read.sock", Grant::Everyone)'));
  check("the mutation landed", !r.same);
  check("a socket opened to every app is caught", r.code === 1);
  check("and names it", r.out.includes("audit-read.sock: open to every app"));
}
{
  const r = run((s) => s.replace('("launch.sock", Grant::Everyone)', '("launch.sock", Grant::Nobody)'));
  check("a listed socket the table closes is caught", r.code === 1);
}
{
  const r = run((s) => s.replace("pub const SOCKETS", "pub const TABLE"));
  check("a table that cannot be found is not a pass", r.code !== 0);
}

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall controls hold");
