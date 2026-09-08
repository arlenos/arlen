// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the dead-wrapper check: watch it fail on each shape it claims
// to catch, and pass on the shapes it must not.
//
// Fixture trees, not the repo, so it keeps working as the carried list shrinks.
// The repo is asked one thing at the end: that the scan reads it at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-command-wrappers-used.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** One app whose frontend is `files`: a map of relative path to contents. */
function tree(files) {
  const root = mint("command-wrappers-");
  const app = join(root, "apps/example/src");
  for (const [rel, text] of Object.entries(files)) {
    const path = join(app, rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
  }
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const WRAPPER = `import { invoke } from "@tauri-apps/api/core";
export async function loadThing(): Promise<void> {
  await invoke("load_thing");
}
`;

// The shape the notification history had: a wrapper naming a real command, and
// nothing anywhere that calls the wrapper.
{
  const root = tree({ "lib/stores/thing.ts": WRAPPER });
  const { code, out } = run(root);
  check("a wrapper nothing calls fails", code === 1 && out.includes("loadThing"));
  cleanup(root);
}

// Called from a component: the ordinary wired case.
{
  const root = tree({
    "lib/stores/thing.ts": WRAPPER,
    "lib/components/Thing.svelte": `<script lang="ts">
  import { loadThing } from "$lib/stores/thing";
  loadThing();
</script>
`,
  });
  const { code } = run(root);
  check("a wrapper a component calls passes", code === 0);
  cleanup(root);
}

// Used only inside its own module. Generous on purpose: a helper its own file
// calls once is wired, and treating that as dead would teach people to add
// exceptions rather than callers.
{
  const root = tree({
    "lib/stores/thing.ts": `${WRAPPER}
export async function refresh(): Promise<void> {
  await loadThing();
}
`,
  });
  const { code } = run(root);
  check("a wrapper its own file calls passes", code === 0);
  cleanup(root);
}

// The marker the command check honours, read the same way here.
{
  const root = tree({
    "lib/stores/thing.ts": `import { invoke } from "@tauri-apps/api/core";

/// NO CALLER: the debugging affordance, deliberately available.
export async function loadThing(): Promise<void> {
  await invoke("load_thing");
}
`,
  });
  const { code } = run(root);
  check("a wrapper that excuses itself passes", code === 0);
  cleanup(root);
}

// A store with no invoke in it is not this check's business, however unused.
{
  const root = tree({
    "lib/stores/thing.ts": `export function pureHelper(n: number): number {
  return n + 1;
}
`,
  });
  const { code } = run(root);
  check("an unused function that invokes nothing passes", code === 0);
  cleanup(root);
}

// An arrow-form export, since half the stores in the tree are written that way.
{
  const root = tree({
    "lib/stores/thing.ts": `import { invoke } from "@tauri-apps/api/core";
export const loadThing = async (): Promise<void> => {
  await invoke("load_thing");
};
`,
  });
  const { code, out } = run(root);
  check("an unused arrow wrapper fails too", code === 1 && out.includes("loadThing"));
  cleanup(root);
}

// A carried count that is too high has to come down, the same ratchet the
// command check runs: a stale allowance is a place a new one hides.
{
  const r = spawnSync("python3", [CHECK], { encoding: "utf8" });
  const out = (r.stdout || "") + (r.stderr || "");
  check("the repo scan reads the tree", r.status === 0 && out.includes("app(s) scanned"));
}

console.log(failures === 0 ? "all ok" : `${failures} failure(s)`);
process.exit(failures === 0 ? 0 : 1);
