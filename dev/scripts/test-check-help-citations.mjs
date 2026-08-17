// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-help-citations.
//
// Both real shapes are planted: a clap `///` (how `forage --help` printed four
// citations to a private repo) and a hand-written usage const (how `trash-rm`
// builds its help).
//
// The negative cases are the ones that matter here. This check was wrong three
// times before it was right, and every time it was a FALSE POSITIVE: a theme
// token named `spacing.md` (md as in medium), a `//` comment in an unrelated
// daemon, and test code writing "app/README.md" after a one-line const opened a
// usage region that never closed. So there is a case for each.
//
// Run: node dev/scripts/test-check-help-citations.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-help-citations.py");
const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-help-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(join(p, ".."), { recursive: true });
    writeFileSync(p, body);
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

console.log("check-help-citations:");

// Shape 1: a clap doc comment, which becomes --help output.
let d = tree({
  "src/cli.rs": `#[derive(Subcommand)]\nenum Cmd {\n    /// Challenge a build (forage-recipes.md section 8a).\n    Challenge,\n}\n`,
});
let r = run(d);
check(
  "a clap doc comment citing a doc is reported",
  r.code === 1 && /forage-recipes\.md/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Shape 2: a hand-written usage const.
d = tree({
  "src/main.rs": `const USAGE: &str = "\\\nUsage: tool [OPTION]...\n\n  --purge   see design-notes.md for why\n";\n`,
});
r = run(d);
check(
  "a hand-written usage string citing a doc is reported",
  r.code === 1 && /design-notes\.md/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Negative 1: the design reference on a `//` line, which clap does not print.
d = tree({
  "src/cli.rs": `#[derive(Subcommand)]\nenum Cmd {\n    // Design: forage-recipes.md section 8a.\n    /// Rebuild a package and check it is byte-identical.\n    Challenge,\n}\n`,
});
r = run(d);
check("a `//` design reference beside a clean `///` is fine", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// Negative 2: a one-line const must not open a region that swallows the file.
// This is the third false positive, exactly as it happened.
d = tree({
  "src/main.rs":
    `const USAGE: &str = "\\\nUsage: tool [FILE]...\n";\n` +
    `const DEFAULT_BASELINE: &str = "dev/baseline.tsv";\n` +
    `fn t() { write("app/README.md", "x"); }\n`,
});
r = run(d);
check(
  "code after a one-line const is not read as help",
  r.code === 0 && !/README/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Negative 3: `.md` that is not a filename at all.
d = tree({
  "src/main.rs":
    `const USAGE: &str = "\\\nUsage: tool [FILE]...\n";\n` +
    `fn t() { m.insert("spacing.md".into(), tokens.spacing.md.clone()); }\n`,
});
r = run(d);
check(
  "a token named `spacing.md` is not a citation",
  r.code === 0 && !/spacing/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Reading nothing is not passing.
d = tree({ "src/lib.rs": "pub fn f() {}\n" });
r = run(d);
check(
  "a tree with no help at all refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("both help shapes are checked, and the three false positives stay false");
