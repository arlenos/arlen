// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the rail gate see a sidebar that announces itself as nothing?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `apps/` would be visible to its
// neighbours mid-run.
//
// The near-misses are what the shape rests on. `<SidebarContent>` and the other
// eight primitives all start with the same seven letters, so a scan that does not
// require a word boundary reads every one of them as a rail. A search-only rail
// is named with `role="search"`, not a `<nav>`. And the two rails in the other
// lane must not turn this gate red for the lane that cannot fix them.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-rail-landmark.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

function gateOver(component, where = ["apps", "demo", "src", "lib", "components"]) {
  const dir = mint("arlen-rail-landmark-");
  try {
    const at = path.join(dir, ...where);
    mkdirSync(at, { recursive: true });
    writeFileSync(path.join(at, "Rail.svelte"), component, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("rail landmark:");

{
  // Eight apps looked exactly like this.
  const r = gateOver(`<Sidebar collapsible="icon">
  <SidebarContent>
    <SidebarMenu><SidebarMenuItem>Inbox</SidebarMenuItem></SidebarMenu>
  </SidebarContent>
</Sidebar>`);
  check("a rail with no landmark is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file and line", r.out.includes("Rail.svelte:1"),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(`<Sidebar>
  <SidebarContent>
    <nav aria-label={$t("x.nav")}>
      <SidebarMenu><SidebarMenuItem>Inbox</SidebarMenuItem></SidebarMenu>
    </nav>
  </SidebarContent>
</Sidebar>`);
  check("a named nav passes", r.code === 0, r.out.trim().split("\n").pop());
  check("and the gate actually read the rail", r.out.includes("1 rail(s)"),
        r.out.trim().split("\n").pop());
}

{
  // A rail that is only a search box takes the search role instead.
  const r = gateOver(`<Sidebar>
  <SidebarHeader><div class="wrap" role="search"><SearchField /></div></SidebarHeader>
</Sidebar>`);
  check("a search-only rail passes on role=search", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // The nine other primitives share the prefix; only the container is a rail.
  const r = gateOver(`<SidebarMenu><SidebarMenuItem>Inbox</SidebarMenuItem></SidebarMenu>`);
  check("a file with only the sub-primitives is not a rail", r.code === 0,
        r.out.trim().split("\n").pop());
  check("and it reads as no rails at all", r.out.includes("0 rail(s)"),
        r.out.trim().split("\n").pop());
}

{
  // The other lane's two rails have this defect and are not this gate's to fail.
  const r = gateOver(`<Sidebar><SidebarContent>x</SidebarContent></Sidebar>`,
                     ["apps", "harness", "src", "lib", "components"]);
  check("a rail in the other lane is left alone", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a rail a reader cannot jump to");
