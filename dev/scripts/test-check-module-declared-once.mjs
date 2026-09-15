// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-module-declared-once.
//
// The planted defect is `daemons/knowledge` as it stood until 15 September: a
// module tree declared by `lib.rs` and again by `main.rs`, so the crate compiled
// and tested itself twice and the two copies' types did not match.
//
// The near-misses are what make the rule usable. A binary may have modules of its
// own, a crate with only one root is not this shape at all, and a `mod` nested
// inside a function or a test block is not a crate-root declaration.
//
// Run: node dev/scripts/test-check-module-declared-once.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-module-declared-once.py");
const failures = [];

function tree(crates) {
  const dir = mint("arlen-modonce-");
  for (const [path, files] of Object.entries(crates)) {
    mkdirSync(join(dir, path, "src"), { recursive: true });
    for (const [name, body] of Object.entries(files)) {
      writeFileSync(join(dir, path, "src", name), body);
    }
  }
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

console.log("check-module-declared-once:");

let d = tree({
  "daemons/thing": {
    "lib.rs": "pub mod graph;\npub mod db;\n",
    "main.rs": "use thing::graph;\nfn main() {}\n",
    "graph.rs": "",
    "db.rs": "",
  },
});
let r = run(d);
check("a thin main over its library passes", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = tree({
  "daemons/thing": {
    "lib.rs": "pub mod graph;\npub mod db;\n",
    "main.rs": "mod graph;\nmod db;\nmod only_here;\nfn main() {}\n",
    "graph.rs": "",
    "db.rs": "",
    "only_here.rs": "",
  },
});
r = run(d);
check(
  "a module declared by both roots is a finding",
  r.code === 1 && /2 module\(s\)/.test(r.out) && /db, graph/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

// The binary is allowed modules the library does not have: those compile once.
d = tree({
  "daemons/thing": {
    "lib.rs": "pub mod graph;\n",
    "main.rs": "mod cli;\nfn main() {}\n",
    "graph.rs": "",
    "cli.rs": "",
  },
});
r = run(d);
check("a module only the binary has is left alone", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

// A `mod` block written inline is not a crate-root declaration of a file.
d = tree({
  "daemons/thing": {
    "lib.rs": "pub mod graph;\n",
    "main.rs": "fn main() {}\n\n#[cfg(test)]\nmod graph {\n    // an inline block, not a file\n}\n",
    "graph.rs": "",
  },
});
r = run(d);
check("an inline mod block is not a second declaration", r.code === 0, `exit=${r.code} out=${r.out}`);
cleanup(d);

d = tree({ "daemons/thing": { "lib.rs": "pub mod graph;\n", "graph.rs": "" } });
r = run(d);
check(
  "a tree with no lib+bin crate refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
cleanup(d);

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
if (failures.length) process.exit(1);
console.log("a source file belongs to one crate, and a second declaration is caught");
