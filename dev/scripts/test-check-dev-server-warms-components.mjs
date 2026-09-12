// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-dev-server-warms-components.
//
// The case it was written from: an app ships a dev server that has not been told
// to pre-transform its components, so it can hand the browser a component's own
// source as that component's stylesheet.
//
// The rest are the lines that are easy to draw wrong. A warmup that names
// something other than this app's `.svelte` files is not this warmup. An app with
// no vite config is not an app with a dev server. And an empty tree must refuse
// rather than report a clean sweep of nothing.
//
// Run: node dev/scripts/test-check-dev-server-warms-components.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dev-server-warms-components.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-warmup-");
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

const config = (server) =>
  "import { defineConfig } from 'vite';\n" +
  "export default defineConfig({\n" +
  `  server: {\n${server}    port: 1420,\n  },\n` +
  "});\n";

const page = "<p>a surface</p>\n";

console.log("check-dev-server-warms-components:");

check(
  "a dev server that does not warm its components is a finding",
  {
    "apps/demo/vite.config.js": config(""),
    "apps/demo/src/routes/+page.svelte": page,
  },
  (code, out) => code === 1 && out.includes("apps/demo/vite.config.js"),
);

check(
  "the same app passes once the warmup is there",
  {
    "apps/demo/vite.config.js": config(
      '    warmup: { clientFiles: ["./src/**/*.svelte"] },\n',
    ),
    "apps/demo/src/routes/+page.svelte": page,
  },
  (code) => code === 0,
);

// A warmup that names something else entirely warms something else entirely.
check(
  "a warmup that does not reach this app's components is not this warmup",
  {
    "apps/demo/vite.config.js": config(
      '    warmup: { clientFiles: ["./src/entry.ts"] },\n',
    ),
    "apps/demo/src/routes/+page.svelte": page,
  },
  (code, out) => code === 1 && out.includes("apps/demo/vite.config.js"),
);

// A directory under `apps/` with no vite config has no dev server to warm, so it
// is not this gate's business - a Rust-only helper crate, say.
check(
  "an app with no vite config is not a finding",
  {
    "apps/demo/vite.config.js": config(
      '    warmup: { clientFiles: ["./src/**/*.svelte"] },\n',
    ),
    "apps/demo/src/routes/+page.svelte": page,
    "apps/helper/src/lib.rs": "fn main() {}\n",
  },
  (code) => code === 0,
);

check(
  "an empty tree refuses rather than passing",
  { "README.md": "no apps here\n" },
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
console.log("a dev server that can serve source as a stylesheet is a finding, and every way of not being one is read");
