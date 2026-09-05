// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the copy-character gate actually catch what it claims to?
//
// A check that has only ever passed is an assertion, not a control. Each case
// below puts one fault into a catalogue, runs the gate, and requires it to fail
// naming that fault - and two cases require it to STAY QUIET on the things it
// deliberately does not ban, because a gate that fires on the platform ellipsis
// convention would be reverted within a week and take the m-dash rule with it.
//
// OVER A FIXTURE, never this tree. The first version edited a real app catalogue
// in place and restored it afterwards, which is unsafe for a reason the hook
// states at its own top: "the gates run concurrently", so a control that mutates
// tracked files is visible to every neighbour while it runs. The gate takes its
// root as `argv[1]` for exactly this.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-copy-characters.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// Run the gate over a throwaway tree holding one catalogue with `line` in it.
function gateOver(line) {
  const dir = mint("arlen-copy-chars-");
  try {
    const i18n = path.join(dir, "apps", "clock", "src", "lib", "i18n");
    mkdirSync(i18n, { recursive: true });
    writeFileSync(
      path.join(i18n, "messages.ts"),
      `const messages = {\n  en: {\n    ${line}\n  },\n};\nexport default messages;\n`,
      "utf8",
    );
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("copy characters:");

// The real tree, read-only. Every case below is only meaningful if this is green.
{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the catalogues as they stand are clean", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver(`"t.ok": "One thing, and another",`);
  check("a clean catalogue passes", r.code === 0, r.out.trim().split("\n")[0]);
  // Pinned: a run that found no catalogue also exits 0, and that is how a control
  // like this goes quietly vacuous.
  check("and the gate actually looked at the fixture", /1 catalogues clean/.test(r.out),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`"t.fault": "One thing — and another",`);
  check("an m-dash in a catalogue value is caught", r.code === 1 && r.out.includes("m-dash"),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`"t.fault": "Arlen OS · 1.2",`);
  check("a middot used as a separator is caught", r.code === 1 && r.out.includes("middot"),
        r.out.trim().split("\n")[0]);
}

// The two deliberate non-bans. These are what keep the gate credible.
{
  const r = gateOver(`"t.fine": "Open with…",`);
  check("the platform ellipsis convention is left alone", r.code === 0, r.out.trim().split("\n")[0]);
}
{
  const r = gateOver(`"t.fine": "Ärztin·Arzt",`);
  check("a middot inside a word is left alone", r.code === 0, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches an m-dash and a middot separator, and stays quiet on an ellipsis");
