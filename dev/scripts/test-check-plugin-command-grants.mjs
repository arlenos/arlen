// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-plugin-command-grants.
//
// The defect it guards happened at scale: ten apps invoked
// `plugin:arlen-shell|menu_register` without granting it, Tauri rejected every
// call, and every call discarded the rejection with `.catch(() => {})`.
//
// A NOTE ABOUT THE CLEANUP BELOW, because the first draft of this file deleted
// the repository. It ran the gate against the real tree for the "as it stands"
// case and then handed that path to the same `rmSync(dir, { recursive: true })`
// the temp-fixture cases use. `rimraf` walked the repo root and stopped only when
// it hit a directory it could not write. So: `mint()` is the ONLY way a path
// enters `cleanup()`, cleanup refuses anything it did not mint, and the
// repository case never goes near it.
//
// Run: node dev/scripts/test-check-plugin-command-grants.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const GATE = new URL("./check-plugin-command-grants.py", import.meta.url).pathname;
const REPO = new URL("../..", import.meta.url).pathname;
const failures = [];

/// Every directory this process created, and the only ones it may remove.
const minted = new Set();

/// A fresh temp directory, recorded so `cleanup` can tell it apart from a path
/// that was passed in.
function mint(prefix) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  minted.add(dir);
  return dir;
}

/// Remove a directory this file made. Refuses anything else, loudly: a helper
/// that deletes its fixture must never receive a path it did not create.
function cleanup(dir) {
  if (!minted.has(dir)) {
    console.log(`  REFUSED to remove ${dir}: not a directory this test created`);
    process.exit(1);
  }
  minted.delete(dir);
  rmSync(dir, { recursive: true, force: true });
}

/// A tree with one app: what its frontend calls, what its capability grants.
function tree({ app = "demo", calls = [], permissions = [] } = {}) {
  const dir = mint("arlen-plugingrants-");
  mkdirSync(join(dir, `apps/${app}/src/lib`), { recursive: true });
  mkdirSync(join(dir, `apps/${app}/src-tauri/capabilities`), { recursive: true });
  writeFileSync(
    join(dir, `apps/${app}/src/lib/menu.ts`),
    calls.map((c) => `void invoke("${c}", {});`).join("\n") + "\n",
  );
  writeFileSync(
    join(dir, `apps/${app}/src-tauri/capabilities/default.json`),
    JSON.stringify({ identifier: "default", permissions }, null, 2),
  );
  return dir;
}

const gateOn = (dir) => {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

/// Build a fixture, run the gate on it, remove it. The fixture is minted here, so
/// nothing a caller passes can reach `cleanup`.
function run(name, opts, expect) {
  const dir = tree(opts);
  const { code, out } = gateOn(dir);
  check(name, expect(code, out), `exit ${code}: ${out}`);
  cleanup(dir);
}

console.log("plugin command grants:");

// The repository, deliberately NOT through `run`: it is not ours to remove.
{
  const { code, out } = gateOn(REPO);
  check("the tree as it stands passes", code === 0, out);
}

run(
  "a call with no grant is caught",
  { calls: ["plugin:arlen-shell|menu_register"], permissions: ["core:default"] },
  (code, out) => code === 1 && out.includes("allow-menu-register"),
);

run(
  "the grant it needs makes it pass",
  {
    calls: ["plugin:arlen-shell|menu_register"],
    permissions: ["core:default", "arlen-shell:allow-menu-register"],
  },
  (code) => code === 0,
);

run(
  "an underscore in the command becomes a hyphen in the permission",
  {
    calls: ["plugin:arlen-shell|toolbar_set_breadcrumb"],
    permissions: ["arlen-shell:allow-toolbar-set-breadcrumb"],
  },
  (code) => code === 0,
);

run(
  "a grant for another plugin does not cover this one",
  {
    calls: ["plugin:arlen-shell|menu_register"],
    permissions: ["arlen-menu:allow-menu-register"],
  },
  (code, out) => code === 1 && out.includes("arlen-shell:allow-menu-register"),
);

// An app named in UNRESOLVED that no longer calls anything is an excuse outliving
// its reason, which is how an acknowledgement list becomes a place to hide.
run(
  "an excused app that calls nothing is reported as stale",
  { app: "settings", calls: [], permissions: [] },
  (code, out) =>
    (code === 2 && out.includes("NOTHING WAS READ")) ||
    (code === 1 && out.includes("remove the entry")),
);

{
  const dir = mint("arlen-plugingrants-empty-");
  const { code, out } = gateOn(dir);
  check(
    "a tree with no apps refuses rather than passing",
    code === 2 && out.includes("NOTHING WAS READ"),
    `exit ${code}: ${out}`,
  );
  cleanup(dir);
}

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.detail}`);
  process.exit(1);
}
console.log("a plugin call the app did not grant itself is caught, and the mapping is exact");
