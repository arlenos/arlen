// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate refuses a translator call that names a message taking a parameter and
// passes none, because MessageFormat prints the placeholder rather than failing.
// The case it was written for is real: the display revert modal headed itself
// `Aenderungen behalten ({$seconds} s)` on 8 September.
//
// Run: node dev/scripts/test-check-message-params.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-message-params.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, why) => {
  console.log(`  FAIL ${n}\n       ${why}`);
  failures++;
};

// One app: a catalogue and one caller file.
function tree(catalogue, caller) {
  const dir = mint("msg-params-");
  const i18n = join(dir, "apps/one/src/lib/i18n");
  const lib = join(dir, "apps/one/src/lib");
  mkdirSync(i18n, { recursive: true });
  writeFileSync(join(i18n, "messages.ts"), catalogue);
  writeFileSync(join(lib, "Thing.svelte"), caller);
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

const CATALOGUE = `
const messages = {
  en: {
    "a.plain": "Keep it",
    "a.counted": "Keep changes ({$seconds}s)",
    "a.formatted": "{$n :number} left",
  },
  de: {
    "a.plain": "Behalten",
    "a.counted": "Änderungen behalten ({$seconds}s)",
    "a.formatted": "{$n :number} übrig",
  },
};
`;

console.log("check-message-params:");

{
  const d = tree(CATALOGUE, '<h2>{$t("a.counted")}</h2>\n');
  const r = run(d);
  if (r.code === 1 && r.out.includes("a.counted")) ok("a bare call to a counted message is caught");
  else bad("a bare call to a counted message is caught", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = tree(CATALOGUE, '<h2>{$t("a.counted", { seconds: 12 })}</h2>\n');
  const r = run(d);
  if (r.code === 0) ok("the same call with its parameter passes");
  else bad("the same call with its parameter passes", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = tree(CATALOGUE, '<h2>{$t("a.plain")}</h2>\n');
  const r = run(d);
  if (r.code === 0) ok("a message that takes nothing is called with nothing");
  else bad("a message that takes nothing is called with nothing", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // `{$n :number}` is the other placeholder form in this tree, and a pattern
  // that insisted on a closing brace right after the name would miss all 114 of
  // them.
  const d = tree(CATALOGUE, '<p>{$t("a.formatted")}</p>\n');
  const r = run(d);
  if (r.code === 1 && r.out.includes("a.formatted")) ok("a formatted placeholder counts as a parameter");
  else bad("a formatted placeholder counts as a parameter", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // The shape the gate must not touch: an id chosen from data. Those pass their
  // parameters along, and no static reader can say which message they land on.
  const d = tree(CATALOGUE, "<p>{$t(line.provenance.id, line.provenance.params)}</p>\n");
  const r = run(d);
  if (r.code === 0) ok("a call whose id is a variable is left alone");
  else bad("a call whose id is a variable is left alone", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  // The kit's translator reaches a catalogue the same way and its ids are called
  // from apps, so `$kt` is read too.
  const d = tree(CATALOGUE, '<span>{$kt("a.counted")}</span>\n');
  const r = run(d);
  if (r.code === 1) ok("the kit's translator is read as well");
  else bad("the kit's translator is read as well", `exit ${r.code}: ${r.out}`);
  cleanup(d);
}

{
  const d = mint("msg-params-empty-");
  const r = run(d);
  if (r.code === 2) ok("a tree with no parameterised message is an error, not a pass");
  else bad("a tree with no parameterised message is an error, not a pass", `exit ${r.code}`);
  cleanup(d);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
