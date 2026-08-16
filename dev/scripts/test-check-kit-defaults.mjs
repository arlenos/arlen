// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-kit-defaults.
//
// The arrow-function case is the one to keep: the first version of this scan
// matched the tag with a non-greedy `.*?>`, and `onadd={() => open()}` ended it
// early, so two settings pages were reported for a prop they pass three lines
// down. A checker that invents work is worse than one that misses some.
//
// Run: node dev/scripts/test-check-kit-defaults.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-kit-defaults.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-kitdef-"));
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

// A kit component with one prose default and one token default. Only the prose
// one is this rule's business; `size = "sm"` is not a sentence anybody reads.
const KIT = {
  "sdk/ui-kit/src/lib/components/browser/Thing.svelte":
    "<script lang=\"ts\">\n" +
    "  let {\n" +
    '    emptyLabel = "Nothing here yet",\n' +
    '    size = "sm",\n' +
    "  }: { emptyLabel?: string; size?: string } = $props();\n" +
    "</script>\n<p>{emptyLabel}{size}</p>\n",
};

console.log("check-kit-defaults:");

check(
  "a tree with no kit components is refused rather than reported clean",
  { "apps/demo/src/lib/View.svelte": "<p>x</p>\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a mount that does not pass the prose default is caught",
  { ...KIT, "apps/demo/src/lib/View.svelte": "<Thing />\n" },
  (code, out) => code === 1 && out.includes("emptyLabel"),
);

check(
  "a mount that passes it is clean",
  { ...KIT, "apps/demo/src/lib/View.svelte": '<Thing emptyLabel={$t("d.empty")} />\n' },
  (code) => code === 0,
);

// The regression that matters: a `>` inside a prop expression must not end the
// tag early and hide the prop that follows it.
check(
  "an arrow function in a prop does not hide the props after it",
  {
    ...KIT,
    "apps/demo/src/lib/View.svelte":
      "<Thing\n  onadd={() => open()}\n  emptyLabel={$t(\"d.empty\")}\n/>\n",
  },
  (code) => code === 0,
);

// A token default is not prose, so a mount that omits it is not a finding.
check(
  "a short token default is not treated as prose",
  { ...KIT, "apps/demo/src/lib/View.svelte": '<Thing emptyLabel={$t("d.empty")} />\n' },
  (code, out) => code === 0 && !out.includes("size"),
);

for (const f of failures) {
  console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
}
if (failures.length) process.exit(1);
console.log("the omission is caught, the fix passes, and an arrow function does not fool it");
