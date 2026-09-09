#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-grants-exercised.py. The real case is from 9 September: the
// file manager held `arlen-shell:allow-shortcuts-clear` beside its register
// grant and never called `shortcuts.clear()`. The check passes against the repo
// now because the grant came out, which is exactly when a check needs proving on
// the state it was written for.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-grants-exercised.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// A miniature of the real API module: one object, two commands.
const API = `
const PLUGIN = "plugin:arlen-shell";
export const shortcuts = {
  async register(list) {
    return invoke(\`\${PLUGIN}|shortcuts_register\`, { shortcuts: list });
  },
  async clear() {
    return invoke(\`\${PLUGIN}|shortcuts_clear\`);
  },
};
`;

function tree(grants, frontend) {
  const dir = mint("arlen-grantsx-");
  mkdirSync(join(dir, "sdk/tauri-plugin-shell"), { recursive: true });
  writeFileSync(join(dir, "sdk/tauri-plugin-shell/index.ts"), API);
  const cap = join(dir, "apps/demo/src-tauri/capabilities");
  mkdirSync(cap, { recursive: true });
  writeFileSync(
    join(cap, "default.json"),
    JSON.stringify({ permissions: grants.map((g) => `arlen-shell:allow-${g}`) }, null, 2),
  );
  mkdirSync(join(dir, "apps/demo/src/lib"), { recursive: true });
  writeFileSync(join(dir, "apps/demo/src/lib/use.ts"), frontend);
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [check, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status, out: String(e.stdout ?? "") };
  }
}

// THE REAL CASE.
let dir = tree(
  ["shortcuts-register", "shortcuts-clear"],
  'import { shortcuts } from "@arlen/tauri-plugin-shell";\nawait shortcuts.register([]);\n',
);
let r = run(dir);
r.code === 1 && r.out.includes("shortcuts-clear")
  ? ok("a grant the app never calls fails")
  : bad("a grant the app never calls fails", `code ${r.code}: ${r.out}`);
cleanup(dir);

dir = tree(
  ["shortcuts-register", "shortcuts-clear"],
  'import { shortcuts } from "@arlen/tauri-plugin-shell";\nawait shortcuts.register([]);\nawait shortcuts.clear();\n',
);
r = run(dir);
r.code === 0
  ? ok("and passes once it is called")
  : bad("and passes once it is called", `code ${r.code}: ${r.out}`);
cleanup(dir);

// THE MATCHER'S OWN TRAP. A bare method name is every store in the tree; only
// the object.method pair means this surface.
dir = tree(
  ["shortcuts-register", "shortcuts-clear"],
  'import { shortcuts } from "@arlen/tauri-plugin-shell";\nawait shortcuts.register([]);\nfunction clear() {}\nclear();\n',
);
r = run(dir);
r.code === 1 && r.out.includes("shortcuts-clear")
  ? ok("a same-named local function is not the surface")
  : bad("a same-named local function is not the surface", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A DESTRUCTURED CALL defeats the pair match. There is none in the tree today;
// this case is here so the day somebody writes one it is a control failure and
// not a silent pass.
dir = tree(
  ["shortcuts-register", "shortcuts-clear"],
  'import { shortcuts } from "@arlen/tauri-plugin-shell";\nconst { register, clear } = shortcuts;\nawait register([]);\nawait clear();\n',
);
r = run(dir);
r.code === 1
  ? ok("a destructured call is reported rather than silently accepted")
  : bad("a destructured call is reported rather than silently accepted", `code ${r.code}: ${r.out}`);
cleanup(dir);

// An app with no shell grants is not this check's business.
dir = tree([], "export const nothing = 1;\n");
r = run(dir);
r.code === 0
  ? ok("an app with no shell grants passes")
  : bad("an app with no shell grants passes", `code ${r.code}: ${r.out}`);
cleanup(dir);

// An empty tree must refuse rather than pass.
dir = mint("arlen-grantsx-");
r = run(dir);
r.code === 1
  ? ok("an empty tree is a failure, not a pass")
  : bad("an empty tree is a failure, not a pass", `code ${r.code}: ${r.out}`);
cleanup(dir);

// And the repo itself.
try {
  execFileSync("python3", [check], { encoding: "utf8" });
  ok("and the repo itself passes");
} catch (e) {
  bad("and the repo itself passes", String(e.stdout ?? e));
}

console.log(
  failures === 0
    ? "a surface an app holds is one it calls, and the check says so"
    : `\n${failures} failure(s)`,
);
process.exit(failures === 0 ? 0 : 1);
