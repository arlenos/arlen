// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the counted-message check. Each case is a shape that
// was really in the tree: a count before a plural noun with no selector, which
// nothing catches because no fixture ever uses one as its number.
//
// The two passing cases matter as much as the failing ones. This check earns its
// keep by being quiet, so it has to stay quiet about `{$app} wants` (a name, not
// a count) and about a message that already selects.
//
// Run: node dev/scripts/test-check-counted-messages.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-counted-messages.py");
const CAT = "apps/demo/src/lib/i18n/messages.ts";

const failures = [];

function check(name, body, expect) {
  const dir = mint("arlen-counted-");
  if (body !== null) {
    mkdirSync(join(dir, "apps/demo/src/lib/i18n"), { recursive: true });
    writeFileSync(join(dir, CAT), body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-counted-messages positive control");

check(
  "a count before a plural noun with no selector is caught",
  `export const m = { en: {\n    "d.files": "{$count} files",\n} };\n`,
  (code, out) => code === 1 && out.includes("d.files") && out.includes("1 files"),
);

check(
  "the same message with a selector passes",
  `export const m = { en: {\n    "d.files": ".input {$count :number}\\n.match $count\\none {{one file}}\\n*   {{{$count} files}}",\n} };\n`,
  (code) => code === 0,
);

check(
  "a placeholder that is a name, not a count, is left alone",
  `export const m = { en: {\n    "d.wants": "{$app} wants access to your files",\n} };\n`,
  (code) => code === 0,
);

check(
  "a word ending in s that is not a plural noun is left alone",
  `export const m = { en: {\n    "d.state": "{$count} is what it reported",\n} };\n`,
  (code) => code === 0,
);

check(
  "a key that selects in one locale and not the other is caught",
  `export const m = {\n  en: {\n    "d.files": ".input {$count :number}\\n.match $count\\none {{one file}}\\n*   {{{$count} files}}",\n  },\n  de: {\n    "d.files": "{$count} Dateien",\n  },\n};\n`,
  (code, out) => code === 1 && out.includes("selects on its count in one locale"),
);

check(
  "a key that selects in both locales passes",
  `export const m = {\n  en: {\n    "d.files": ".input {$count :number}\\n.match $count\\none {{one file}}\\n*   {{{$count} files}}",\n  },\n  de: {\n    "d.files": ".input {$count :number}\\n.match $count\\none {{eine Datei}}\\n*   {{{$count} Dateien}}",\n  },\n};\n`,
  (code) => code === 0,
);

check(
  "a tree with no catalogue is refused rather than reported clean",
  null,
  (code, out) => code === 1 && out.includes("no message catalogues"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) console.log(`FAILED ${f.name}\n  exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all cases behaved");
