#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-cypher-groups.py. The repository passes today, so the only question worth
// asking is whether the check can fail at all - and whether its two exclusions hold. Both are
// load-bearing: keying on the SDK client is what keeps the daemon's own message and regression
// test (which quote the refused shape on purpose) out of the results, and what keeps SQLite's
// perfectly legal `WHERE (?1 IS NULL OR ...)` out of a rule about Cypher.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-cypher-groups.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// A gated reader: builds a pattern and sends it over the socket.
const CLEAN = `use arlen_os_sdk::UnixGraphClient;

async fn members(c: &UnixGraphClient) {
    c.query_rows("MATCH (f:File)-[:FILE_PART_OF]->(p:Project) WHERE p.name = 'x' RETURN f.path").await;
}
`;

const GROUPED = CLEAN.replace(
  "WHERE p.name = 'x'",
  "WHERE (p.name = 'x' OR p.name = 'y') AND f.path IS NOT NULL",
);

function tree(files) {
  const root = mkdtempSync(join(tmpdir(), "cypher-groups-"));
  for (const [rel, body] of Object.entries(files)) {
    mkdirSync(join(root, dirname(rel)), { recursive: true });
    writeFileSync(join(root, rel), body);
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

{
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": GROUPED });
  const rc = run(root);
  rc === 1
    ? ok("a grouped WHERE in a gated reader is caught")
    : bad("a grouped WHERE in a gated reader is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": CLEAN });
  const rc = run(root);
  rc === 0
    ? ok("an ungrouped predicate passes")
    : bad("an ungrouped predicate passes", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The daemon does not pass its own gate, so its message and its regression test
  // quote the shape deliberately. Paired with a real reader so the run has
  // something in scope - alone it would trip the "not plausible" guard and pass
  // for the wrong reason.
  const root = tree({
    "apps/demo/src-tauri/src/kg.rs": CLEAN,
    "daemons/knowledge/src/daemon.rs": `fn refuse() -> &'static str {
    // MATCH (n) is what the scanner walks
    "a query like MATCH (a:A) WHERE (x OR y) AND z is refused on purpose"
}
`,
  });
  const rc = run(root);
  rc === 0
    ? ok("the daemon's own source is out of scope")
    : bad("the daemon's own source is out of scope", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // SQLite is not Cypher. The audit ledger's real shape, in a file that also
  // happens to mention a graph pattern, must not be flagged.
  const root = tree({
    "apps/demo/src-tauri/src/kg.rs": CLEAN,
    "daemons/audit-daemon/src/ledger/store.rs": `fn q() -> &'static str {
    "SELECT id FROM entries WHERE (?1 IS NULL OR project_id = ?1)"
}
fn unrelated() -> &'static str { "MATCH (x:X) RETURN x" }
`,
  });
  const rc = run(root);
  rc === 0
    ? ok("SQL is not judged by a Cypher rule")
    : bad("SQL is not judged by a Cypher rule", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // A comment describing the problem is not a query - the lens that found this
  // carries exactly such a comment.
  const root = tree({
    "apps/demo/src-tauri/src/kg.rs":
      CLEAN + `\n// A predicate like WHERE (a OR b) reads as an unlabelled node.\n`,
  });
  const rc = run(root);
  rc === 0
    ? ok("a comment about the shape is not a query")
    : bad("a comment about the shape is not a query", `expected 0, got ${rc}`);
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
console.log("a grouped predicate is caught before it reaches the gate");
