// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-tests-run.
//
// The planted defect is the state `apps/desktop-shell` was in for months: test
// files present, no script, CI green and silent about it. The near-misses matter
// as much - a component with no tests must not be nagged (several are thin enough
// that tests would be ceremony), and a component using a runner other than vitest
// must be accepted, which the first draft of the gate got wrong.
//
// Run: node dev/scripts/test-check-tests-run.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-tests-run.py");
const failures = [];

function tree(components) {
  const dir = mint("arlen-testsrun-");
  for (const [path, { script, tests }] of Object.entries(components)) {
    mkdirSync(join(dir, path, "src/lib"), { recursive: true });
    const pkg = { name: "x", scripts: script ? { test: script } : {} };
    writeFileSync(join(dir, path, "package.json"), JSON.stringify(pkg));
    for (const t of tests) writeFileSync(join(dir, path, "src/lib", t), "");
  }
  return dir;
}

const run = (dir) => {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-tests-run:");

let d = tree({ "apps/shell": { script: "vitest run", tests: ["a.test.ts"] } });
let r = run(d);
check("a component that runs its tests passes", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = tree({
  "apps/shell": { script: null, tests: ["a.test.ts", "b.test.ts"] },
  "apps/other": { script: "vitest run", tests: ["c.test.ts"] },
});
r = run(d);
check(
  "tests with no script are reported",
  r.code === 1 && /apps\/shell/.test(r.out) && !/apps\/other/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

// Any runner counts: `ai/pi-plugins` uses `node --test` on purpose and CI runs it.
// Requiring vitest here would demand a change nobody needed.
d = tree({
  "apps/node-ish": { script: "tsc && node --test dist/*.test.js", tests: ["a.test.ts"] },
});
r = run(d);
check("a component using another runner is accepted", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = tree({
  "apps/thin": { script: null, tests: [] },
  "apps/other": { script: "vitest run", tests: ["c.test.ts"] },
});
r = run(d);
check("a component with no tests is not nagged", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = mint("arlen-testsrun-empty-");
r = run(d);
check(
  "a tree with no tests at all refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("a directory of tests nobody runs is caught, and a thin component is left alone");
