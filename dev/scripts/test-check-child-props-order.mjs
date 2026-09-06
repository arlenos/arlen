// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the props-order gate see a click handler that a later spread replaces?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses are why this parses tags rather than grepping lines. A handler
// and the spread routinely sit on different lines of the same tag; a handler
// body contains `=>`, whose `>` stops a naive tag scan; and a spread outside any
// `child` snippet is ordinary Svelte, not this fault.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-child-props-order.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component, where = ["apps", "demo", "src"]) {
  const dir = mint("arlen-child-props-order-");
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

console.log("child props order:");

{
  // The exact shape of the now-playing art button, whose panel was unreachable.
  const r = gateOver(`<Tooltip.Trigger>
  {#snippet child({ props })}
    <button
      class="mpris-art"
      onclick={() => togglePopover("mpris")}
      {...props}
    >art</button>
  {/snippet}
</Tooltip.Trigger>`);
  check("a handler before the spread is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file and the handler",
        r.out.includes("Thing.svelte:5") && r.out.includes("`onclick`"),
        r.out.trim().split("\n")[0]);
}

{
  // The fix: spread first, then a handler that also runs the component's own.
  const r = gateOver(`<Tooltip.Trigger>
  {#snippet child({ props })}
    <button {...props} onclick={(e) => { props.onclick?.(e); go(); }}>art</button>
  {/snippet}
</Tooltip.Trigger>`);
  check("the spread-first form passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the file", r.out.includes("1 component(s)"),
        r.out.trim().split("\n").pop());
}

{
  // A multi-line handler whose body holds `=>` and `>`. A tag scan that stops at
  // the first `>` never reaches the spread and reports nothing at all.
  const r = gateOver(`<Tooltip.Trigger>
  {#snippet child({ props })}
    <button
      onclick={(e) => {
        if (n > 1) e.stopPropagation();
        mute();
      }}
      {...props}
    >x</button>
  {/snippet}
</Tooltip.Trigger>`);
  check("an arrow body with `>` in it does not hide the fault", r.code === 1,
        r.out.trim().split("\n")[0]);
}

{
  // A spread with no handler before it, which is most of the tree.
  const r = gateOver(`<Tooltip.Trigger>
  {#snippet child({ props })}
    <button {...props}>x</button>
  {/snippet}
</Tooltip.Trigger>`);
  check("a bare spread is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // Outside a `child` snippet these props are the component's own, and putting a
  // handler before them is the ordinary override-the-default form.
  const r = gateOver(`<button onclick={go} {...rest}>x</button>`);
  check("a spread outside a child snippet is not this fault", r.code === 0,
        r.out.trim().split("\n").pop());
}

{
  // Two elements in one snippet: the fault is on the second.
  const r = gateOver(`<Tooltip.Trigger>
  {#snippet child({ props })}
    <span class="wrap">
      <button onblur={go} {...props}>x</button>
    </span>
  {/snippet}
</Tooltip.Trigger>`);
  check("a nested element carries the finding too", r.code === 1 && r.out.includes("`onblur`"),
        r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a control whose own handler a later spread replaces");
