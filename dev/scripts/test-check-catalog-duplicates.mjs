// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control: a duplicate has to fail, a same id in two LOCALES has to pass
// (that is what a catalogue is), and an empty tree has to refuse rather than
// report a clean scan of nothing.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-catalog-duplicates.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

function tree(body) {
  const dir = mint("catalog-dup-");
  const at = join(dir, "apps/one/src/lib/i18n");
  mkdirSync(at, { recursive: true });
  writeFileSync(join(at, "messages.ts"), body);
  return dir;
}

// An app whose catalogue is SPLIT over several files and merged, which is what
// settings does. A key in two of them is dead in the earlier one and nothing
// looked across the split until 8 September.
function splitTree(a, b) {
  const dir = mint("catalog-dup-split-");
  const at = join(dir, "apps/one/src/lib/i18n");
  mkdirSync(at, { recursive: true });
  writeFileSync(join(at, "messages.a.ts"), a);
  writeFileSync(join(at, "messages.b.ts"), b);
  writeFileSync(
    join(at, "messages.ts"),
    "const messages = {\n  en: { ...a.en, ...b.en },\n  de: { ...a.de, ...b.de },\n};\n",
  );
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [GATE, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout || "") + (e.stderr || "") };
  }
}

const CLEAN = `
const messages = {
  en: {
    "a.one": "One",
    "a.two": "Two",
  },
  de: {
    "a.one": "Eins",
    "a.two": "Zwei",
  },
};
`;

{
  const d = tree(CLEAN);
  const r = run(d);
  if (r.code === 0) ok("the same id in two locales is what a catalogue is");
  else bad("the same id in two locales is what a catalogue is", r.out);
  cleanup(d);
}

{
  const d = tree(`
const messages = {
  en: {
    "a.one": "One",
    "a.two": "Two",
    "a.one": "One again, and this is the one a reader gets",
  },
  de: {
    "a.one": "Eins",
    "a.two": "Zwei",
  },
};
`);
  const r = run(d);
  if (r.code === 1 && r.out.includes("a.one") && r.out.includes("en"))
    ok("a duplicate inside one locale is caught, with the locale named");
  else bad("a duplicate inside one locale is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // The half that is easy to get wrong the other way: a duplicate in the SECOND
  // locale block, which a scanner that stops at the first `}` would never reach.
  const d = tree(`
const messages = {
  en: {
    "a.one": "One",
  },
  de: {
    "a.one": "Eins",
    "a.one": "Eins, nochmal",
  },
};
`);
  const r = run(d);
  if (r.code === 1 && r.out.includes("de"))
    ok("a duplicate in a later locale block is reached");
  else bad("a duplicate in a later locale block is reached", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = splitTree(
    'const a = {\n  en: {\n    "a.one": "One",\n  },\n  de: {\n    "a.one": "Eins",\n  },\n};\n',
    'const b = {\n  en: {\n    "a.one": "One again",\n  },\n  de: {\n    "a.one": "Eins, nochmal",\n  },\n};\n',
  );
  const r = run(d);
  if (r.code === 1 && r.out.includes("messages.a.ts") && r.out.includes("messages.b.ts"))
    ok("a key defined by two of an app's catalogue files is caught");
  else bad("a key defined by two of an app's catalogue files is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // The other direction, and the reason this cannot just count files: a split
  // catalogue is the normal case, and two files that share no key are fine.
  const d = splitTree(
    'const a = {\n  en: {\n    "a.one": "One",\n  },\n  de: {\n    "a.one": "Eins",\n  },\n};\n',
    'const b = {\n  en: {\n    "a.two": "Two",\n  },\n  de: {\n    "a.two": "Zwei",\n  },\n};\n',
  );
  const r = run(d);
  if (r.code === 0) ok("two catalogue files that share no key pass");
  else bad("two catalogue files that share no key pass", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // And the merge file itself must not read as a definition of everything it
  // spreads, or every split catalogue in the tree is a finding.
  const d = splitTree(
    'const a = {\n  en: {\n    "a.one": "One",\n  },\n  de: {\n    "a.one": "Eins",\n  },\n};\n',
    'const b = {\n  en: {\n    "a.two": "Two",\n  },\n  de: {\n    "a.two": "Zwei",\n  },\n};\n',
  );
  const r = run(d);
  if (r.code === 0 && !r.out.includes("messages.ts")) ok("the merge file defines nothing");
  else bad("the merge file defines nothing", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = mint("catalog-dup-empty-");
  const r = run(d);
  if (r.code === 2) ok("a tree with no catalogue is an error, not a pass");
  else bad("a tree with no catalogue is an error, not a pass", `exit ${r.code}`);
  cleanup(d);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
