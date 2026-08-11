// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate reports a field a query READS that the profile does not grant, and
// deliberately says nothing about the reverse. These cases pin both halves of
// that, and the two shapes that made the hand version of this check wrong:
// a query wrapped over several lines, and Rust that merely looks like Cypher.
//
// Run: node dev/scripts/test-check-read-grants-cover-queries.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-read-grants-cover-queries.py");

const failures = [];

const P = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";
const APP = "apps/probe/src-tauri/src";

const profile = (...grants) => `[info]
app_id = "dev.arlen.probe"
tier = "first-party"

[graph]
read = [
${grants.map((g) => `    "${g}",`).join("\n")}
]
`;

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-grants-"));
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
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-read-grants-cover-queries:");

check(
  "a query inside its grants passes",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.File.id", "system.File.path"),
    [`${APP}/lib.rs`]: 'let q = "MATCH (f:File) RETURN f.id AS id, f.path AS path";\n',
  },
  (code) => code === 0,
);

check(
  "a returned field that is not granted is caught",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.File.id"),
    [`${APP}/lib.rs`]: 'let q = "MATCH (f:File) RETURN f.id AS id, f.path AS path";\n',
  },
  (code, out) => code === 1 && out.includes("system.File.path"),
);

// The kind that is easiest to miss and the reason the knowledge profile's header
// had to change: it never comes back in a row, so nobody checking columns sees it.
check(
  "a filter-only field that is not granted is caught",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.Project.id"),
    [`${APP}/lib.rs`]: 'let q = "MATCH (p:Project) WHERE p.expired_at IS NULL RETURN p.id AS id";\n',
  },
  (code, out) => code === 1 && out.includes("system.Project.expired_at"),
);

// The shape that defeated the by-hand pass: Rust wraps a long query with `\`
// continuations, and a line-oriented scan stops at the first newline and calls
// the profile covered.
check(
  "a query wrapped over several lines is still read",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.File.id"),
    [`${APP}/lib.rs`]: 'let q = "MATCH (f:File) \\\n    RETURN f.id AS id, f.last_accessed AS at";\n',
  },
  (code, out) => code === 1 && out.includes("system.File.last_accessed"),
);

// The mirror, and the more dangerous one: Rust and Cypher share the `x.field`
// shape. A window that runs past the query turns a sort comparator into an
// apparent read, and the fix that invites is granting a field nothing reads.
check(
  "Rust that looks like Cypher outside a query is not read",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.App.id"),
    [`${APP}/lib.rs`]: `let q = "MATCH (a:App) RETURN a.id AS id";
fn sort(a: &Row, b: &Row) -> Ordering { a.name.cmp(&b.name) }
`,
  },
  (code) => code === 0,
);

// One direction only: a grant nothing appears to use is never reported, because
// the fix that suggests is deleting a grant an app needs.
check(
  "a grant no query uses is not reported",
  {
    [`${P}/dev.arlen.probe.toml`]: profile("system.File.id", "system.File.mode"),
    [`${APP}/lib.rs`]: 'let q = "MATCH (f:File) RETURN f.id AS id";\n',
  },
  (code) => code === 0,
);

// An app with no grants at all reads through an allowlisted op instead (meetings
// does), and this check cannot tell that from an omission - so it says nothing.
check(
  "a profile with no grants is left alone",
  {
    [`${P}/dev.arlen.probe.toml`]: '[info]\napp_id = "dev.arlen.probe"\ntier = "first-party"\n',
    [`${APP}/lib.rs`]: 'let q = "MATCH (f:File) RETURN f.path AS path";\n',
  },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all read-grant cases passed");
