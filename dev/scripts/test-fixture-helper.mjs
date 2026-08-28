#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `lib/fixture.mjs`. The guard is the only thing standing between a
// control's cleanup and the repository, so it is worth watching it refuse.
//
// The refusal case runs in a CHILD process, because `cleanup` answers a path it
// did not mint by exiting - which is the point, and which would take this test
// down with it if it ran here.

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup, isMinted } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const HELPER = join(HERE, "lib", "fixture.mjs");

let failed = 0;

function check(name, ok, detail) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed += 1;
    if (detail) console.log(`     ${detail}`);
  }
}

// A minted directory is removable, and stops being minted once it is gone.
const dir = mint("fixture-control-");
writeFileSync(join(dir, "file"), "x");
check("a minted directory is recorded", isMinted(dir));
cleanup(dir);
check("cleanup removes what it minted", !existsSync(dir));
check("and forgets it afterwards, so a double cleanup is refused too", !isMinted(dir));

// The case that matters: a path this process did not create. It is a real
// directory with a real file in it, so a helper that went ahead would delete
// something - and the assertion is that the file is still there afterwards.
const outside = mkdtempSync(join(tmpdir(), "fixture-not-minted-"));
writeFileSync(join(outside, "precious"), "the repository stood here");
const child = spawnSync(
  process.execPath,
  [
    "--input-type=module",
    "-e",
    `import { cleanup } from ${JSON.stringify(HELPER)}; cleanup(${JSON.stringify(outside)});`,
  ],
  { encoding: "utf8" },
);
check(
  "a path it did not mint is refused",
  child.status === 1 && child.stderr.includes("REFUSED"),
  `exit ${child.status}: ${(child.stderr || "").trim()}`,
);
check(
  "and nothing in it is touched",
  existsSync(join(outside, "precious")),
  "the guard let a delete through, which is the whole failure this exists to stop",
);

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log("\nall 5 cases behaved");
