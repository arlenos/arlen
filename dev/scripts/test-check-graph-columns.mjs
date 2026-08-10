// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A Cypher query names columns a schema in another crate declares, and nothing
// joins the two until a socket round trip at runtime. This check is that join, so
// it has to read the queries that actually ship.
//
// The third case is why this file exists. The gate cut each file at the first
// `#[cfg(test)]` and dropped the rest, and `daemons/knowledge/src/write/entity.rs`
// puts one at line 589 of 1431 - so 14 production write-path queries after it were
// never checked. Measured on 11 August; they all pass, so it was coverage rather
// than a live break, and the predecessor's own comment had predicted exactly this
// file shape.
//
// The fourth case is the reason the exclusion exists at all and must keep working:
// a test's Cypher names whatever the test needs, including columns no table has,
// and checking those would report passing tests as broken queries.
//
// Run: node dev/scripts/test-check-graph-columns.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-graph-columns.py");

const failures = [];

const SCHEMA = `pub fn create_schema() {
    conn.query("CREATE NODE TABLE IF NOT EXISTS File (id STRING, path STRING, PRIMARY KEY (id))");
}
`;

const TESTS = (query) => `#[cfg(test)]
mod tests {
    #[test]
    fn the_builder_renders_what_it_was_given() {
        assert_eq!(build(), "${query}");
    }
}
`;

function check(name, source, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-graphcol-"));
  const src = join(dir, "daemons/knowledge/src");
  mkdirSync(src, { recursive: true });
  writeFileSync(join(src, "graph.rs"), SCHEMA);
  writeFileSync(join(src, "queries.rs"), source);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const GOOD = 'const Q: &str = "MATCH (f:File) RETURN f.path";\n';
const BAD = 'const Q2: &str = "MATCH (f:File) RETURN f.no_such_column";\n';

console.log("check-graph-columns:");

check("a query naming a declared column passes", GOOD, (code) => code === 0);

check(
  "a query naming a column no table has is caught",
  `${GOOD}${BAD}`,
  (code, out) => code === 1 && out.includes("no_such_column"),
);

// The case the cut-at-the-first-marker version dropped.
check(
  "a bad query BELOW the test module is caught",
  `${GOOD}${TESTS("MATCH (f:File) RETURN f.whatever_the_test_wants")}${BAD}`,
  (code, out) => code === 1 && out.includes("no_such_column"),
);

check(
  "a test's own Cypher is still not held to the schema",
  `${GOOD}${TESTS("MATCH (f:File) RETURN f.whatever_the_test_wants")}`,
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all graph-columns cases passed");
