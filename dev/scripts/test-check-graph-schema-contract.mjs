// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does check-graph-schema-contract.py notice the AI's schema falling behind the DDL?
//
// Each case is a two-file tree: a `graph.rs` with the DDL and a `lib.rs` contract.
// The gate is run from here, against the fixture root.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const gate = join(here, "check-graph-schema-contract.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

// The tables the gate keeps from the AI by name. A fixture DDL carries them,
// because the gate also refuses an exclusion whose table is gone.
const EXCLUDED_DDL = (skip = "") =>
  ["Grant", "CapabilityUse", "EntityType", "MergeSuggestion"]
    .filter((t) => t !== skip)
    .map((t) => `    conn.query("CREATE NODE TABLE IF NOT EXISTS ${t}(id STRING, PRIMARY KEY(id))");\n`)
    .join("") +
  ["GRANTS", "USED_BY", "LAST_EXERCISED"]
    .filter((t) => t !== skip)
    .map((t) => `    conn.query("CREATE REL TABLE IF NOT EXISTS ${t}(FROM Grant TO App)");\n`)
    .join("");

const DDL = (extra = "", skip = "") => `
fn create_schema(conn: &Connection) {
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS File(
            id STRING,
            path STRING,
            PRIMARY KEY(id)
        )",
    );
    conn.query(
        "CREATE NODE TABLE IF NOT EXISTS App(
            id STRING,
            pid INT64,
            PRIMARY KEY(id)
        )",
    );
    conn.query("CREATE REL TABLE IF NOT EXISTS ACCESSED_BY(FROM File TO App)");
${EXCLUDED_DDL(skip)}${extra}
}
`;

const node = (label, fields) =>
  `    NodeSchema {\n        label: "${label}",\n        fields: &[\n${fields
    .map(([n, t]) => `            ("${n}", FieldType::${t}),\n`)
    .join("")}        ],\n    },\n`;
const edge = (label, from, to) =>
  `    EdgeSchema {\n        label: "${label}",\n        from: "${from}",\n        to: "${to}",\n    },\n`;
const CONTRACT = (nodes, edges) =>
  `const NODES: &[NodeSchema] = &[\n${nodes.join("")}];\nconst EDGES: &[EdgeSchema] = &[\n${edges.join("")}];\n`;

const FILE = node("File", [["id", "Text"], ["path", "Text"]]);
const APP = node("App", [["id", "Text"], ["pid", "Int"]]);
const ACC = edge("ACCESSED_BY", "File", "App");

function run(ddl, contract) {
  const root = mint("graph-schema-contract-");
  try {
    mkdirSync(join(root, "daemons/knowledge/src"), { recursive: true });
    mkdirSync(join(root, "contracts/graph-schema/src"), { recursive: true });
    if (ddl !== null) writeFileSync(join(root, "daemons/knowledge/src/graph.rs"), ddl);
    if (contract !== null) writeFileSync(join(root, "contracts/graph-schema/src/lib.rs"), contract);
    const r = spawnSync("python3", [gate, root], { encoding: "utf8" });
    return { code: r.status, out: r.stdout + r.stderr };
  } finally {
    cleanup(root);
  }
}

let r = run(DDL(), CONTRACT([FILE, APP], [ACC]));
check("a contract that agrees passes", r.code === 0, r.out);

r = run(DDL(`    conn.query("CREATE REL TABLE IF NOT EXISTS LAUNCHED(FROM App TO App)");`), CONTRACT([FILE, APP], [ACC]));
check("a table missing from the contract is named", r.code === 1 && r.out.includes("LAUNCHED is declared"), r.out);

r = run(DDL(), CONTRACT([FILE, APP], [ACC, edge("GRANTS", "Grant", "App")]));
check("an excluded table that is listed anyway is named", r.code === 1 && r.out.includes("GRANTS is excluded"), r.out);

r = run(DDL(), CONTRACT([node("File", [["id", "Text"]]), APP], [ACC]));
check("a column missing from the contract is named", r.code === 1 && r.out.includes("File.path is a column"), r.out);

r = run(DDL(), CONTRACT([FILE, node("App", [["id", "Text"], ["pid", "Text"]])], [ACC]));
check("a type that disagrees is named", r.code === 1 && r.out.includes("App.pid is Text in the contract and Int"), r.out);

r = run(DDL(), CONTRACT([FILE, APP], [edge("ACCESSED_BY", "App", "File")]));
check("endpoints that disagree are named", r.code === 1 && r.out.includes("ACCESSED_BY runs App -> File"), r.out);

r = run(DDL(), CONTRACT([FILE, APP, node("Ghost", [["id", "Text"]])], [ACC]));
check("a contract entry with no table is named", r.code === 1 && r.out.includes("Ghost is in the contract"), r.out);

r = run(DDL("", "MergeSuggestion"), CONTRACT([FILE, APP], [ACC]));
check("an exclusion whose table is gone is named", r.code === 1 && r.out.includes("MergeSuggestion is excluded but"), r.out);

r = run(null, null);
check("a tree with neither file refuses rather than passing", r.code === 2, r.out);

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log("check-graph-schema-contract: control cases pass.");
