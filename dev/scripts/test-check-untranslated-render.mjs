#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-untranslated-render.py. The fault is staged as it arrived in
// the file manager - a store set from `String(e)` and a red bar drawing it - and
// every case that must PASS is a real shape from the tree beside it.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-untranslated-render.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(files) {
  const root = mkdtempSync(join(tmpdir(), "untranslated-"));
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

console.log("check-untranslated-render:");

{
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  let opError = null;\n  function go(e) { opError = String(e); }\n</script>\n" +
      "{#if opError}<span>{opError}</span>{/if}\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("opError")
    ? ok("a stringified error drawn bare is caught")
    : bad("a stringified error drawn bare is caught", `got ${r.code}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // THE FIX, and it must pass: the store holds a KEY and the markup calls the
  // catalogue. One character of difference from the line above.
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  let opError = null;\n  function go(e) { opError = keyOf(e); }\n</script>\n" +
      "{#if opError}<span>{$t(opError)}</span>{/if}\n",
  });
  const r = run(root);
  r.code === 0 ? ok("holding a key and calling the catalogue passes") : bad("holding a key and calling the catalogue passes", r.out);
  rmSync(root, { recursive: true, force: true });
}

{
  // ACROSS A FILE, along an import - the file manager's actual shape.
  const root = tree({
    "apps/thing/src/lib/stores/ops.ts":
      'export const opError = writable(null);\nexport function go(e) { opError.set(String(e)); }\n',
    "apps/thing/src/lib/Overlay.svelte":
      '<script>\n  import { opError } from "$lib/stores/ops";\n</script>\n<span>{$opError}</span>\n',
  });
  const r = run(root);
  r.code === 1 && r.out.includes("opError")
    ? ok("a taint imported from a store is followed into the markup")
    : bad("a taint imported from a store is followed into the markup", `got ${r.code}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The boundary the import requirement draws: two same-named locals in
  // unrelated files are two variables, not one taint.
  const root = tree({
    "apps/thing/src/lib/other.ts": "export function f(e) { const error = String(e); log(error); }\n",
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  const error = countOf(rows);\n</script>\n<span>{error}</span>\n",
  });
  const r = run(root);
  r.code === 0 ? ok("a same-named local in an unrelated file is not a taint") : bad("a same-named local in an unrelated file is not a taint", r.out);
  rmSync(root, { recursive: true, force: true });
}

{
  // A surface drawing its own data is not this check's business.
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  let count = 3;\n  let path = '/tmp/x';\n</script>\n<span>{count} in {path}</span>\n",
  });
  const r = run(root);
  r.code === 0 ? ok("a count and a path drawn bare are not findings") : bad("a count and a path drawn bare are not findings", r.out);
  rmSync(root, { recursive: true, force: true });
}

{
  // The same taint inside a `$t(...)` belongs to the sibling check, which words
  // it better because it can name the key. Reporting it here too would make one
  // defect two findings in two files.
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  let reason = null;\n  function go(e) { reason = String(e); }\n</script>\n" +
      '<span>{$t("th.failed", { reason })}</span>\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a taint inside a translate call is left to the sibling check") : bad("a taint inside a translate call is left to the sibling check", r.out);
  rmSync(root, { recursive: true, force: true });
}

{
  // A tainted FIELD read as a field, which is how a tagged refusal reaches markup.
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  let failure = null;\n  function go(e) { failure = { kind: 'unavailable', reason: String(e) }; }\n</script>\n" +
      "{#if failure}<span>{failure.reason}</span>{/if}\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("failure.reason")
    ? ok("a tainted field drawn as a field is caught")
    : bad("a tainted field drawn as a field is caught", `got ${r.code}: ${r.out}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // THE BOUNDARY the field rule turns on, and it is a real file: `FmInfoPanel`
  // has a local holding a TRANSLATED sentence three lines from a store carrying a
  // tainted field of the same name. Field names are the most ordinary words in
  // the language, so a bare `{reason}` must not be assumed to be the field.
  const root = tree({
    "apps/thing/src/routes/+page.svelte":
      "<script>\n  const store = { reason: String(err) };\n</script>\n" +
      "{#if x}{@const reason = sentenceFor(read)}<span>{reason}</span>{/if}\n",
  });
  const r = run(root);
  r.code === 0 ? ok("a local of the same name as a tainted field passes") : bad("a local of the same name as a tainted field passes", r.out);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = tree(null);
  const r = run(root);
  r.code === 2 && r.out.includes("NOTHING WAS READ")
    ? ok("finding no sources at all is not a pass")
    : bad("finding no sources at all is not a pass", `got ${r.code}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The queue can only shrink. A carried file that grows a new one fails, which
  // is the whole reason the exception is a COUNT and not a filename.
  const root = tree({
    "apps/settings/src/routes/display/+page.svelte":
      "<script>\n  let applyError = null;\n  function go(e) { applyError = String(e); }\n</script>\n" +
      "<span>{applyError}</span>\n<span>{applyError}</span>\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("display/+page.svelte")
    ? ok("a carried file that grows a second one fails")
    : bad("a carried file that grows a second one fails", `got ${r.code}: ${r.out}`);
  rmSync(root, { recursive: true, force: true });
}

console.log(
  failures === 0
    ? "a raw error never reaches a reader without a catalogue, and the queue only shrinks"
    : `${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
