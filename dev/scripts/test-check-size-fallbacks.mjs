#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-size-fallbacks.py. The real case is from 9 September: all
// forty height fallbacks in the kit were exactly 2px under their token, which is
// what a raised height register leaves behind. The check passes against the repo
// now because they were moved, which is when a check needs proving on the state
// it was written for.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-size-fallbacks.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

function tree(component) {
  const dir = mint("arlen-sizefb-");
  mkdirSync(join(dir, "sdk/ui-kit/src/lib/components/ui/demo"), { recursive: true });
  writeFileSync(
    join(dir, "sdk/ui-kit/src/app.css"),
    ":root {\n  --height-control:             30px;\n  --height-tag:                 20px;\n}\n",
  );
  writeFileSync(join(dir, "sdk/ui-kit/src/lib/components/ui/demo/demo.svelte"), component);
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

// THE REAL CASE: two under the token.
let dir = tree("<style>\n  .b { height: var(--height-control, 28px); }\n</style>\n");
let r = run(dir);
r.code === 1 && r.out.includes("28px")
  ? ok("a fallback under its token fails")
  : bad("a fallback under its token fails", `code ${r.code}: ${r.out}`);
cleanup(dir);

dir = tree("<style>\n  .b { height: var(--height-control, 30px); }\n</style>\n");
r = run(dir);
r.code === 0
  ? ok("and passes once it is the token")
  : bad("and passes once it is the token", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A fallback for a variable the stylesheet does not declare is not this check's
// question - it may be somebody else's token entirely.
dir = tree("<style>\n  .b { height: var(--height-mystery, 12px); }\n</style>\n");
r = run(dir);
r.code === 0
  ? ok("an unknown variable is left alone")
  : bad("an unknown variable is left alone", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A use with no fallback at all is fine: the token is always there in an app.
dir = tree("<style>\n  .b { height: var(--height-control); }\n</style>\n");
r = run(dir);
r.code === 0
  ? ok("a use with no fallback passes")
  : bad("a use with no fallback passes", `code ${r.code}: ${r.out}`);
cleanup(dir);

// An empty tree must refuse rather than pass.
dir = mint("arlen-sizefb-");
r = run(dir);
r.code === 1
  ? ok("an empty tree is a failure, not a pass")
  : bad("an empty tree is a failure, not a pass", `code ${r.code}: ${r.out}`);
cleanup(dir);

try {
  execFileSync("python3", [check], { encoding: "utf8" });
  ok("and the repo itself passes");
} catch (e) {
  bad("and the repo itself passes", String(e.stdout ?? e));
}

console.log(
  failures === 0
    ? "a fallback and its token say the same number, and the check says so"
    : `\n${failures} failure(s)`,
);
process.exit(failures === 0 ? 0 : 1);
