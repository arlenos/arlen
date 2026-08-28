#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-cypher-groups.py. The repository passes today, so the questions worth asking
// are whether the check can fail at all, and whether each exclusion holds without swallowing the
// code around it. Every exclusion here is load-bearing and was added because its absence broke
// something: keying on the SDK client keeps out the daemon's own refusal message and regression
// test (which quote the refused shape on purpose) and SQLite's perfectly legal
// `WHERE (?1 IS NULL OR ...)`; skipping `#[cfg(test)]` keeps out parser tests that feed
// themselves unlabelled patterns deliberately. The last case is the important one - an exclusion
// that never ends leaves the rest of the file unchecked, which reads exactly like a clean board.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

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
  const root = mint("cypher-groups-");
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
  cleanup(root);
}

{
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": CLEAN });
  const rc = run(root);
  rc === 0
    ? ok("an ungrouped predicate passes")
    : bad("an ungrouped predicate passes", `expected 0, got ${rc}`);
  cleanup(root);
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
  cleanup(root);
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
  cleanup(root);
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
  cleanup(root);
}

{
  // The second way to hand the gate an unlabelled node, and the one that cost the
  // harness its whole scope picker.
  const bare = CLEAN.replace(
    "MATCH (f:File)-[:FILE_PART_OF]->(p:Project) WHERE p.name = 'x'",
    "MATCH (p:Project) WHERE p.name = 'x' OPTIONAL MATCH (f:File)-[:FILE_PART_OF]->(p)",
  );
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": bare });
  const rc = run(root);
  rc === 1
    ? ok("a bare back-reference is caught")
    : bad("a bare back-reference is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const labelled = CLEAN.replace(
    "MATCH (f:File)-[:FILE_PART_OF]->(p:Project) WHERE p.name = 'x'",
    "MATCH (p:Project) WHERE p.name = 'x' OPTIONAL MATCH (f:File)-[:FILE_PART_OF]->(p:Project)",
  );
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": labelled });
  const rc = run(root);
  rc === 0
    ? ok("repeating the label passes")
    : bad("repeating the label passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A parser test feeds itself deliberately unlabelled patterns - modulesd asserts
  // on `MATCH (anonymous) RETURN anonymous`. Flagging those is what made the first
  // attempt at this rule unusable, so the exclusion is load-bearing, not a nicety.
  const withTest =
    CLEAN +
    `
#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        assert_eq!(namespace("MATCH (p:shared.Person)-[:WORKS_AT]->(o)"), Some("x"));
        assert_eq!(namespace("MATCH (anonymous) RETURN anonymous"), None);
    }
}
`;
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": withTest });
  const rc = run(root);
  rc === 0
    ? ok("a parser test's unlabelled pattern is not a query")
    : bad("a parser test's unlabelled pattern is not a query", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // ...but the exclusion must END with the module, or everything after a test
  // module goes unchecked - the failure mode that reads exactly like a clean board.
  const afterTest =
    CLEAN +
    `
#[cfg(test)]
mod tests {
    #[test]
    fn parses() {}
}

fn later(c: &UnixGraphClient) {
    c.query_rows("MATCH (p:Project) OPTIONAL MATCH (f:File)-[:FILE_PART_OF]->(p) RETURN p.name");
}
`;
  const root = tree({ "apps/demo/src-tauri/src/kg.rs": afterTest });
  const rc = run(root);
  rc === 1
    ? ok("checking resumes after the test module closes")
    : bad("checking resumes after the test module closes", `expected 1, got ${rc}`);
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
console.log("an unlabelled node is caught before it reaches the gate");
