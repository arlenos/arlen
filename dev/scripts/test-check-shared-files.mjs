// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The protocol-copy comparison must catch a divergence between ANY two copies of
// a Wayland protocol, not just the two that were easiest to reach.
//
// The gate's first version globbed the shell's protocol directory and compared
// each file against the compositor's. That silently missed
// `sdk/tauri-plugin-menu/protocols/arlen-titlebar-v1.xml`, a third copy that
// generates its own client bindings - so the titlebar protocol had three copies
// and two were watched, while the output said "identical". The third-copy case
// below is that gap. It cannot be run against the old version, which had no way
// to be pointed at a fixture, but it is uncovered there by construction: that
// loop iterated the shell's directory and required a compositor copy of the same
// name, so a divergence between two in-repo copies was never a comparison it made.
//
// The fixtures are real git repos because the gate collects its subjects with
// `git ls-files`, deliberately: an untracked stray copy is not part of the
// contract, and a check that walked the filesystem would flag build output.
//
// Run: node dev/scripts/test-check-shared-files.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-shared-files.py");

const failures = [];

// The gate also checks a writer/reader struct pair, which is not what these cases
// are about; the fixtures carry a satisfied one so a failure here can only come
// from the protocol comparison.
const PAIR = {
  "daemons/installd/installd/src/lock.rs": "pub struct LockEntry {\n    pub id: String,\n}\n",
  "store-backend/src/discover.rs": "pub struct InstalledEntry {\n    pub id: String,\n}\n",
};

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-shared-gate-"));
  for (const [rel, body] of Object.entries({ ...PAIR, ...files })) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  const git = (...a) => spawnSync("git", ["-C", dir, ...a], { encoding: "utf8" });
  git("init", "-q");
  git("add", "-A");
  return dir;
}

function run(dir, compositor) {
  const r = spawnSync("python3", [GATE, dir], {
    encoding: "utf8",
    // Point the cross-repo half at a checkout that is there, or at one that is
    // not, which the gate reports rather than counting as agreement.
    env: { ...process.env, COMPOSITOR_PATH: compositor ?? join(dir, "no-compositor") },
  });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, dir, compositor, expect) {
  const r = run(dir, compositor);
  const ok = expect(r.code, r.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...r });
  rmSync(dir, { recursive: true, force: true });
}

const XML = '<protocol name="arlen_titlebar_v1"><interface name="a"/></protocol>\n';
const DRIFTED = XML.replace("<interface", "<interface_drifted");

const SHELL = "apps/desktop-shell/src-tauri/protocols/arlen-titlebar-v1.xml";
const PLUGIN = "sdk/tauri-plugin-menu/protocols/arlen-titlebar-v1.xml";

console.log("check-shared-files:");

check(
  "two agreeing copies pass",
  tree({ [SHELL]: XML, [PLUGIN]: XML }),
  null,
  (code) => code === 0,
);

// The case the shell-versus-compositor version could not see at all.
check(
  "a third in-repo copy that has drifted is caught",
  tree({ [SHELL]: XML, [PLUGIN]: DRIFTED }),
  null,
  (code, out) => code === 1 && out.includes("tauri-plugin-menu"),
);

// Written out rather than squeezed into `check`, because this case needs two
// trees and the helper only cleans up one.
{
  const comp = tree({ "resources/protocols/arlen-titlebar-v1.xml": DRIFTED });
  check(
    "a drifted compositor copy is caught across the repo boundary",
    tree({ [SHELL]: XML }),
    comp, // the gate appends resources/protocols itself
    (code, out) => code === 1 && out.includes("arlen-titlebar-v1.xml"),
  );
  rmSync(comp, { recursive: true, force: true });
}

// An absent checkout must read as "not compared", never as agreement - that is
// how the one real wire divergence in this system survived for months.
check(
  "an absent compositor checkout is reported, not counted as agreement",
  tree({ [SHELL]: XML }),
  "/nonexistent/compositor",
  (code, out) => code === 0 && out.includes("NOT CHECKED"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all shared-files gate cases passed");
