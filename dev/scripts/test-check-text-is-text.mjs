#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-text-is-text.py`. The NUL is written here from a character
// code rather than typed, because a control the shell cannot echo and a reviewer
// cannot see is worse than the defect it guards.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-text-is-text.py");
const REPO = join(HERE, "..", "..");
const NUL = String.fromCharCode(0);

function tree(files) {
  const root = mint("text-is-text-");
  const dir = join(root, "src");
  mkdirSync(dir, { recursive: true });
  for (const [name, body] of Object.entries(files)) writeFileSync(join(dir, name), body);
  return root;
}

function gateOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "a NUL in a TypeScript file is caught",
    () => tree({ "key.ts": `const k = \`a${NUL}b\`;\n` }),
    (code, out) => code === 1 && out.includes("key.ts"),
    true,
  ],
  [
    "and the line it names is the line it is on",
    () => tree({ "key.ts": `// one\n// two\nconst k = \`a${NUL}b\`;\n` }),
    (code, out) => code === 1 && out.includes("key.ts:3"),
    true,
  ],
  [
    "the same separator written as an escape passes",
    () => tree({ "key.ts": "const k = `a\\u0000b`;\n" }),
    (code) => code === 0,
    true,
  ],
  [
    "a NUL in a Rust file is caught too",
    () => tree({ "key.rs": `let k = format!("a${NUL}b");\n` }),
    (code, out) => code === 1 && out.includes("key.rs"),
    true,
  ],
  [
    "a binary file under a non-source extension is not the subject",
    () => tree({ "shot.png": `PNG${NUL}${NUL}data` }),
    (code) => code === 0,
    true,
  ],
];

let failed = 0;
for (const [name, build, ok, temp] of cases) {
  const root = build();
  const { code, out } = gateOn(root);
  if (temp) cleanup(root);
  if (ok(code, out)) {
    console.log(`  ok   ${name}`);
  } else {
    console.log(`  FAIL ${name}`);
    console.log(`       exit ${code}: ${out.trim()}`);
    failed = 1;
  }
}
process.exit(failed);
