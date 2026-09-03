// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control. The defect it catches is quiet by nature: nothing is
// wrong until an image build runs, and then the damage is thousands of untracked
// files that look like somebody's mistake rather than a build's.
//
// Run: node dev/scripts/test-check-node-artifacts-ignored.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-node-artifacts-ignored.py");

const failures = [];

function check(name, build, expect) {
  const dir = mint("arlen-nodeign-");
  spawnSync("git", ["init", "-q", dir]);
  build(dir);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

const pkg = (dir, rel, gitignore) => {
  mkdirSync(join(dir, rel), { recursive: true });
  writeFileSync(join(dir, rel, "package.json"), '{"name":"demo"}\n');
  if (gitignore !== null) writeFileSync(join(dir, rel, ".gitignore"), gitignore);
};

console.log("check-node-artifacts-ignored positive control");

check(
  "a package whose node_modules nothing ignores is caught",
  (dir) => pkg(dir, "ai/demo", null),
  (code, out) => code === 1 && out.includes("ai/demo"),
);

check(
  "the same package with its own .gitignore passes",
  (dir) => pkg(dir, "ai/demo", "node_modules\n"),
  (code) => code === 0,
);

check(
  "a rule written `node_modules/` - directories only - still passes",
  (dir) => pkg(dir, "ai/demo", "node_modules/\n"),
  (code) => code === 0,
);

check(
  "a package covered by an ignore higher up passes",
  (dir) => {
    writeFileSync(join(dir, ".gitignore"), "**/node_modules\n");
    pkg(dir, "ai/demo", null);
  },
  (code) => code === 0,
);

check(
  "a package inside an ignored tree is not judged at all",
  (dir) => {
    writeFileSync(join(dir, ".gitignore"), "vendor\n");
    pkg(dir, "vendor/demo", null);
    pkg(dir, "ai/real", "node_modules\n");
  },
  (code) => code === 0,
);

check(
  "a tree with no packages is refused rather than reported clean",
  () => {},
  (code, out) => code === 1 && out.includes("no Node packages"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) console.log(`FAILED ${f.name}\n  exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all cases behaved");
