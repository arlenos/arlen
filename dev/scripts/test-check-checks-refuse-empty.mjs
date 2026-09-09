#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-checks-refuse-empty.py. The real measurement is from
// 10 September: 27 of 152 checks called an empty tree clean. The check passes
// against the repo because those 27 are carried; a twenty-eighth fails.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, chmodSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-checks-refuse-empty.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// A check that refuses an empty tree, and one that shrugs.
const REFUSES = `#!/usr/bin/env python3
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
files = list((root / "apps").rglob("*.svelte"))
if not files:
    print("nothing to read, so the scan is pointed wrong")
    sys.exit(1)
print("ok")
`;
const SHRUGS = `#!/usr/bin/env python3
import sys
from pathlib import Path
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".")
print(f"{len(list((root / 'apps').rglob('*.svelte')))} file(s) read")
sys.exit(0)
`;

function tree(scripts) {
  const dir = mint("arlen-vacuity-");
  const d = join(dir, "dev/scripts");
  mkdirSync(d, { recursive: true });
  for (const [name, body] of Object.entries(scripts)) {
    const p = join(d, name);
    writeFileSync(p, body);
    chmodSync(p, 0o755);
  }
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [check, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status, out: String(e.stdout ?? "") };
  }
}

// THE REAL CASE: a check that calls an empty tree clean and is not carried.
let dir = tree({ "check-shrugs.py": SHRUGS });
let r = run(dir);
r.code === 1 && r.out.includes("check-shrugs.py")
  ? ok("a check that calls an empty tree clean fails")
  : bad("a check that calls an empty tree clean fails", `code ${r.code}: ${r.out}`);
cleanup(dir);

dir = tree({ "check-refuses.py": REFUSES });
r = run(dir);
r.code === 0
  ? ok("and one that refuses it passes")
  : bad("and one that refuses it passes", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A tree with no checks at all is a scan pointed wrong, not a clean answer -
// the same rule this check asks of everybody else.
dir = mint("arlen-vacuity-");
mkdirSync(join(dir, "dev/scripts"), { recursive: true });
r = run(dir);
r.code === 1
  ? ok("no checks at all is a refusal, not a pass")
  : bad("no checks at all is a refusal, not a pass", `code ${r.code}: ${r.out}`);
cleanup(dir);

// And the repo itself, where the 27 are carried.
try {
  execFileSync("python3", [check], { encoding: "utf8" });
  ok("and the repo itself passes");
} catch (e) {
  bad("and the repo itself passes", String(e.stdout ?? e));
}

console.log(
  failures === 0
    ? "a check that read nothing says so, and the meta-check says so"
    : `\n${failures} failure(s)`,
);
process.exit(failures === 0 ? 0 : 1);
