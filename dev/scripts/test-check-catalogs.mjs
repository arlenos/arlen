// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the catalog gate still go red for a message that does not format?
//
// The check's own note says the gap plainly: nothing had ever shown it going RED
// for a catalog that genuinely does not format, "which is the case it exists for".
// What it guards is invisible to everything else - a MessageFormat 2.0 source
// string inside a TypeScript literal is a `string` to tsc and svelte-check, and
// its selector syntax is not parsed until the message is first formatted, which
// for a locale nobody on the team reads is in front of a user.
//
// Over a fixture; it takes a base directory as `argv[2]`.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-catalogs.mjs");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// One app under a throwaway base, holding `body` as its catalogue.
function gateOver(body) {
  const dir = mint("arlen-catalogs-");
  try {
    const i18n = path.join(dir, "probe", "src", "lib", "i18n");
    mkdirSync(i18n, { recursive: true });
    writeFileSync(path.join(i18n, "messages.ts"), body, "utf8");
    try {
      return { code: 0, out: execFileSync("node", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

const catalogue = (en) =>
  `const messages = {\n  en: {\n${en}\n  },\n  de: {\n${en}\n  },\n};\nexport default messages;\n`;

console.log("catalogs:");

{
  let r;
  try {
    r = { code: 0, out: execFileSync("node", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver(catalogue(`    "p.plain": "A plain message",`));
  check("a plain message passes", r.code === 0, r.out.trim().split("\n").pop());
  // Pinned: a base with no catalogue in it also exits 0, and that is how this
  // control would go quietly vacuous.
  check("and the gate actually formatted something",
        /\b[1-9]\d* catalog message\(s\) compile and format/.test(r.out),
        r.out.trim().split("\n").pop());
}

{
  // A selector with no `.match` - the exact shape the header names, and one that
  // every other tool in the pipeline calls a valid string.
  const r = gateOver(catalogue(
    `    "p.broken": ".input {$n :number}\\none {{one}}\\n* {{many}}",`));
  check("a selector missing its .match is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  // An unbalanced brace, likewise invisible until the message is formatted.
  const r = gateOver(catalogue(`    "p.unbalanced": "Hello {$name",`));
  check("an unbalanced brace is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  // A duplicate id: this reader takes the last one and so does the bundler, so
  // the app silently shows that message wherever the earlier id was used. The
  // check's own comment records it costing a real one.
  const r = gateOver(catalogue(
    `    "p.same": "first",\n    "p.same": "second",`));
  check("a duplicate id is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate goes red for a message that does not format and for a duplicate id");
