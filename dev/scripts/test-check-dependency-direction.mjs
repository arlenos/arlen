// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-dependency-direction.
//
// The planted defect is the edit the gate exists to catch: deduplicating the
// knowledge daemon's hand-written typed-value encoder against the AI layer, which
// removes visible duplication and adds an invisible dependency.
//
// The long-form control matters because `[dependencies.arlen-ai-core]` names the
// dependency in a table header rather than on a line, and a line-only scan reads
// that as no dependency at all - the same "absence read as nothing" shape the
// gates elsewhere are about.
//
// Run: node dev/scripts/test-check-dependency-direction.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dependency-direction.py");
const failures = [];

function tree(knowledgeDeps) {
  const dir = mint("arlen-depdir-");
  mkdirSync(join(dir, "daemons/knowledge"), { recursive: true });
  writeFileSync(
    join(dir, "daemons/knowledge/Cargo.toml"),
    `[package]\nname = "x"\n\n[dependencies]\nserde = "1"\n${knowledgeDeps}\n`,
  );
  return dir;
}

const run = (dir) => {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-dependency-direction:");

let d = tree("");
let r = run(d);
check("the tree as it stands passes", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = tree('arlen-ai-core = { path = "../../ai/ai-core" }');
r = run(d);
check(
  "deduplicating against the AI layer is caught",
  r.code === 1 && /arlen-ai-core/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

d = tree('[dependencies.arlen-ai-core]\npath = "../../ai/ai-core"');
r = run(d);
check(
  "the long table form is caught too",
  r.code === 1 && /arlen-ai-core/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

d = tree('[dev-dependencies]\narlen-ai-engine = "0.1"');
r = run(d);
check(
  "a dev-dependency counts, since it still makes the crate need the layer to test",
  r.code === 1 && /arlen-ai-engine/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

// A near-miss: a crate whose name merely starts similarly must not trip it.
d = tree('arlen-audit-proto = "0.1"');
r = run(d);
check("an unrelated arlen crate is not an AI dependency", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

// Reading nothing is not passing.
d = mint("arlen-depdir-empty-");
r = run(d);
check(
  "a tree with no manifests refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("the dedupe edit is caught in both manifest forms, and a look-alike name is not");
