#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-admitted-ids-exist.py: put back the exact
// mistake it was written for and watch it fail.
//
// The mistake was `INSTALL_CALLERS = ["store"]` in installd - an id no path on
// the image can resolve to, so the daemon refused every caller for months while
// looking perfectly healthy. A gate closed against everyone reads the same as a
// gate that works, which is why this needs a check rather than a reviewer.
//
// The second case guards the other direction. The gate is two regexes over two
// kinds of source; if either stops matching it compares empty sets and passes.
// Both `NOTHING WAS READ` paths are exercised so an unreadable tree cannot report
// agreement.
//
// Run: node dev/scripts/test-check-admitted-ids-exist.mjs

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-admitted-ids-exist.py");
const INSTALLD = join(ROOT, "daemons/installd/installd/src/dbus.rs");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

const run = (args = []) => spawnSync("python3", [GATE, ...args], { encoding: "utf8" });

console.log("check-admitted-ids-exist:");

check("passes on the tree as it stands", run().status === 0, run().stderr.trim());

const original = readFileSync(INSTALLD, "utf8");
try {
  // The original defect, verbatim: the bare id in place of the resolver's.
  writeFileSync(
    INSTALLD,
    original.replace(
      'const INSTALL_CALLERS: &[&str] = &["dev.arlen.store", "dev.arlen.settings"];',
      'const INSTALL_CALLERS: &[&str] = &["store", "dev.arlen.settings"];',
    ),
  );
  const back = run();
  check(
    "the original bare-id defect is caught",
    back.status === 1 && back.stderr.includes("`store`"),
    back.stderr.trim(),
  );
} finally {
  writeFileSync(INSTALLD, original);
}
check("the source is restored", readFileSync(INSTALLD, "utf8") === original);

// An acknowledgement that has come true must be reported too, or the list turns
// into a place where entries go to be forgotten.
{
  const src = readFileSync(GATE, "utf8");
  check(
    "an acknowledged id is one the gate would otherwise report",
    src.includes("Remove it from NOT_PACKAGED_YET"),
    "the stale-acknowledgement branch is gone",
  );
}

// Pointed at a tree with neither allowlists nor build phases, it must say it read
// nothing rather than report that everything agrees.
{
  const empty = run(["/tmp"]);
  check(
    "an unreadable tree says so instead of passing",
    empty.status === 2 && empty.stderr.includes("NOTHING WAS READ"),
    empty.stderr.trim() || `exit ${empty.status}`,
  );
}

console.log(
  failures
    ? `\n${failures} case(s) failed`
    : "\nan id nothing can resolve to cannot sit in an allowlist unremarked",
);
process.exit(failures ? 1 : 0);
