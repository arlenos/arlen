#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-system-paths-agree.mjs: plant a disagreement
// between the two copies of the System field map and watch the check fail.
//
// It matters more than usual here because the check is two regexes over two
// source files. A regex that stops matching - a formatting change, a rename -
// makes the comparison vacuous and the check PASSES while comparing nothing. So
// the empty-read case is exercised alongside the disagreement.
//
// Run: node dev/scripts/test-check-system-paths-agree.mjs

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-system-paths-agree.mjs");
const RUST = join(ROOT, "apps/settings/src-tauri/src/commands/theme.rs");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

function run() {
  return spawnSync("node", [GATE], { encoding: "utf8" });
}

console.log("check-system-paths-agree:");

check("passes on the tree as it stands", run().status === 0, run().stderr);

const original = readFileSync(RUST, "utf8");
try {
  // One path misspelled on the backend side. The store would then clear a key
  // the file does not hold, which deletes nothing and looks like a reset button
  // that works sometimes.
  writeFileSync(RUST, original.replace('"terminal.ansi.bright_white"', '"terminal.ansi.bright_whit"'));
  const drift = run();
  check(
    "a single misspelled path is caught",
    drift.status === 1 && drift.stderr.includes("ansi15"),
    drift.stderr.trim(),
  );

  // The function renamed out from under the regex: the check must say it read
  // nothing rather than report agreement between two empty maps.
  writeFileSync(RUST, original.replace("fn system_key_path", "fn system_key_route"));
  const blind = run();
  check(
    "a rename that blinds the regex fails rather than passing empty",
    blind.status !== 0,
    "an unreadable source must not read as agreement",
  );
} finally {
  writeFileSync(RUST, original);
}

check("the source is restored", readFileSync(RUST, "utf8") === original);

console.log(
  failures ? `\n${failures} case(s) failed` : "\ntwo copies of the map cannot drift, and an unreadable one cannot pass",
);
process.exit(failures ? 1 : 0);
