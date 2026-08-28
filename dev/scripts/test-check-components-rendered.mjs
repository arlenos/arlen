// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the component-reachability gate.
//
// The cases that matter are not "does it print OK". They are the four ways this
// walk could quietly stop finding orphans: a component reached only through a
// chain, one reached only from a harness route, one reached through SvelteKit's
// import-the-emitted-name convention, and an exception that outlived its reason.
// Each of the last three has already gone wrong once in this file's short life.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-components-rendered.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(files) {
  const dir = mint("components-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("components rendered:");

{
  const r = run({
    "apps/demo/src/routes/+page.svelte": 'import Bar from "$lib/components/Bar.svelte";',
    "apps/demo/src/lib/components/Bar.svelte": "<div>bar</div>",
  });
  check("a component a route imports passes", r.code === 0);
}
{
  // Two hops, because a one-level check would pass a real orphan behind a live
  // parent and fail a live grandchild.
  const r = run({
    "apps/demo/src/routes/+page.svelte": 'import Bar from "$lib/components/Bar.svelte";',
    "apps/demo/src/lib/components/Bar.svelte": 'import Deep from "./Deep.svelte";',
    "apps/demo/src/lib/components/Deep.svelte": "<div>deep</div>",
  });
  check("a component reached through a chain passes", r.code === 0);
}
{
  const r = run({
    "apps/demo/src/routes/+page.svelte": "<div>nothing imported</div>",
    "apps/demo/src/lib/components/Lonely.svelte": "<div>lonely</div>",
  });
  check("a component nothing imports is caught", r.code === 1 && r.out.includes("Lonely"));
}
{
  // Mutual imports are not reachability. Without a walk from the roots, these two
  // vouch for each other forever.
  const r = run({
    "apps/demo/src/routes/+page.svelte": "<div>nothing imported</div>",
    "apps/demo/src/lib/components/A.svelte": 'import B from "./B.svelte";',
    "apps/demo/src/lib/components/B.svelte": 'import A from "./A.svelte";',
  });
  check("two orphans importing each other are still orphans", r.code === 1 && r.out.includes("A.svelte"));
}
{
  // The case this gate was written for: reachable only from the screenshot route
  // someone added while working on it.
  const r = run({
    "apps/demo/src/routes/+page.svelte": "<div>nothing imported</div>",
    "apps/demo/src/routes/_rendertest/+page.svelte": 'import Orphan from "$lib/components/Orphan.svelte";',
    "apps/demo/src/lib/components/Orphan.svelte": "<div>orphan</div>",
  });
  check("reachable only from a harness route is still unreached", r.code === 1 && r.out.includes("Orphan"));
}
{
  // SvelteKit imports the EMITTED name. Missing this reported nine live
  // quick-settings tiles as rendered by nobody.
  const r = run({
    "apps/demo/src/routes/+page.svelte": 'import { reg } from "$lib/registry.js";',
    "apps/demo/src/lib/registry.ts": 'import Tile from "./components/Tile.svelte";',
    "apps/demo/src/lib/components/Tile.svelte": "<div>tile</div>",
  });
  check("a .js specifier resolving to a .ts file is followed", r.code === 0);
}
{
  // The module direction. A store is the same question with a different
  // extension, and the walk was already visiting these files without reporting
  // them - which is how `applets.ts` sat in the tree since Phase 4 holding a
  // battery level nobody measured.
  const r = run({
    "apps/demo/src/routes/+page.svelte": "<div>nothing imported</div>",
    "apps/demo/src/lib/stores/dead.ts": "export const x = 1;",
  });
  check("a lib module nothing imports is caught", r.code === 1 && r.out.includes("dead.ts"));
}
{
  const r = run({
    "apps/demo/src/routes/+page.svelte": 'import { x } from "$lib/stores/live";',
    "apps/demo/src/lib/stores/live.ts": "export const x = 1;",
  });
  check("a lib module a route imports passes", r.code === 0);
}
{
  const r = run({ "README.md": "no frontend here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe component-reachability gate holds");
process.exit(failures ? 1 : 0);
