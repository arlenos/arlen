// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the id gate see a reference that resolves to no element, and an id that
// will resolve to several?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses are the reason it counts occurrences rather than collecting
// ids. A dialog hands its title id DOWN as a prop (`titleId="sh-consent-title"`),
// so the target carries no literal `id=` in this file and the reference is still
// sound; and a computed reference (`aria-labelledby={x}`) cannot be followed at
// all, so it must not be guessed at.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-id-references.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component, where = ["apps", "demo", "src"]) {
  const dir = mint("arlen-id-references-");
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

console.log("id references:");

{
  // The screenshot app's shape: a label beside a component that forwards no id.
  const r = gateOver(`<label class="source-label" for="capture-source">Source</label>
<PopoverSelect ariaLabel="Source" />`);
  check("a for that names nothing is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file, line and id",
        r.out.includes("Thing.svelte:1") && r.out.includes("capture-source"),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<label for="pick">Source</label>\n<select id="pick"></select>`);
  check("a for with a real target passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 component(s)"),
        r.out.trim().split("\n").pop());
}

{
  // The shell's dialogs: the id goes down as a prop, so there is no literal
  // `id=` here and the reference is still sound.
  const r = gateOver(`<Dialog.Content aria-labelledby="sh-consent-title">
  <ConsentCard titleId="sh-consent-title" />
</Dialog.Content>`);
  check("an id handed to a component passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // A computed reference cannot be followed and must not be guessed at.
  const r = gateOver(`<div aria-labelledby={titleId}>x</div>`);
  check("a computed reference is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // A comment is not markup, and this tree explains a fix by quoting the
  // attribute it removed - which reported the finding again from inside the
  // sentence describing its own fix.
  const r = gateOver(`<!-- was: for="capture-source", which named nothing -->
<span class="source-label">Source</span>`);
  check("an id quoted in a comment is not a reference", r.code === 0,
        r.out.trim().split("\n").pop());
}

{
  // Two ids in one reference: both have to land.
  const r = gateOver(`<input aria-describedby="hint err" />\n<p id="hint">h</p>`);
  check("a second id in the same reference is checked too",
        r.code === 1 && r.out.includes('"err"'), r.out.trim().split("\n")[0]);
}

{
  // The other half: a literal id inside an each renders once per row.
  const r = gateOver(`{#each lines as line (line.surface)}
  <Button id="sent-exposure-fix">Fix</Button>
{/each}`);
  check("an id inside an each block is caught",
        r.code === 1 && r.out.includes("each block"), r.out.trim().split("\n")[0]);
}

{
  // Built from the row's key, so each row gets its own.
  const r = gateOver(
    "{#each lines as line (line.surface)}\n" +
    "  <Button id={`fix-\u0024{line.surface}`}>Fix</Button>\n" +
    "{/each}");
  check("an id built from the row's key passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // Outside the block again: the depth has to come back down.
  const r = gateOver(`{#each lines as line (line.id)}
  <span>{line.id}</span>
{/each}
<p id="after">done</p>`);
  check("an id after the block is not inside it", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds an id that names nothing and one that names too much");
