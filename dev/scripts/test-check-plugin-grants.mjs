#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-plugin-grants.py. The case that matters is the REAL one from 16 August:
// an app calls toolbar.setBreadcrumb and its capability file grants only theme and locale, so
// every navigation is refused at runtime. The check now passes against the repo because both
// apps were fixed, which is exactly when a check needs proving on the state it was written for.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-plugin-grants.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// A miniature of the real API module: two objects, one command each.
const API = `
const PLUGIN = "plugin:arlen-shell";

export const toolbar = {
  async setBreadcrumb(items: BreadcrumbItem[]): Promise<void> {
    return invoke(\`\${PLUGIN}|toolbar_set_breadcrumb\`, { items });
  },
  async clear(): Promise<void> {
    return invoke(\`\${PLUGIN}|toolbar_clear\`);
  },
};

export const presence = {
  async set(params: PresenceParams): Promise<void> {
    return invoke(\`\${PLUGIN}|presence_set\`, { params });
  },
};
`;

function tree({ source, permissions }) {
  const root = mkdtempSync(join(tmpdir(), "plugin-grants-"));
  mkdirSync(join(root, "sdk/tauri-plugin-shell"), { recursive: true });
  writeFileSync(join(root, "sdk/tauri-plugin-shell/index.ts"), API);
  mkdirSync(join(root, "apps/demo/src/lib"), { recursive: true });
  mkdirSync(join(root, "apps/demo/src-tauri/capabilities"), { recursive: true });
  writeFileSync(join(root, "apps/demo/src/lib/topbar.ts"), source);
  writeFileSync(
    join(root, "apps/demo/src-tauri/capabilities/default.json"),
    JSON.stringify({ identifier: "default", windows: ["main"], permissions }, null, 2),
  );
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

const CALLS_BREADCRUMB = `import { toolbar } from "@arlen/tauri-plugin-shell";
export function push(state) {
  void toolbar.setBreadcrumb(crumbs(state));
}
`;

{
  const root = tree({ source: CALLS_BREADCRUMB, permissions: ["arlen-shell:allow-theme-get"] });
  const rc = run(root);
  rc === 1
    ? ok("the real 16 August shape is caught")
    : bad("the real 16 August shape is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = tree({
    source: CALLS_BREADCRUMB,
    permissions: ["arlen-shell:allow-theme-get", "arlen-shell:allow-toolbar-set-breadcrumb"],
  });
  const rc = run(root);
  rc === 0
    ? ok("granting the command it calls passes")
    : bad("granting the command it calls passes", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // A method the app does NOT call must not be demanded, or the check would push
  // every consumer toward granting the whole plugin.
  const root = tree({
    source: CALLS_BREADCRUMB,
    permissions: ["arlen-shell:allow-toolbar-set-breadcrumb"],
  });
  const rc = run(root);
  rc === 0
    ? ok("an uncalled command is not demanded")
    : bad("an uncalled command is not demanded", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // Importing one object must not make another object's methods required, even
  // when the name matches a method name elsewhere.
  const source = `import { presence } from "@arlen/tauri-plugin-shell";
export function mark() {
  void presence.set({ kind: "editing" });
}
`;
  const root = tree({ source, permissions: ["arlen-shell:allow-presence-set"] });
  const rc = run(root);
  rc === 0
    ? ok("only the imported object's calls are required")
    : bad("only the imported object's calls are required", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = mkdtempSync(join(tmpdir(), "plugin-grants-noapi-"));
  mkdirSync(join(root, "apps"), { recursive: true });
  const rc = run(root);
  rc === 1
    ? ok("a missing plugin API is not a pass")
    : bad("a missing plugin API is not a pass", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
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
console.log("a call without its grant fails the check");
