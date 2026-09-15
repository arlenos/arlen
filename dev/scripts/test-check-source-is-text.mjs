// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the source-is-text gate see a NUL byte in a text file?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `sdk/` would be visible to its
// neighbours mid-run.
//
// The near-miss is the point of the whole check: the ESCAPED form is the fix, so a
// file containing the six characters of an escape must pass while a file containing
// the byte must not. A gate that could not tell them apart would ask people to
// delete a correct separator.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-source-is-text.py");
const NUL = String.fromCharCode(0);

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(files) {
  const dir = mint("arlen-source-is-text-");
  try {
    for (const [rel, text] of Object.entries(files)) {
      const at = path.join(dir, rel);
      mkdirSync(path.dirname(at), { recursive: true });
      writeFileSync(at, text);
    }
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("source is text:");

{
  const r = gateOver({ "sdk/thing/src/key.ts": "const key = `${a}" + NUL + "${b}`;\n" });
  check("a NUL byte in a .ts file is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the line", r.out.includes(":1:"), r.out.trim().split("\n")[0]);
}

{
  // THE FIX: the same string at runtime, written as an escape. Six ASCII characters.
  const r = gateOver({ "sdk/thing/src/key.ts": "const key = `${a}\\u0000${b}`;\n" });
  check("the escaped form passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver({ "daemons/x/src/main.rs": 'let k = format!("{a}' + NUL + '{b}");\n' });
  check("Rust counts too", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  // A compiled terminfo entry is named after the TERMINAL, so the image ships one
  // called `xterm.js` which is binary on purpose. It must not be reported.
  const r = gateOver({
    "dev/mkosi/mkosi.tools/usr/share/terminfo/x/xterm.js": NUL + NUL + "data",
    // One ordinary file beside it, because a tree with nothing to scan is a
    // pointed-wrong scan and the gate refuses that on purpose.
    "sdk/thing/src/ok.ts": "export const a = 1;\n",
  });
  check("a terminfo entry named like a source file is left alone", r.code === 0,
        r.out.trim().split("\n").pop());
}

{
  // A genuinely binary format is not in scope at all.
  const r = gateOver({
    "apps/x/static/icon.png": NUL + "PNG",
    "sdk/thing/src/ok.ts": "export const a = 1;\n",
  });
  check("a png is not a text source file", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a text source file grep cannot search");
