// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for the frontend type-check step in ci.yml.
//
// That step ran `npm run check || true` from the day it was written until 17
// August, which made it a check that could not fail. When it was finally
// measured it was holding two real defects (a test importing `node:fs` with no
// @types/node, and a deprecated `<slot>` in the shell's root layout). The
// swallowing is the thing worth guarding: it is one character to put back, it
// looks tidy in a diff, and nothing else in the tree would notice.
//
// The exception list is guarded too. A list that matches loosely silences
// packages nobody meant to silence, which is the same failure wearing a
// different hat - so `sdk/ui` and a bare `ui-kit` must NOT match `sdk/ui-kit`.
//
// The shell logic is READ OUT OF ci.yml and executed, never copied here. A copy
// would pass forever while the workflow drifted underneath it.
//
// Run: node dev/scripts/test-typecheck-gating.mjs

import { readFileSync, writeFileSync, rmSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const CI = join(ROOT, ".github/workflows/ci.yml");

const yml = readFileSync(CI, "utf8");
const failures = [];

function check(name, ok, detail = "") {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("typecheck gating:");

// 1. The blanket swallow must not come back.
check(
  "the frontend check is not swallowed by a blanket `|| true`",
  !/npm run check \|\| true/.test(yml),
  "found `npm run check || true` in ci.yml",
);

// 2. The gating branch has to exist at all: a step that only ever runs the
//    advisory branch would satisfy the rule above and still gate nothing.
check(
  "a branch runs `npm run check` with nothing catching its failure",
  /\n\s*npm run check\s*\n/.test(yml),
  "no bare `npm run check` line, so nothing can fail",
);

// 3. Run the workflow's OWN matching logic, with a stubbed failing check.
const block = yml.match(
  /(TYPECHECK_ADVISORY=[\s\S]*?\n\s*if printf[\s\S]*?\n\s*fi)\n/,
);
check("the advisory-list logic can be found in ci.yml", Boolean(block));

if (block) {
  // Strip the YAML block indentation so it is a runnable script.
  const body = block[1]
    .split("\n")
    .map((l) => l.replace(/^\s{0,12}/, ""))
    .join("\n")
    // The workflow reads the package from the job matrix; the test supplies it.
    .replace(/APP="\$\{\{ matrix\.app \}\}"/, 'APP="$1"')
    // The real list is empty (and should stay so unless a reason is written
    // beside a new entry). The machinery still has to spare a listed package
    // the day one is listed, so the test injects a sample entry and exercises
    // the branch with it rather than depending on the tree carrying a defect.
    .replace(/TYPECHECK_ADVISORY="[^"]*"/, 'TYPECHECK_ADVISORY="sdk/ui-kit"');

  const dir = mkdtempSync(join(tmpdir(), "arlen-typecheck-"));
  const script = join(dir, "step.sh");
  // `set -e` because that is how GitHub runs a `run:` block, and it is the
  // reason the gating branch stops the step rather than merely printing.
  // `npm` is stubbed to fail, so each branch is exercised at its failure.
  writeFileSync(script, `set -e\nnpm() { return 1; }\n${body}\necho REACHED_END\n`);

  const run = (app) => spawnSync("bash", [script, app], { encoding: "utf8" });

  const listed = run("sdk/ui-kit");
  check(
    "a listed package survives a failing check",
    listed.status === 0 && /advisory \(listed\)/.test(listed.stdout),
    `exit=${listed.status} out=${listed.stdout}`,
  );

  const gated = run("apps/files");
  check(
    "an unlisted package fails the step",
    gated.status !== 0 && !/REACHED_END/.test(gated.stdout),
    `exit=${gated.status} out=${gated.stdout}`,
  );

  // The near-misses. A substring or suffix match here would quietly silence a
  // package that was never on the list.
  for (const near of ["sdk/ui", "ui-kit", "apps/store"]) {
    const r = run(near);
    check(`"${near}" does not match the listed "sdk/ui-kit"`, r.status !== 0, `exit=${r.status}`);
  }

  rmSync(dir, { recursive: true, force: true });
}

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("the type check can fail, and only the listed package is spared");
