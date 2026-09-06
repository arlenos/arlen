// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the dialog-name gate see a modal that announces itself as "dialog"?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses are the reason this parses tags rather than grepping. A dialog
// named through the primitive's own `<Dialog.Title>` is correct and must pass; so
// is one whose name is a Svelte expression, and `aria-label={a > b ? p : q}`
// contains a `>` that a naive tag scan would stop at.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-dialog-named.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component, where = ["apps", "demo", "src"]) {
  const dir = mint("arlen-dialog-named-");
  try {
    const at = path.join(dir, ...where);
    mkdirSync(at, { recursive: true });
    writeFileSync(path.join(at, "Thing.svelte"), component, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("dialog named:");

{
  // The real shape of the shell's five modals before 6 September.
  const r = gateOver(`<Dialog.Root>
  <Dialog.Content class="arlen-consent-card">
    <h2>Allow this?</h2>
  </Dialog.Content>
</Dialog.Root>`);
  check("an unnamed Dialog.Content is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file and line", r.out.includes("Thing.svelte:2"),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<div role="dialog" aria-modal="true"><h2>Keep this?</h2></div>`);
  check("a hand-rolled role=dialog is caught too", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<Dialog.Root>
  <Dialog.Content aria-labelledby="t"><h2 id="t">Allow this?</h2></Dialog.Content>
</Dialog.Root>`);
  check("aria-labelledby passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 component(s)"),
        r.out.trim().split("\n").pop());
}

{
  // The primitive's own naming path, which several apps outside this tree use.
  const r = gateOver(`<Dialog.Root>
  <Dialog.Content><Dialog.Title>Allow this?</Dialog.Title></Dialog.Content>
</Dialog.Root>`);
  check("a Dialog.Title names the dialog", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // A `>` inside an attribute expression. A tag scan that stops at the first one
  // reads the tag as unnamed and reports a correct component.
  const r = gateOver(`<div role="dialog" aria-label={n > 1 ? many : one}>x</div>`);
  check("a `>` inside an attribute expression does not fool it", r.code === 0,
        r.out.trim().split("\n").pop());
}

{
  // The kit's own wrapper IS the primitive; the name belongs to whoever raises it.
  const r = gateOver(`<Dialog.Content {...rest}>{@render children?.()}</Dialog.Content>`,
                     ["sdk", "ui-kit", "src", "lib", "components", "ui", "dialog"]);
  check("the kit's own dialog wrapper is skipped", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // Not a dialog at all, and the tree is full of these.
  const r = gateOver(`<div role="listbox"><span>x</span></div>`);
  check("a non-dialog role is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a modal that would be announced as an unnamed dialog");
