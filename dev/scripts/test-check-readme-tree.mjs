// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-readme-tree.
//
// The load-bearing one is the first: a directory that is ON DISK and NOT in the
// repository. That is the real defect - `docs/` is the private specs repo cloned
// into the tree - and it is why this check asks git rather than the filesystem.
// The first cut asked `Path.exists()`, and passed with the defect deliberately
// restored, because on the machine that wrote the README the directory is right
// there. A control that only planted a missing directory would have blessed it.
//
// The third is the parse: `    └── src/` sits one level deeper than `└── x/`,
// and the four spaces standing in for that level are easy to strip and forget.
//
// Run: node dev/scripts/test-check-readme-tree.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-readme-tree.py");
const failures = [];

function repo(files, { ignore = "", alsoOnDisk = [] } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-readme-"));
  for (const [rel, body] of Object.entries(files)) {
    mkdirSync(join(dir, rel, ".."), { recursive: true });
    writeFileSync(join(dir, rel), body);
  }
  if (ignore) writeFileSync(join(dir, ".gitignore"), ignore);
  spawnSync("git", ["init", "-q"], { cwd: dir });
  spawnSync("git", ["add", "-A"], { cwd: dir });
  // Untracked but present: the shape the filesystem cannot tell apart.
  for (const d of alsoOnDisk) mkdirSync(join(dir, d), { recursive: true });
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

console.log("check-readme-tree:");

// The real defect: drawn in the block, present on this machine, absent from a clone.
let d = repo(
  {
    "README.md": "# x\n\n```\nproj/\n  src/     code\n  docs/    specs\n```\n",
    "src/main.rs": "fn main() {}\n",
  },
  { ignore: "docs/\n", alsoOnDisk: ["docs"] },
);
let r = run(d);
check(
  "a directory that is on disk but not in the repo is reported",
  r.code === 1 && /docs/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Everything drawn is tracked.
d = repo({
  "README.md": "# x\n\n```\nproj/\n  src/     code\n```\n",
  "src/main.rs": "fn main() {}\n",
});
r = run(d);
check("a block whose entries are all tracked passes", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// Box-drawing, where four spaces stand in for a level. `theme.rs` lives at
// `host/src/theme.rs`; a parser that drops the indent looks for `host/theme.rs`.
d = repo({
  "README.md":
    "# x\n\n```\nproj/\n" +
    "├── src/\n" +
    "│   ├── lib/\n" +
    "└── host/\n" +
    "    └── src/\n" +
    "        └── theme.rs   reads the theme\n" +
    "```\n",
  "src/lib/a.ts": "export {};\n",
  "host/src/theme.rs": "pub fn f() {}\n",
});
r = run(d);
check(
  "a box-drawing tree nests by indent, not just by branch",
  r.code === 0,
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Reading nothing is not passing.
d = repo({ "README.md": "# x\n\nNo tree here.\n", "src/main.rs": "fn main() {}\n" });
r = run(d);
check(
  "a repo whose READMEs draw no tree refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("on-disk-but-untracked is caught, tracked entries pass, and an empty read refuses");
