// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate exists because a comment can name a file that is not there and
// nothing else notices. These cases pin what counts as a path, which is the
// whole difficulty: a first version matched any `dir/word` and produced 65
// findings that were almost all prose.
//
// Run: node dev/scripts/test-check-comment-paths.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-comment-paths.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-cpath-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-comment-paths:");

check(
  "a comment naming a file that exists passes",
  {
    "daemons/probe/src/lib.rs": "// see daemons/probe/src/other.rs for the writer\n",
    "daemons/probe/src/other.rs": "",
  },
  (code) => code === 0,
);

check(
  "a comment naming a file that moved is caught",
  { "daemons/probe/src/lib.rs": "// see apps/settings/src-tauri/src/toml_writer.rs\n" },
  (code, out) => code === 1 && out.includes("toml_writer.rs"),
);

// The reason the first version was unusable. Each of these is a sentence, not a
// path, and each one matched a `dir/word` rule.
check(
  "prose that happens to contain a slash is not a path",
  {
    "daemons/probe/src/lib.rs":
      "// stdout goes to dev/null, the apps/AI layer reads it, and\n" +
      "// forage/flatpak/apt all install differently. Device dev/A pairs with dev/B.\n",
  },
  (code) => code === 0,
);

// Ordered alternation: `ts` before `tsv` reads a .tsv path as a broken .ts one.
// It did exactly that against a path that had just been corrected.
check(
  "a .tsv path is not read as a missing .ts one",
  {
    "daemons/probe/src/lib.rs": "// the baseline lives at dev/i18n-baseline.tsv\n",
    "dev/i18n-baseline.tsv": "",
  },
  (code) => code === 0,
);

// A package specifier is not a repo path. `-` counts as a word boundary, so a
// `\b`-anchored pattern reads the tail of `@arlen/module-sdk/postmsg.ts` as a
// claim about `sdk/postmsg.ts` - which is a file that does not exist, so the gate
// would have reported a defect in a correct line.
check(
  "a package name ending in a top-level dir is not a path",
  {
    "daemons/probe/src/lib.rs":
      "// mirrors `@arlen/module-sdk/postmsg.ts` and `module-sdk/package.json`\n",
  },
  (code) => code === 0,
);

// Code is not prose: a path in a string literal is the program's business, and
// checking it here would fail on every runtime path the tree constructs.
check(
  "a path in code rather than a comment is left alone",
  { "daemons/probe/src/lib.rs": 'let p = "daemons/probe/src/gone.rs";\n' },
  (code) => code === 0,
);

check(
  "a known outside-the-tree path is excused",
  { "daemons/probe/src/lib.rs": "// reads forage/recipe.toml from the user's project\n" },
  (code) => code === 0,
);

// It read only Rust at first - 27 paths, while the same kind of note in a gate
// script, a build step or a Svelte store went unchecked. Those are four fifths of
// the references in the tree.
check(
  "a stale path in a script comment is caught too",
  { "dev/scripts/probe.py": "# see daemons/probe/src/gone.rs for the writer\n" },
  (code, out) => code === 1 && out.includes("gone.rs"),
);

check(
  "and a correct one in a script comment passes",
  {
    "dev/scripts/probe.py": "# see daemons/probe/src/lib.rs for the writer\n",
    "daemons/probe/src/lib.rs": "",
  },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all comment-path cases passed");
