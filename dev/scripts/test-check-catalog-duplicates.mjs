// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control: a duplicate has to fail, a same id in two LOCALES has to pass
// (that is what a catalogue is), and an empty tree has to refuse rather than
// report a clean scan of nothing.
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-catalog-duplicates.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

function tree(body) {
  const dir = mkdtempSync(join(tmpdir(), "catalog-dup-"));
  const at = join(dir, "apps/one/src/lib/i18n");
  mkdirSync(at, { recursive: true });
  writeFileSync(join(at, "messages.ts"), body);
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
  rmSync(d, { recursive: true, force: true });
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
  rmSync(d, { recursive: true, force: true });
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
  rmSync(d, { recursive: true, force: true });
}

{
  const d = mkdtempSync(join(tmpdir(), "catalog-dup-empty-"));
  const r = run(d);
  if (r.code === 2) ok("a tree with no catalogue is an error, not a pass");
  else bad("a tree with no catalogue is an error, not a pass", `exit ${r.code}`);
  rmSync(d, { recursive: true, force: true });
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
