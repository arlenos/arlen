// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the read-scope gate must catch, and what it must leave alone.
//
// The gate decides whether an app is allowed to read what it queries. Both wrong
// answers cost: a miss ships a feature that renders an empty graph and looks like
// "you did nothing" (the shape it was written for, found in two shipped apps at
// once), and a false alarm on a label nobody actually queries trains people to
// skim a red gate. So both directions are pinned here.
//
// NOT covered: the ACCEPTED table, which is empty and lives as a module constant
// a fixture tree cannot reach. If it ever gains an entry, that entry is a claim
// about intent and belongs next to a reason, not in a test.
//
// Run: node dev/scripts/test-check-read-scope.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-read-scope.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-read-scope-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  // Both streams on every path. Reading `execFileSync`'s return value catches
  // stdout alone, so a case asserting on something the gate writes to stderr while
  // still exiting 0 would silently compare against an empty string - and the sync
  // call additionally echoes the child's stderr here, printing a wall of red above
  // an EXPECTED failure. Found twice in sibling gate tests before being fixed here.
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

const PROFILE = (labels) =>
  `[info]\napp_id = "dev.arlen.demo"\ntier = "first-party"\n\n[graph]\nread = [\n${labels
    .map((l) => `    "system.${l}.id",\n`)
    .join("")}]\n`;

const QUERIES = (labels) =>
  labels.map((l) => `let q = "MATCH (n:${l}) RETURN n.id";\n`).join("");

check(
  "a queried label that is granted passes",
  tree({
    [`${PROFILES}/dev.arlen.demo.toml`]: PROFILE(["File", "Project"]),
    "apps/demo/src-tauri/src/lib.rs": QUERIES(["File", "Project"]),
  }),
  (code) => code === 0,
);

check(
  "a queried label with no grant fails and names it",
  tree({
    [`${PROFILES}/dev.arlen.demo.toml`]: PROFILE(["File"]),
    "apps/demo/src-tauri/src/lib.rs": QUERIES(["File", "App"]),
  }),
  (code, out) => code !== 0 && out.includes("MATCH (:App)") && out.includes("system.App"),
);

check(
  "the app's core crate is read too, not just src-tauri",
  // The file manager keeps queries in both, so scanning one would half-check it.
  tree({
    [`${PROFILES}/dev.arlen.demo.toml`]: PROFILE(["File"]),
    "apps/demo/core/src/kg.rs": QUERIES(["Event"]),
  }),
  (code, out) => code !== 0 && out.includes("system.Event"),
);

check(
  "a label named only in the frontend is not reported",
  // The webview cannot query the graph; only the app's own process can, and a
  // string in a .ts file is not a read. Reporting it would be a false alarm
  // against a profile that is correct.
  tree({
    [`${PROFILES}/dev.arlen.demo.toml`]: PROFILE(["File"]),
    "apps/demo/src-tauri/src/lib.rs": QUERIES(["File"]),
    "apps/demo/src/lib/x.ts": 'const q = "MATCH (a:App) RETURN a.id";\n',
  }),
  (code) => code === 0,
);

check(
  "an app with a profile but no Rust of its own is skipped",
  // Whether it should HAVE a profile is check-app-profiles' question. Reporting
  // it here would be the same finding twice, in two voices.
  tree({
    [`${PROFILES}/dev.arlen.demo.toml`]: PROFILE(["File"]),
  }),
  (code, out) => code === 0 && out.includes("0 app(s) checked"),
);

console.log(failures.length ? "\nsome cases regressed" : "\nboth directions hold");
process.exit(failures.length ? 1 : 0);
