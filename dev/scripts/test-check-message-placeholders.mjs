// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the placeholder gate see a message that prints the word inside its braces?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses are what the shape of the check rests on. `{$name}` is the
// correct form and must pass; `{{` is an escaped brace in prose and is not a
// placeholder at all; and a compiled copy of a catalogue under `.svelte-kit`
// reports the state of the last build rather than the tree, which is how the
// first run of this gate called ten already-fixed messages broken.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-message-placeholders.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(catalogue, where = ["apps", "demo", "src", "lib", "i18n"], file = "messages.ts") {
  const dir = mint("arlen-message-placeholders-");
  try {
    const at = path.join(dir, ...where);
    mkdirSync(at, { recursive: true });
    writeFileSync(path.join(at, file), catalogue, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

const wrap = (body) => `export const messages = {\n  en: {\n${body}\n  },\n};\n`;

console.log("message placeholders:");

{
  // The file manager's five, in the shape they shipped in.
  const r = gateOver(wrap('    "f.places.ejectAria": "Eject {place}",'));
  check("a bare {name} is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the key and the placeholder",
        r.out.includes("f.places.ejectAria") && r.out.includes("{place}"),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(wrap('    "f.places.ejectAria": "Eject {$place}",'));
  check("the {$name} form passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 catalogue(s)"),
        r.out.trim().split("\n").pop());
}

{
  // An escaped brace in prose. MessageFormat 2 writes it `{{`, and it is text.
  const r = gateOver(wrap('    "x.brace": "Write {{ to open a block",'));
  check("an escaped brace is not a placeholder", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // A function on a variable, which is the other legitimate form.
  const r = gateOver(wrap('    "x.count": "{$n :number} files",'));
  check("a formatted variable passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // A compiled catalogue under a build directory is the past, not the tree.
  const r = gateOver(wrap('    "x.stale": "Eject {place}",'),
                     ["apps", "demo", ".svelte-kit", "output", "server", "chunks"],
                     "messages.js");
  check("a build output is not read", r.code === 0, r.out.trim().split("\n").pop());
  check("and reading nothing is not reported as clean coverage",
        r.out.includes("0 catalogue(s)"), r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a message that would print the word inside its braces");
