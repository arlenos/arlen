// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The gate flags a test file that states nothing about the answer. The case it
// exists for is the quiet one: an agent writes the file, the imports and the
// call reliably, and the line saying what the result SHOULD be unreliably.
//
// Run: node dev/scripts/test-check-test-asserts.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-test-asserts.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, why) => {
  console.log(`  FAIL ${n}\n       ${why}`);
  failures++;
};

function tree(files) {
  const dir = mint("test-asserts-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [GATE, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout || "") + (e.stderr || "") };
  }
}

// Every case needs at least one PASSING test file too, or the gate reports the
// empty tree instead of the finding and the case proves nothing.
const GOOD_RS =
  "#[cfg(test)]\nmod t {\n    #[test]\n    fn it_holds() {\n        assert_eq!(1 + 1, 2);\n    }\n}\n";

console.log("check-test-asserts:");

{
  const d = tree({
    "crate/src/good.rs": GOOD_RS,
    "crate/src/hollow.rs":
      "#[cfg(test)]\nmod t {\n    #[test]\n    fn it_runs() {\n        compute(3);\n    }\n}\n",
  });
  const r = run(d);
  if (r.code === 1 && r.out.includes("hollow.rs")) ok("a rust test that asserts nothing is caught");
  else bad("a rust test that asserts nothing is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // `#[should_panic]` is an oracle without an `assert!`: the claim is that the
  // body panics, and a pattern reading only `assert` would call it hollow.
  const d = tree({
    "crate/src/panics.rs":
      "#[cfg(test)]\nmod t {\n    #[test]\n    #[should_panic]\n    fn it_refuses() {\n        parse(\"\");\n    }\n}\n",
  });
  const r = run(d);
  if (r.code === 0) ok("should_panic counts as the oracle it is");
  else bad("should_panic counts as the oracle it is", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = tree({
    "crate/src/good.rs": GOOD_RS,
    "app/src/lib/thing.test.ts": 'it("does a thing", () => {\n  doThing();\n});\n',
  });
  const r = run(d);
  if (r.code === 1 && r.out.includes("thing.test.ts")) ok("a vitest file with no expect is caught");
  else bad("a vitest file with no expect is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = tree({
    "app/src/lib/thing.test.ts":
      'it("does a thing", () => {\n  expect(doThing()).toBe(2);\n});\n',
  });
  const r = run(d);
  if (r.code === 0) ok("the same file with an expect passes");
  else bad("the same file with an expect passes", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // The half that took a measurement to get right. A control's verdict is its
  // exit code, and its helper is named differently in about half of the 156 in
  // this tree - so the pattern is the failing exit, not the helper's name.
  const d = tree({
    "crate/src/good.rs": GOOD_RS,
    "dev/scripts/test-check-thing.mjs":
      "const r = run();\nconsole.log(r);\nprocess.exit(0);\n",
  });
  const r = run(d);
  if (r.code === 1 && r.out.includes("test-check-thing.mjs"))
    ok("a control that can only exit zero is caught");
  else bad("a control that can only exit zero is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = tree({
    "dev/scripts/test-check-thing.mjs":
      "let failures = 0;\nif (!run()) failures++;\nprocess.exit(failures ? 1 : 0);\n",
  });
  const r = run(d);
  if (r.code === 0) ok("a control with a failing exit path passes, whatever its helper is called");
  else bad("a control with a failing exit path passes, whatever its helper is called", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // A source file with no tests in it is not a test file, or every crate in the
  // tree becomes a finding.
  const d = tree({
    "crate/src/good.rs": GOOD_RS,
    "crate/src/plain.rs": "pub fn compute(n: u32) -> u32 {\n    n + 1\n}\n",
  });
  const r = run(d);
  if (r.code === 0) ok("a file with no test in it is not read as one");
  else bad("a file with no test in it is not read as one", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = mint("test-asserts-empty-");
  const r = run(d);
  if (r.code === 2) ok("a tree with no test file at all is an error, not a pass");
  else bad("a tree with no test file at all is an error, not a pass", `exit ${r.code}`);
  cleanup(d);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
