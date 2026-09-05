// Does the copy-character gate actually catch what it claims to?
//
// A check that has only ever passed is an assertion, not a control. Each case
// below puts one fault into a real catalogue, runs the gate, and requires it to
// fail naming that fault - and the last two require it to STAY QUIET on the two
// things it deliberately does not ban, because a gate that fires on the platform
// ellipsis convention would be reverted within a day and take the m-dash rule
// with it.
//
// The fixture is a copy of the tree made with `git worktree`-free plumbing: we
// edit a real tracked catalogue, run, then restore it from git. Nothing is
// deleted, so no path this script did not create can be lost.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-copy-characters.py");
const target = path.join(root, "apps/clock/src/lib/i18n/messages.ts");
const original = readFileSync(target, "utf8");

function run() {
  try {
    return { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
}

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function withLine(line, fn) {
  const anchor = "  en: {";
  if (!original.includes(anchor)) throw new Error("fixture shape changed: no `en:` block");
  writeFileSync(target, original.replace(anchor, `${anchor}\n    ${line}`), "utf8");
  try { fn(run()); } finally { writeFileSync(target, original, "utf8"); }
}

console.log("copy characters:");

// It passes on the tree as it stands. Stated first, because every case below is
// only meaningful if the baseline is green.
{
  const r = run();
  check("the catalogues as they stand are clean", r.code === 0, r.out.trim().split("\n").pop());
}

withLine('"t.fault": "One thing — and another",', (r) => {
  check("an m-dash in a catalogue value is caught", r.code === 1 && r.out.includes("m-dash"), r.out.trim().split("\n")[0]);
});

withLine('"t.fault": "Arlen OS · 1.2",', (r) => {
  check("a middot used as a separator is caught", r.code === 1 && r.out.includes("middot"), r.out.trim().split("\n")[0]);
});

// The two deliberate non-bans. These are the cases that keep the gate credible.
withLine('"t.fine": "Open with…",', (r) => {
  check("the platform ellipsis convention is left alone", r.code === 0, r.out.trim().split("\n")[0]);
});

withLine('"t.fine": "Ärztin·Arzt",', (r) => {
  check("a middot inside a word is left alone", r.code === 0, r.out.trim().split("\n")[0]);
});

// And the fixture is back exactly as it was, which the next run depends on.
check("the catalogue it edited is restored byte for byte", readFileSync(target, "utf8") === original);

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches an m-dash and a middot separator, and stays quiet on an ellipsis");
