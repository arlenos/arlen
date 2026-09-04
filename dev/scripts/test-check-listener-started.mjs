#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-listener-started.py`. Every case here is one the check got
// WRONG on its first run and now gets right, which is why they are worth
// keeping: a scanner's false positives are how it teaches people to work around
// it rather than fix what it found.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-listener-started.py");
const REPO = join(HERE, "..", "..");

/// A tree with one app, its files written verbatim.
function tree(files) {
  const root = mint("listener-started-");
  for (const [rel, body] of Object.entries(files)) {
    const path = join(root, "apps", "demo", rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
  }
  return root;
}

function checkOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const FEED = `import { listen } from "@tauri-apps/api/event";
export async function watchThings() {
  return listen("things", () => {});
}
`;

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "a feed nothing starts is caught",
    () => tree({ "src/lib/feed.ts": FEED }),
    (code, out) => code === 1 && out.includes("watchThings"),
    true,
  ],
  [
    "a feed a component starts is fine",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+layout.svelte": `<script>\n  import { watchThings } from "$lib/feed";\n  watchThings();\n</script>\n`,
      }),
    (code) => code === 0,
    true,
  ],
  [
    // `void f();` is fire-and-forget of a promise, which is how two live feeds in
    // this tree are started. The first cut treated it as a linter-silencing
    // reference and called them dark.
    "a fire-and-forget call is a call",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+page.svelte": `<script>\n  import { watchThings } from "$lib/feed";\n  void watchThings();\n</script>\n`,
      }),
    (code) => code === 0,
    true,
  ],
  [
    // `void f;` with no parens IS only a reference, and the one real dark feed in
    // this tree is turned off exactly that way.
    "a bare reference is not a call",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+page.svelte": `<script>\n  import { watchThings } from "$lib/feed";\n  void watchThings;\n</script>\n`,
      }),
    (code, out) => code === 1 && out.includes("watchThings"),
    true,
  ],
  [
    // A Svelte action is never called in the file; the framework calls it when
    // the element mounts. Two live actions were reported as dark for this.
    "a svelte action used on an element is started",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+page.svelte": `<div use:watchThings>text</div>\n`,
      }),
    (code) => code === 0,
    true,
  ],
  [
    // The import alias case: a component importing under another name is calling
    // the function, and a scanner that missed it would report a live feed dead.
    "an aliased import counts as the caller",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+page.svelte": `<script>\n  import { watchThings as startThings } from "$lib/feed";\n  startThings();\n</script>\n`,
      }),
    (code) => code === 0,
    true,
  ],
  [
    // A comment naming the function is prose. Counting it as a call reported a
    // deliberately-dark feed as live, because the note explaining why it stays
    // off mentions it by name.
    "a comment naming it is not a call",
    () =>
      tree({
        "src/lib/feed.ts": FEED,
        "src/routes/+page.svelte": `<script>\n  // watchThings() is off until the pool is wired\n</script>\n`,
      }),
    (code, out) => code === 1 && out.includes("watchThings"),
    true,
  ],
  [
    // The body must end at its own closing brace. A fixed window reads into the
    // next function, which made two ordinary loaders look like feeds.
    "a function that does not subscribe is not a feed",
    () =>
      tree({
        "src/lib/feed.ts": `import { listen } from "@tauri-apps/api/event";
export async function loadOnce() {
  return 1;
}
export async function watchThings() {
  return listen("things", () => {});
}
`,
        "src/routes/+page.svelte": `<script>\n  import { watchThings } from "$lib/feed";\n  watchThings();\n</script>\n`,
      }),
    (code, out) => code === 0 && !out.includes("loadOnce"),
    true,
  ],
];

let failed = 0;
for (const [name, build, expect, disposable] of cases) {
  const root = build();
  const { code, out } = checkOn(root);
  if (disposable) cleanup(root);
  const ok = expect(code, out);
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed += 1;
    console.log(`     exit ${code}\n     ${out.trim().split("\n").slice(0, 3).join("\n     ")}`);
  }
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log(`\nall ${cases.length} cases behaved`);
