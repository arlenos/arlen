// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the commit-msg hook: a subject that fits passes, one over 50
// fails, a missing area fails, too few or too many words fail, a body fails, and
// git's own wordings (merge, revert, fixup) are left alone.
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const HOOK = join(ROOT, ".githooks/commit-msg");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

function run(message) {
  const dir = mint("commit-msg-");
  const file = join(dir, "MSG");
  writeFileSync(file, message);
  try {
    execFileSync(HOOK, [file], { encoding: "utf8" });
    cleanup(dir);
    return 0;
  } catch (e) {
    cleanup(dir);
    return e.status ?? 1;
  }
}

const cases = [
  ["a subject that fits passes", "theme: declare a target", 0],
  // Exactly at the limit, with a legal word count: the boundary is inclusive.
  ["exactly fifty characters passes", "knowledge: delete the memberships a range recorded", 0],
  [
    "a sentence of a subject is refused",
    "ai-engine-daemon: take the chosen action mode and stop granting the engine its own autonomy",
    1,
  ],
  ["a subject with no area is refused", "declare a theme target now", 1],
  ["two words after the colon is refused", "theme: declare targets", 1],
  ["seven words after the colon is refused", "a: one two three four five six seven", 1],
  ["a body is refused", "theme: declare a target\n\nAnd here is why.", 1],
  ["a merge is git's wording", "Merge branch 'main' into a-very-long-branch-name-indeed", 0],
  ["a revert is git's wording", "Revert \"theme: declare a target that was wrong all along\"", 0],
  ["a fixup is git's wording", "fixup! theme: declare a target with a long tail on it", 0],
];

for (const [name, message, want] of cases) {
  const got = run(message);
  got === want ? ok(name) : bad(name, `expected exit ${want}, got ${got}`);
}

// The comment lines git appends must not be read as a body.
{
  const got = run("theme: declare a target\n\n# Please enter the commit message\n# with '#' ignored.\n");
  got === 0 ? ok("git's own comment lines are not a body") : bad("git's own comment lines are not a body", `exit ${got}`);
}

console.log(failures === 0 ? "all commit-msg cases passed" : `${failures} case(s) failed`);
process.exit(failures === 0 ? 0 : 1);
