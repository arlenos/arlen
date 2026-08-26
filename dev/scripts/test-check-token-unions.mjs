// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-token-unions.
//
// The first case is the real one from 26 August, trimmed: the provenance halo's
// `relation` arrived as English prose and the window compared it against tokens,
// so every graph step took the fallback branch and read as a membership.
//
// The third case is the one that decides whether the check is worth having. The
// first cut scanned the whole crate for `field: "literal"` and reported three
// findings, all false - a `status` on one struct matched against a union on
// another that happened to share the field name. Literals are read only inside
// the struct's own construction, and this case fails if that ever regresses.
//
// Run: node dev/scripts/test-check-token-unions.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-token-unions.py");
const failures = [];

function run(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-tok-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  rmSync(dir, { recursive: true, force: true });
}

const TS = (union) => `
export interface Step {
  actor: string;
  relation?: ${union};
}
`;

const RS = (value) => `
struct Step {
    actor: String,
    relation: Option<String>,
}

fn build() -> Step {
    Step {
        actor: "Atlas".to_string(),
        relation: Some("${value}".to_string()),
    }
}
`;

console.log("token unions:");

run(
  "prose where the union names a token is caught",
  {
    "apps/files/src/lib/x.ts": TS('"partOf" | "lastOpenedBy"'),
    "apps/files/src-tauri/src/lib.rs": RS("Last opened by"),
  },
  (code, out) => code === 1 && out.includes("Last opened by"),
);

run(
  "a literal the union names passes",
  {
    "apps/files/src/lib/x.ts": TS('"partOf" | "lastOpenedBy"'),
    "apps/files/src-tauri/src/lib.rs": RS("lastOpenedBy"),
  },
  (code) => code === 0,
);

run(
  "a same-named field on a DIFFERENT struct is not compared",
  {
    "apps/files/src/lib/x.ts": TS('"partOf" | "lastOpenedBy"'),
    "apps/files/src-tauri/src/lib.rs": `
struct Step {
    relation: Option<String>,
}

struct Other {
    relation: Option<String>,
}

fn build() -> Other {
    Other { relation: Some("something else".to_string()) }
}
`,
  },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "an interface with no same-named struct is left alone",
  {
    "apps/files/src/lib/x.ts": TS('"partOf" | "lastOpenedBy"'),
    "apps/files/src-tauri/src/lib.rs": "struct Unrelated { relation: Option<String> }\n",
  },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a union of type names is not read as a union of tokens",
  {
    "apps/files/src/lib/x.ts": `
export interface Step {
  body?: "Alpha" | "Beta";
}
`,
    "apps/files/src-tauri/src/lib.rs": `
struct Step { body: Option<String> }
fn b() -> Step { Step { body: Some("gamma".to_string()) } }
`,
  },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a value the backend sends is one the window's union names");
