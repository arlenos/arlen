#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-refusal-language.py. The fault is staged as it actually
// arrived - a catalog sentence ending in `{$reason}` and a caller filling it with
// `String(e)` - rather than as a contrived string.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-refusal-language.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(source, name = "store.ts") {
  const root = mint("refusal-");
  mkdirSync(join(root, "apps/thing/src/lib"), { recursive: true });
  if (source !== null) writeFileSync(join(root, "apps/thing/src/lib", name), source);
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf-8", stdio: "pipe" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

{
  const root = tree(`export function f(e: unknown) {\n  return get(t)("th.failed", { reason: String(e) });\n}\n`);
  const rc = run(root);
  rc === 1 ? ok("a sentence finished with a stringified error is caught") : bad("a sentence finished with a stringified error is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // The same defect in markup, where `$t` is the call.
  const root = tree(`<p>{$t("th.failed", { reason: String(err) })}</p>\n`, "Thing.svelte");
  const rc = run(root);
  rc === 1 ? ok("the markup form is caught too") : bad("the markup form is caught too", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // Real data in a sentence is what a placeholder is FOR.
  const root = tree(`export function f(p: string) {\n  return get(t)("th.notFound", { path: p });\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("a placeholder filled with real data passes") : bad("a placeholder filled with real data passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A tagged refusal read into its own sentence is the shape this asks for.
  const root = tree(`export function f(e: unknown) {\n  const p = e as { problem: string; why: string };\n  if (p.problem === "not-written") return get(t)("th.notWritten", { why: p.why });\n  return get(t)("th.failed");\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("a tagged refusal writing its own sentence passes") : bad("a tagged refusal writing its own sentence passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // Stringifying an error to LOG it is not putting it on screen.
  const root = tree(`export function f(e: unknown) {\n  console.warn("thing: refused", String(e));\n  return get(t)("th.failed");\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("logging a stringified error is not the defect") : bad("logging a stringified error is not the defect", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // THE LAUNDERED FORM, which the gate missed for its first hours and which two
  // real defects took: the stringified error is put in a variable, and the
  // variable finishes the sentence.
  const root = tree(`export function f(e: unknown) {\n  let msg: string | null = null;\n  try { go(); } catch (e) { msg = String(e); }\n  return get(t)("th.failed", { reason: msg });\n}\n`);
  const rc = run(root);
  rc === 1 ? ok("a raw error laundered through a variable is caught") : bad("a raw error laundered through a variable is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // A store's `.set(String(e))` and a `$store` read in the argument are the same
  // defect across a `$`, which the name test has to allow for.
  const root = tree(`export const failure = writable<string | null>(null);\nexport function f(e: unknown) {\n  failure.set(String(e));\n  return get(t)("th.failed", { reason: $failure });\n}\n`);
  const rc = run(root);
  rc === 1 ? ok("a store set from a stringified error is caught through its $ read") : bad("a store set from a stringified error is caught through its $ read", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // THE BOUNDARY, one character from the defect: the same variable REASSIGNED to
  // a token before use is the fix, and must pass. Without this the gate would
  // punish the shape it is asking for.
  const root = tree(`export function f(e: unknown) {\n  let msg: string | null = null;\n  try { go(); } catch (e) { msg = tokenOf(e); }\n  return get(t)("th.failed", { reason: msg });\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("the same variable holding a token instead passes") : bad("the same variable holding a token instead passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A name that merely CONTAINS a tainted name is a different name. `reason` is
  // tainted here and `reasonKey` is not, which is exactly the pair the text
  // editor ended up with.
  const root = tree(`export function f(e: unknown) {\n  const reason = String(e);\n  console.warn(reason);\n  const reasonKey = keyOf(e);\n  return get(t)("th.failed", { why: reasonKey });\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("a longer name that merely contains a tainted one is not it") : bad("a longer name that merely contains a tainted one is not it", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // ACROSS A FILE, along an import: the store sets it, the page shows it.
  const root = tree(null);
  const dir = `${root}/apps/thing/src`;
  mkdirSync(`${dir}/lib/stores`, { recursive: true });
  mkdirSync(`${dir}/routes`, { recursive: true });
  writeFileSync(`${dir}/lib/stores/thing.ts`, `export const failure = writable<string | null>(null);\nexport function load(e: unknown) { failure.set(String(e)); }\n`);
  writeFileSync(`${dir}/routes/+page.svelte`, `<script>\n  import { failure } from "$lib/stores/thing";\n</script>\n<p>{$t("th.failed", { reason: $failure })}</p>\n`);
  const rc = run(root);
  rc === 1 ? ok("a taint imported from another file is caught") : bad("a taint imported from another file is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // THE BOUNDARY that made the name-only version report nonsense: two files with
  // a same-named local and NO import between them are two different variables.
  const root = tree(null);
  const dir = `${root}/apps/thing/src`;
  mkdirSync(`${dir}/lib`, { recursive: true });
  mkdirSync(`${dir}/routes`, { recursive: true });
  writeFileSync(`${dir}/lib/other.ts`, `export function f(e: unknown) { const reason = String(e); console.warn(reason); }\n`);
  writeFileSync(`${dir}/routes/+page.svelte`, `<script>\n  const reason = tokenOf(err);\n</script>\n<p>{$t("th.failed", { reason })}</p>\n`);
  const rc = run(root);
  rc === 0 ? ok("a same-named local in an unrelated file is not a taint") : bad("a same-named local in an unrelated file is not a taint", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // THE TAGGED-REFUSAL SHAPE: the taint is a FIELD of an object literal, never
  // bound to a name. Mail and calendar both carried it past this check for a day.
  const root = tree(`export function f(e: unknown) {\n  let failure = null;\n  try { go(); } catch (e) { failure = { problem: "launch", reason: String(e) }; }\n  return get(t)("th.failed", { reason: failure.reason });\n}\n`);
  const rc = run(root);
  rc === 1 ? ok("a stringified error reached through an object field is caught") : bad("a stringified error reached through an object field is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // The boundary: the same field name carrying real data. `reason` is a common
  // name and the rule must not fire on every object that has one.
  const root = tree(`export function f(kind: string) {\n  const failure = { problem: "launch", reason: kind };\n  return get(t)("th.failed", { reason: failure.reason });\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("the same field carrying real data passes") : bad("the same field carrying real data passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A TERNARY is not a field. `e instanceof Error ? e.message : String(e)` has
  // the same shape as `{ message: String(e) }` to a regex, and binding `message`
  // from it would taint a name nobody assigned - the shell's theme store alone
  // has six.
  const root = tree(`export function f(e: unknown) {\n  const message = e instanceof Error ? e.message : String(e);\n  console.warn(message);\n  return get(t)("th.failed", { detail: shortOf(message) });\n}\n`);
  const rc = run(root);
  rc === 0 ? ok("a ternary is not read as a tainted field") : bad("a ternary is not read as a tainted field", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A gate that reads nothing must not look like a gate that found nothing wrong.
  const root = tree(null);
  const rc = run(root);
  rc === 2 ? ok("finding no sources at all is not a pass") : bad("finding no sources at all is not a pass", `expected 2, got ${rc}`);
  cleanup(root);
}

{
  // An acknowledged line that no longer matches is a stale excuse. The file has to
  // be PRESENT for that to mean anything - an absent file just means this root is
  // not the repository - so it is staged here with the offending line gone.
  const root = mint("refusal-stale-");
  mkdirSync(join(root, "apps/settings/src/routes/appearance/quicksettings"), { recursive: true });
  writeFileSync(
    join(root, "apps/settings/src/routes/appearance/quicksettings/+page.svelte"),
    "<p>nothing here any more</p>\n",
  );
  const rc = run(root);
  rc === 1 ? ok("an acknowledgement whose line is gone fails") : bad("an acknowledgement whose line is gone fails", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const repo = join(here, "..", "..");
  const rc = run(repo);
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `expected 0, got ${rc}`);
}

console.log("a refusal reaches the reader in one language");
process.exit(failures === 0 ? 0 : 1);
