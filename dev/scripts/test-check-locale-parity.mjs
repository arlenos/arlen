// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the locale-parity gate see a key one language has and the other does not?
//
// The gap it covers is invisible to `check-message-keys` by that check's own
// deliberate design: it only reports keys asked for by a literal name, and about
// nine hundred keys in this tree are reached at runtime instead. So a
// dynamically-reached key added to `en` and not `de` is seen by nothing, and the
// German build renders the key forever - the shape that already shipped once, in
// a greeter whose entire German catalogue was unreachable.
//
// Over a fixture; the gate takes its root as `argv[1]`.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-locale-parity.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(body) {
  const dir = mint("arlen-locale-parity-");
  try {
    const i18n = path.join(dir, "apps", "probe", "src", "lib", "i18n");
    mkdirSync(i18n, { recursive: true });
    writeFileSync(path.join(i18n, "messages.ts"), body, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

const cat = (en, de) =>
  `const messages = {\n  en: {\n${en}\n  },\n  de: {\n${de}\n  },\n};\nexport default messages;\n`;

console.log("locale parity:");

{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver(cat(`    "p.a": "One",\n    "p.b": "Two",`, `    "p.a": "Eins",\n    "p.b": "Zwei",`));
  check("matching locales pass", r.code === 0, r.out.trim().split("\n").pop());
  // Pinned: a fixture the gate never read also exits 0, printing "0 locale pair(s)".
  check("and the gate actually compared a pair", r.out.includes("1 locale pair(s)"),
        r.out.trim().split("\n").pop());
}

{
  // THE DEFECT: present in English, absent in German. Nothing else in the tree
  // sees this when the key is reached dynamically.
  const r = gateOver(cat(`    "p.a": "One",\n    "p.only": "Two",`, `    "p.a": "Eins",`));
  check("a key English has and German lacks is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the key", r.out.includes("p.only"));
}

{
  // And the other direction, which is a real shape too: a German entry left
  // behind after its English key was renamed renders for nobody.
  const r = gateOver(cat(`    "p.a": "One",`, `    "p.a": "Eins",\n    "p.stale": "Alt",`));
  check("a key German has and English lacks is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and that finding names it too", r.out.includes("p.stale"));
}

{
  // A catalogue with one locale is not a mismatch - the kit shipped that way
  // before it had a second language, and refusing it would be inventing a rule.
  const r = gateOver(`const messages = {\n  en: {\n    "p.a": "One",\n  },\n};\nexport default messages;\n`);
  check("a single-locale catalogue is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate sees a key one language carries and the other does not, in both directions");
