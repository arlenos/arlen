// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-tests-run.
//
// Two rungs, two planted defects. The component one is the state
// `apps/desktop-shell` was in for months: test
// files present, no script, CI green and silent about it. The near-misses matter
// as much - a component with no tests must not be nagged (several are thin enough
// that tests would be ceremony), and a component using a runner other than vitest
// must be accepted, which the first draft of the gate got wrong.
//
// The Rust one is `apps/files/core` on 15 September: a function in `mod tests`,
// full of assertions, with no `#[test]`, between two neighbours that had it. Its
// near-misses are the ones that make the rung usable rather than noisy - a helper
// that asserts but IS called, and a test that carries a harness attribute rather
// than the plain one.
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

// --- the Rust rung -------------------------------------------------------

function rustTree(body) {
  const dir = mint("arlen-testsrun-rs-");
  mkdirSync(join(dir, "crate/src"), { recursive: true });
  writeFileSync(join(dir, "crate/src/lib.rs"), body);
  return dir;
}

const ORPHAN = `pub fn thing() -> u8 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_that_runs() {
        assert_eq!(thing(), 1);
    }

    fn an_assertion_nobody_runs() {
        assert_eq!(thing(), 1, "this has never been checked");
    }
}
`;

d = rustTree(ORPHAN);
r = run(d);
check(
  "an assertion with no attribute is caught",
  r.code === 1 && /an_assertion_nobody_runs/.test(r.out) && !/a_test_that_runs/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

// A helper asserts too. Being called is what makes it a helper, and it is the
// only signal that separates the two without reading intent.
d = rustTree(`pub fn thing() -> u8 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    fn a_called_helper(n: u8) -> u8 {
        assert!(n > 0, "the helper guards its own input");
        n
    }

    #[test]
    fn uses_the_helper() {
        assert_eq!(a_called_helper(thing()), 1);
    }
}
`);
r = run(d);
check("a helper that asserts but is called is left alone", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

// `#[tokio::test(flavor = "multi_thread")]` runs the function as surely as
// `#[test]` does. A gate that only knows the plain spelling reports every async
// test in the tree.
d = rustTree(`pub fn thing() -> u8 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn an_async_test_runs() {
        assert_eq!(thing(), 1);
    }
}
`);
r = run(d);
check("a harness attribute counts as running it", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

// A function outside a test module is ordinary code, whatever it asserts.
d = rustTree(`pub fn guarded(n: u8) -> u8 {
    assert!(n > 0, "a debug assertion in shipping code is not a test");
    n
}
`);
r = run(d);
check("an assertion in shipping code is not a test", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

// --- the ignore rung --------------------------------------------------------

d = rustTree(`pub fn thing() -> u8 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn a_test_that_does_not_say_why_it_is_off() {
        assert_eq!(thing(), 1);
    }
}
`);
r = run(d);
check(
  "an ignored test with no reason is caught",
  r.code === 1 && /a_test_that_does_not_say_why_it_is_off/.test(r.out) && /does not say why/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

// The reason is the whole point, and it is what sixty tests in the tree already
// write. A test that gives one is waiting for a machine, not switched off.
d = rustTree(`pub fn thing() -> u8 { 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a pty"]
    fn a_test_that_says_why() {
        assert_eq!(thing(), 1);
    }
}
`);
r = run(d);
check("an ignored test that says why passes", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = mint("arlen-testsrun-empty-");
r = run(d);
check(
  "a tree with neither tests nor Rust refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("at all three rungs a test that does not run is caught, and helpers are left alone");
