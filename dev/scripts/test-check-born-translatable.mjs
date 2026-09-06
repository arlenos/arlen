#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-born-translatable.py. The fault is staged as it arrived in
// the shell - a hint written into the DOM in English - and every case that must
// PASS is a real shape from the tree beside it.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-born-translatable.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(files) {
  const root = mint("born-translatable-");
  for (const [rel, body] of Object.entries(files ?? {})) {
    mkdirSync(join(root, dirname(rel)), { recursive: true });
    writeFileSync(join(root, rel), body);
  }
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

console.log("check-born-translatable:");

{
  // THE FAULT, as the Waypointer had it: a sentence written into the DOM.
  const root = tree({
    "apps/thing/src/lib/A.svelte":
      '<script>\n  function paint(el) { el.textContent = "Enter: run. Shift+Enter: terminal."; }\n</script>\n',
  });
  const r = run(root);
  r.code === 1 && r.out.includes("Enter: run")
    ? ok("a sentence assigned to a text property is caught")
    : bad("a sentence assigned to a text property is caught", `got ${r.code}`);
  cleanup(root);
}

{
  // The same fault as a RETURN, which is how the command editor had it.
  const root = tree({
    "apps/thing/src/lib/B.svelte":
      '<script>\n  function validate(s) { if (!s) return "The command is empty here."; return null; }\n</script>\n',
  });
  const r = run(root);
  r.code === 1 && r.out.includes("The command is empty")
    ? ok("a sentence returned from a validator is caught")
    : bad("a sentence returned from a validator is caught", `got ${r.code}`);
  cleanup(root);
}

{
  // An ACCESSIBLE NAME, which a screen reader reads out.
  const root = tree({
    "apps/thing/src/lib/C.svelte": '<div role="img" aria-label="Signal strength is low">x</div>\n',
  });
  const r = run(root);
  r.code === 1 && r.out.includes("Signal strength")
    ? ok("an English accessible name is caught")
    : bad("an English accessible name is caught", `got ${r.code}`);
  cleanup(root);
}

{
  // THE FIX, and it must pass: the sentence comes from the catalogue.
  const root = tree({
    "apps/thing/src/lib/D.svelte":
      '<script>\n  function paint(el) { el.textContent = $t("sh.wp.runHint"); }\n</script>\n' +
      '<div aria-label={$t("sh.net.signalAria", { percent: n })}>x</div>\n',
  });
  const r = run(root);
  r.code === 0 ? ok("reading the sentence from the catalogue passes") : bad("reading the sentence from the catalogue passes", r.out);
  cleanup(root);
}

{
  // A `$props()` DEFAULT. The kit writes its fallbacks in English deliberately
  // and `check-kit-defaults.py` holds apps to overriding them; counting them
  // here would report a pattern that is not the defect.
  const root = tree({
    "sdk/ui-kit/src/lib/E.svelte":
      '<script lang="ts">\n  let {\n    errorTitle = "Can\'t open this folder",\n    hint = "Something went wrong reading it",\n  }: { errorTitle?: string; hint?: string } = $props();\n</script>\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a kit default in a $props() block passes") : bad("a kit default in a $props() block passes", r.out);
  cleanup(root);
}

{
  // THE FIELD FORM. A catalogue of rows is written with a `:`, not an `=`, so the
  // assignment matcher never saw it - which is how two Settings rows carried an
  // English sentence into a German page while the `label` beside them was a key.
  const root = tree({
    "apps/thing/src/lib/rows.ts":
      'export const ROWS = [\n  { key: "a", label: "s.row.a", description: "Written to disk as you type." },\n];\n',
  });
  const r = run(root);
  r.code === 1 && r.out.includes("Written to disk")
    ? ok("a prose description field is caught")
    : bad("a prose description field is caught", r.out);
  cleanup(root);
}

{
  // A key in the same position is not prose and must stay quiet, or the form
  // would fire on every catalogue-driven row in the tree.
  const root = tree({
    "apps/thing/src/lib/rows.ts":
      'export const ROWS = [\n  { key: "a", label: "s.row.a", description: "s.row.aDesc" },\n];\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a key in a description field passes") : bad("a key in a description field passes", r.out);
  cleanup(root);
}

{
  // A file may declare its data foreign, and five in the tree already do: the
  // strings are a third party's own words arriving as data.
  const root = tree({
    "apps/thing/src/lib/fixture.ts":
      '/// i18n-foreign: the fixture holds correspondents\' own words.\nexport const M = [\n  { subject: "The hall is free on Thursday." },\n  { label: "Re: review notes", description: "Bringing the printouts, someone else brings coffee." },\n];\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a file marked i18n-foreign is skipped") : bad("a file marked i18n-foreign is skipped", r.out);
  cleanup(root);
}

{
  // An identifier is not a sentence, however long.
  const root = tree({
    "apps/thing/src/lib/F.svelte":
      '<script>\n  function kind() { return "no-renderer-available"; }\n  const label = "data-place-recent";\n</script>\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a lowercase identifier is not read as prose") : bad("a lowercase identifier is not read as prose", r.out);
  cleanup(root);
}

{
  // The catalogue itself is where sentences live.
  const root = tree({
    "apps/thing/src/lib/i18n/messages.ts":
      'export const messages = { en: { "t.k": "This is a whole sentence." } };\n',
    // A source beside it, so the run has something to look at: a tree with no
    // frontend at all is the refusal case below, not this one.
    "apps/thing/src/lib/G.svelte": '<span>{$t("t.k")}</span>\n',
  });
  const r = run(root);
  r.code === 0 ? ok("the catalogue file is not its own finding") : bad("the catalogue file is not its own finding", r.out);
  cleanup(root);
}

{
  // A run that looked at nothing must refuse, not pass.
  const root = tree({});
  const r = run(root);
  r.code === 2 ? ok("a tree with no frontend refuses") : bad("a tree with no frontend refuses", `got ${r.code}`);
  cleanup(root);
}

{
  // THE REPOSITORY, which is the case that keeps the carried numbers honest.
  const repo = join(here, "..", "..");
  const r = run(repo);
  r.code === 0 ? ok("the repository itself passes") : bad("the repository itself passes", r.out);
}

console.log(failures === 0
  ? "a sentence a person reads comes from the catalogue"
  : `${failures} case(s) failed`);
process.exit(failures === 0 ? 0 : 1);
