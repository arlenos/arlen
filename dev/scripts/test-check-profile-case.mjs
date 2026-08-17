// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-profile-case.
//
// The defect is planted with a REAL profile name: `kitty.toml` is in the corpus,
// so a `Kitty.desktop` entry is exactly the shape that bit six apps - a written,
// correct profile the launcher answers with "profile not found" because the
// desktop-id it looks up carries different case.
//
// Run: node dev/scripts/test-check-profile-case.mjs

import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const CHECK = join(ROOT, "dev/scripts/check-profile-case.sh");

const failures = [];

function run(dir) {
  const r = spawnSync("bash", [CHECK, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-profile-case:");

// The planted defect: a desktop-id whose only difference from a shipped profile
// is its case.
const bad = mkdtempSync(join(tmpdir(), "arlen-desktop-bad-"));
writeFileSync(join(bad, "Kitty.desktop"), "[Desktop Entry]\nName=Kitty\n");
const r1 = run(bad);
check(
  "a desktop id that differs only in case is reported",
  r1.code === 1 && /Kitty\.desktop is unreachable/.test(r1.out),
  `exit=${r1.code} out=${r1.out}`,
);
rmSync(bad, { recursive: true, force: true });

// The same app, named the way the profile is: nothing to report.
const good = mkdtempSync(join(tmpdir(), "arlen-desktop-good-"));
writeFileSync(join(good, "kitty.desktop"), "[Desktop Entry]\nName=kitty\n");
const r2 = run(good);
check(
  "an exact-case match is not reported",
  r2.code === 0 && /no profile is hidden/.test(r2.out),
  `exit=${r2.code} out=${r2.out}`,
);

// An app with no profile at all is a different question and not this check's.
writeFileSync(join(good, "nonesuch-xyzzy.desktop"), "[Desktop Entry]\nName=x\n");
const r3 = run(good);
check(
  "an app with no profile in any case is left alone",
  r3.code === 0,
  `exit=${r3.code} out=${r3.out}`,
);
rmSync(good, { recursive: true, force: true });

// Reading nothing is not passing. Without this the check reports clean on a
// machine with no applications, which is every CI runner.
const empty = mkdtempSync(join(tmpdir(), "arlen-desktop-empty-"));
const r4 = run(empty);
check(
  "a directory with no entries refuses rather than passing",
  r4.code === 2 && /NOTHING WAS READ/.test(r4.out),
  `exit=${r4.code} out=${r4.out}`,
);
rmSync(empty, { recursive: true, force: true });

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("case-only mismatches are found, exact names are not, and an empty read refuses");
