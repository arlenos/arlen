#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-spawned-tools-classified.py. The fault staged is the one that
// matters: a new surface shells out to a tool nobody has thought about, which is
// how twenty-three of them got into the tree unnoticed in the first place.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-spawned-tools-classified.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(src) {
  const root = mint("spawned-");
  mkdirSync(join(root, "apps/thing/src"), { recursive: true });
  writeFileSync(join(root, "apps/thing/src/lib.rs"), src);
  // The classification is READ from here now rather than kept in the check, so a
  // fixture has to carry one. Two rows are enough for every case below: one the
  // image has and one it does not.
  mkdirSync(join(root, "dev/scripts"), { recursive: true });
  writeFileSync(
    join(root, "dev/scripts/runtime-deps.tsv"),
    [
      "# tool\tpackage\tcomponent\tstate\tnote",
      "systemctl\tsystemd\tshell\tbase\tstarting and stopping units",
      "sh\tdash\tshell\tbase\trunning a desktop entry's Exec",
      "nmcli\tnetwork-manager\tshell\tabsent\tthe whole network popover",
    ].join("\n") + "\n",
  );
  return root;
}

const run = (root) => {
  try { execFileSync("python3", [check, root], { encoding: "utf8" }); return 0; }
  catch (e) { return e.status ?? 1; }
};

{
  // A tool in neither list. This is the case the check exists for.
  const root = tree('fn f() { Command::new("brand-new-tool").spawn(); }\n');
  const rc = run(root);
  rc === 1 ? ok("an unclassified tool is caught") : bad("an unclassified tool is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree('fn f() { Command::new("nmcli").spawn(); }\n');
  const rc = run(root);
  rc === 0 ? ok("a tool classified as absent passes") : bad("a tool classified as absent passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  const root = tree('fn f() { Command::new("systemctl").spawn(); }\n');
  const rc = run(root);
  rc === 0 ? ok("a tool classified as shipped passes") : bad("a tool classified as shipped passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // STALENESS IS NOT ASSERTED HERE, and the reason is a property of the check
  // rather than a gap in the control. Over an arbitrary root, "this entry is
  // dead" and "this fixture only spawns one thing" are the same observation, so
  // the check deliberately says nothing about staleness unless it is scanning
  // the whole repository. What IS asserted is that silence: a fixture spawning
  // one classified tool must pass rather than reporting the other 37 as dead.
  // The shrink direction is exercised by the repository case below - if an entry
  // here outlives its spawn site, that case goes red.
  const root = tree('fn f() { Command::new("sh").spawn(); }\n');
  const rc = run(root);
  rc === 0
    ? ok("a small tree is not accused of having killed every other tool")
    : bad("a small tree is not accused of having killed every other tool", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // Reading nothing must not read as a pass.
  const root = mint("spawned-empty-");
  const rc = run(root);
  rc === 2 ? ok("finding no spawn sites at all is not a pass") : bad("finding no spawn sites at all is not a pass", `expected 2, got ${rc}`);
  cleanup(root);
}

{
  const rc = run(join(here, "..", ".."));
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `got ${rc}`);
}

console.log(
  failures === 0
    ? "a tool the tree shells out to cannot go unclassified"
    : `\n${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
