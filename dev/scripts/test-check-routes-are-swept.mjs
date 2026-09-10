// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-routes-are-swept.
//
// The case it was written from: a route lands on disk and nobody adds it to the
// sweep table, so it renders for the first time when a person opens it.
//
// The rest are the lines that were easy to draw wrong. A group segment is not
// part of the URL. A dynamic segment cannot be matched literally, because the
// table has to pin a real value to render anything. A row that names the route
// with a click selector, a query or a host still names the route. And an empty
// tree must refuse rather than report a clean sweep of nothing.
//
// Run: node dev/scripts/test-check-routes-are-swept.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-routes-are-swept.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-routes-");
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

const tableWith = (row) =>
  "#!/usr/bin/env bash\nAPPS=(\n" + `  "${row}"\n` + ")\n";

const page = "<p>a surface</p>\n";

console.log("check-routes-are-swept:");

check(
  "a route the table does not name is a finding",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith("demo /"),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/settings/+page.svelte": page,
  },
  (code, out) => code === 1 && out.includes("demo /settings"),
);

check(
  "the same tree passes once the row names it",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith("demo /|/settings"),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/settings/+page.svelte": page,
  },
  (code) => code === 0,
);

// A row usually carries more than the route: a click selector, a locale, a host
// fixture. All three still name the route.
check(
  "a row with a selector, a query and a host still names its route",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith(
      "demo /::[data-x=y]|/settings?locale=de@@demo-refuses-save",
    ),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/settings/+page.svelte": page,
  },
  (code) => code === 0,
);

// `(app)` is a SvelteKit grouping directory. It shapes which layout applies and
// never appears in the URL, so reading it as a path segment reported a route
// nobody could ever type.
check(
  "a group segment is not part of the route",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith("demo /|/settings"),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/(app)/settings/+page.svelte": page,
  },
  (code) => code === 0,
);

// The table cannot name `[id]`; it has to pin a real one to render anything.
check(
  "a dynamic segment is matched by the concrete value the table pins",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith("demo /|/app/wp.tally"),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/app/[id]/+page.svelte": page,
  },
  (code) => code === 0,
);

// And the wildcard must not swallow depth: a two-segment row does not name a
// three-segment route.
check(
  "a shorter named route does not cover a deeper one",
  {
    "dev/screenshot/sweep-render-all.sh": tableWith("demo /|/app/wp.tally"),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/demo/src/routes/app/[id]/files/+page.svelte": page,
  },
  (code, out) => code === 1 && out.includes("/app/[id]/files"),
);

check(
  "a tree with no render table refuses rather than passing",
  { "apps/demo/src/routes/+page.svelte": page },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a tree with no app refuses rather than passing",
  { "dev/screenshot/sweep-render-all.sh": tableWith("demo /") },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("a route that ships without a sweep row is a finding, and every way a row can name one is read");
