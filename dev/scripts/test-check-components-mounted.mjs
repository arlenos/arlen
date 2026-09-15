// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-components-mounted.
//
// The case it was written from: three components nothing renders, sitting in the
// tree looking like features - compiling, carrying catalogue strings, measured by
// no sweep because no route reaches them.
//
// Run: node dev/scripts/test-check-components-mounted.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-components-mounted.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-mounted-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-components-mounted:");

check(
  "a component a page imports and tags passes",
  {
    "apps/demo/src/lib/Widget.svelte": "<p>hi</p>\n",
    "apps/demo/src/routes/+page.svelte":
      '<script>import Widget from "$lib/Widget.svelte";</script>\n<Widget />\n',
  },
  (c) => c === 0,
);

check(
  "a component nothing imports is a finding",
  {
    "apps/demo/src/lib/Widget.svelte": "<p>hi</p>\n",
    "apps/demo/src/routes/+page.svelte": "<p>nothing here</p>\n",
  },
  (c, out) => c === 1 && out.includes("Widget.svelte"),
);

// A kit component reaches its apps through a barrel, which is an import by path
// and no tag anywhere in this tree.
check(
  "a re-export from an index counts as mounted",
  {
    "sdk/ui-kit/src/lib/components/ui/thing/Thing.svelte": "<p>hi</p>\n",
    "sdk/ui-kit/src/lib/components/ui/thing/index.ts":
      'export { default as Thing } from "./Thing.svelte";\n',
  },
  (c) => c === 0,
);

// Routes are mounted by the router, not by an import.
check(
  "a route file is not asked who imports it",
  {
    "apps/demo/src/routes/+page.svelte":
      '<script>import Widget from "$lib/Widget.svelte";</script>\n<Widget />\n',
    "apps/demo/src/lib/Widget.svelte": "<p>hi</p>\n",
  },
  (c) => c === 0,
);

check(
  "a tree with no components refuses rather than passing",
  { "apps/demo/src/lib/store.ts": "export const x = 1;\n" },
  (c, out) => c === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("a component nothing renders is named or it is a finding");
