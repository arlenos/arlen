#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-dev-ports.py. The collision it was written for is silent - one
// app's interface renders inside another app's window - so each shape it claims to
// catch is demonstrated failing here rather than described.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-dev-ports.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

/// Each app is [server, hmr, devUrl].
function tree(apps) {
  const root = mint("dev-ports-");
  for (const [name, [server, hmr, dev]] of Object.entries(apps)) {
    mkdirSync(join(root, "apps", name, "src-tauri"), { recursive: true });
    writeFileSync(
      join(root, "apps", name, "vite.config.js"),
      `export default { server: { port: ${server}, strictPort: true, hmr: { port: ${hmr} } } };\n`,
    );
    writeFileSync(
      join(root, "apps", name, "src-tauri", "tauri.conf.json"),
      JSON.stringify({ build: { devUrl: `http://localhost:${dev}` } }, null, 2),
    );
  }
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

// A frontend under `daemons/` with a TypeScript vite config: two things this
// check could not see at once, which is how the picker came to serve on the same
// port as Settings with `strictPort` on both.
{
  const root = mint("dev-ports-daemon-");
  mkdirSync(join(root, "apps", "one", "src-tauri"), { recursive: true });
  writeFileSync(
    join(root, "apps", "one", "vite.config.js"),
    "export default { server: { port: 1421, strictPort: true, hmr: { port: 1422 } } };\n",
  );
  writeFileSync(
    join(root, "apps", "one", "src-tauri", "tauri.conf.json"),
    JSON.stringify({ build: { devUrl: "http://localhost:1421" } }, null, 2),
  );
  const ui = join(root, "daemons", "some-portal", "its-ui");
  mkdirSync(join(ui, "src-tauri"), { recursive: true });
  writeFileSync(
    join(ui, "vite.config.ts"),
    "export default { server: { port: 1421, strictPort: true, hmr: { port: 1422 } } };\n",
  );
  writeFileSync(
    join(ui, "src-tauri", "tauri.conf.json"),
    JSON.stringify({ build: { devUrl: "http://localhost:1421" } }, null, 2),
  );
  const code = run(root);
  if (code === 1) ok("a daemon frontend on an app's port is caught, TypeScript config and all");
  else bad("a daemon frontend on an app's port is caught, TypeScript config and all", `exit ${code}`);
  cleanup(root);
}


const cases = [
  ["distinct ports pass", { a: [1420, 1520, 1420], b: [1422, 1522, 1422] }, 0],
  ["two apps on one server port is caught", { a: [1429, 1529, 1429], b: [1429, 1530, 1429] }, 1],
  ["an HMR port on another app's server port is caught", { a: [1420, 1421, 1420], b: [1421, 1521, 1421] }, 1],
  ["two apps sharing an HMR port is caught", { a: [1420, 1520, 1420], b: [1422, 1520, 1422] }, 1],
  ["a window loading a port its own build does not serve is caught", { a: [1420, 1520, 1429] }, 1],
];

for (const [name, apps, expect] of cases) {
  const root = tree(apps);
  const rc = run(root);
  rc === expect ? ok(name) : bad(name, `expected ${expect}, got ${rc}`);
  cleanup(root);
}

{
  const root = mint("dev-ports-empty-");
  mkdirSync(join(root, "apps"), { recursive: true });
  const rc = run(root);
  rc === 1 ? ok("no ports at all is not a pass") : bad("no ports at all is not a pass", `got ${rc}`);
  cleanup(root);
}

{
  const rc = run(join(here, "..", ".."));
  rc === 0
    ? ok("the repository itself passes")
    : bad("the repository itself passes", `expected 0, got ${rc}`);
}

if (failures) {
  console.log(`\n${failures} case(s) failed`);
  process.exit(1);
}
console.log("a shared server port, a shared HMR port, an HMR port on a server port and a self-inconsistent app are all caught");
