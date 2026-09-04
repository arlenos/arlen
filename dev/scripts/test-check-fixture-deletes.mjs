#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-fixture-deletes.py`. Every fixture here is minted through the
// shared helper the gate exists to enforce, which is the only honest way to write
// this particular control.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-fixture-deletes.py");
const REPO = join(HERE, "..", "..");

function tree(controls) {
  const root = mint("fixture-deletes-");
  const scripts = join(root, "dev", "scripts");
  mkdirSync(join(scripts, "lib"), { recursive: true });
  writeFileSync(join(scripts, "lib", "fixture.mjs"), "export function mint() {}\n");
  for (const [name, body] of Object.entries(controls)) {
    writeFileSync(join(scripts, name), body);
  }
  return root;
}

function gateOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const GUARDED = `import { mint, cleanup } from "./lib/fixture.mjs";
const d = mint("x-");
cleanup(d);
`;

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "a control that mints and cleans up passes",
    () => tree({ "test-good.mjs": GUARDED }),
    (code) => code === 0,
    true,
  ],
  [
    "a direct recursive rmSync is caught",
    () =>
      tree({
        "test-bad.mjs": 'import { rmSync } from "node:fs";\nrmSync(dir, { recursive: true, force: true });\n',
      }),
    (code, out) => code === 1 && out.includes("test-bad.mjs"),
    true,
  ],
  [
    "a delete split across lines is caught too",
    () =>
      tree({
        "test-multiline.mjs": "rmSync(root, {\n  recursive: true,\n  force: true,\n});\n",
      }),
    (code, out) => code === 1 && out.includes("test-multiline.mjs"),
    true,
  ],
  [
    "and one reached through a shell",
    // Assembled rather than written whole: `rm -rf` reaches a shell only ever as a
    // string, so the gate has to match it inside one - and a control holding the
    // literal would be reported as an offender itself.
    () => tree({ "test-shell.mjs": `execSync("rm -${"rf"} " + root);\n` }),
    (code, out) => code === 1 && out.includes("test-shell.mjs"),
    true,
  ],
  [
    // The regression that made this half of the scanner necessary. Until 4
    // September an apostrophe in an English comment opened a string as far as the
    // gate was concerned, and every delete up to the next apostrophe went unseen.
    // It hid 31 real offenders and its verdict flipped on whether a paragraph of
    // prose happened to contain an even number of apostrophes.
    "an apostrophe in a comment does not hide the delete below it",
    () =>
      tree({
        "test-apostrophe.mjs":
          // ONE apostrophe, deliberately. Two would balance out and the fixture
          // would pass under the broken scanner too, which is how the first cut
          // of this case failed to discriminate.
          "// the machine's load\nrmSync(dir, { recursive: true });\n",
      }),
    (code, out) => code === 1 && out.includes("test-apostrophe.mjs"),
    true,
  ],
  [
    // The other side of it: blanking comments must not turn prose into evidence.
    // A delete someone commented out does not delete anything.
    "a delete inside a comment is not a delete",
    () =>
      tree({
        "test-commented.mjs": "// rmSync(dir, { recursive: true, force: true });\nconst x = 1;\n",
      }),
    (code) => code === 0,
    true,
  ],
  [
    "a non-recursive remove is not a fixture delete",
    () => tree({ "test-single.mjs": 'import { rmSync } from "node:fs";\nrmSync(file);\n' }),
    (code) => code === 0,
    true,
  ],
  [
    "a file that is not a control is not read",
    () =>
      tree({
        "test-good.mjs": GUARDED,
        "helper.mjs": "rmSync(anything, { recursive: true });\n",
      }),
    (code) => code === 0,
    true,
  ],
  [
    "no controls at all is a failure rather than a pass with nothing read",
    () => mint("fixture-deletes-empty-"),
    (code, out) => code === 1 && out.includes("no control scripts"),
    true,
  ],
];

let failed = 0;
for (const [name, build, expect, disposable] of cases) {
  const root = build();
  const { code, out } = gateOn(root);
  if (disposable) cleanup(root);
  const ok = expect(code, out);
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed += 1;
    console.log(`     exit ${code}\n     ${out.trim().split("\n").slice(0, 3).join("\n     ")}`);
  }
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log(`\nall ${cases.length} cases behaved`);
