// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-app-locale-used.
//
// The first case is the defect it was written from: the viewers app wrote a
// file's modified date with `new Date(ms).toLocaleString()` and no locale, so a
// German window showed an English date whenever the machine was English.
//
// Run: node dev/scripts/test-check-app-locale-used.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-app-locale-used.py");
const failures = [];

function run(name, files, expect) {
  const dir = mint("arlen-loc-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  cleanup(dir);
}

const F = "apps/demo/src/lib/panel.ts";

run(
  "a date formatted with no locale is caught",
  { [F]: "export const when = (ms: number) => new Date(ms).toLocaleString();\n" },
  (code, out) => code === 1 && out.includes("panel.ts"),
);

// `undefined` is the same defect spelled deliberately, and it is how the
// Settings notifications page had it - which reads more like a decision than an
// omission, so it must not read as one to the check either.
run(
  "an explicit undefined is the same defect",
  {
    [F]:
      'export const when = (ms: number) =>\n' +
      '  new Date(ms).toLocaleString(undefined, { hour: "2-digit" });\n',
  },
  (code, out) => code === 1 && out.includes("panel.ts"),
);

run(
  "the app's locale passes",
  { [F]: "export const when = (ms: number, loc: string) => new Date(ms).toLocaleString(loc);\n" },
  (code) => code === 0,
);

// A bare `Intl` formatter is the same question in a different spelling. None
// existed on the day this was written; it is here so the first one is caught.
run(
  "a bare Intl formatter is caught too",
  { [F]: "export const n = new Intl.NumberFormat().format(1);\n" },
  (code, out) => code === 1 && out.includes("panel.ts"),
);

// Letter casing is a different question with a different right answer: a bare
// `toLocaleLowerCase` is ordinary code, not a surface writing in the wrong
// language.
run(
  "case conversion is not this rule's business",
  { [F]: 'export const k = (s: string) => s.toLocaleLowerCase();\n' },
  (code) => code === 0,
);

run(
  "an empty tree refuses rather than passing",
  { "README.md": "nothing here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

for (const f of failures) console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
if (failures.length) process.exit(1);
console.log("a surface formats in the language its reader chose, and the list only shrinks");
