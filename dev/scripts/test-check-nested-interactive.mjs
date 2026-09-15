// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the nested-interactive gate see a control inside a list option?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses carry the weight. A palette whose action sits in the FOOTER is
// the fix this check asks for, and it has a button three lines below a
// `CommandItem` - so a gate that searched the file rather than the element would
// report every corrected palette in the tree and be switched off by the end of the
// week. The three real instances all had the control INSIDE the row, which is what
// this tracks.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-nested-interactive.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component) {
  const dir = mint("arlen-nested-interactive-");
  try {
    const at = path.join(dir, "apps", "demo", "src");
    mkdirSync(at, { recursive: true });
    writeFileSync(path.join(at, "Palette.svelte"), component, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("nested interactive:");

{
  // The terminal's quick-connect row, as it stood on 16 September.
  const r = gateOver(`<CommandItem value="recent-1" onSelect={pick}>
  <span class="addr">root@10.0.0.5</span>
  <button class="promote" aria-label="Save">
    <Star size={13} />
  </button>
</CommandItem>`);
  check("a button inside a CommandItem is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the component", r.out.includes("CommandItem"), r.out.trim().split("\n")[0]);
}

{
  // The launcher's, where the control was inside an inline snippet.
  const r = gateOver(`<CommandItem value="clip-1" onSelect={copy}>
  <Result title="x">
    {#snippet trailing()}
      <button class="inline" aria-label="Remove"><Trash2 /></button>
    {/snippet}
  </Result>
</CommandItem>`);
  check("a control inside an inline snippet counts", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<SelectItem value="a"><a href="/x">link</a></SelectItem>`);
  check("a SelectItem holding a link is caught too", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  // THE FIX, and it must pass: the row holds text, the action is in the footer.
  const r = gateOver(`<Command bind:value={highlighted}>
  <CommandList>
    <CommandItem value="recent-1" onSelect={pick}>
      <span class="addr">root@10.0.0.5</span>
    </CommandItem>
  </CommandList>
  <div class="foot">
    <button onclick={save}>Save this host</button>
  </div>
</Command>`);
  check("the footer-action fix passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 component(s)"),
        r.out.trim().split("\n").pop());
}

{
  // A self-closing item holds nothing, and the button after it is a sibling.
  const r = gateOver(`<CommandItem value="a" onSelect={go} />
<button onclick={other}>elsewhere</button>`);
  check("a self-closing item does not swallow what follows", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // An item on one line is closed on that line.
  const r = gateOver(`<CommandItem value="a"><span>text</span></CommandItem>
<button onclick={other}>elsewhere</button>`);
  check("a one-line item does not swallow what follows", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a control inside a list option");
