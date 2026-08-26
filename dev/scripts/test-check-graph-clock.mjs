// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-graph-clock.
//
// The defective fixtures are the real ones from 26 August, trimmed: the
// quick-settings tile's day constant and the Waypointer file plugin's week
// constant, both a few lines from a `last_accessed` the graph stores in
// microseconds. The last case is the one that decides whether the rule is worth
// having: `86_400_000_000` is the CORRECT microsecond day and contains the
// millisecond one as a substring, so a rule without a word boundary would fire
// on every file that had just been fixed.
//
// Run: node dev/scripts/test-check-graph-clock.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-graph-clock.py");
const failures = [];

function run(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-clock-"));
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

const READS_GRAPH = `
fn recent(now: i64) -> String {
    format!("MATCH (f:File) WHERE f.last_accessed >= {now} RETURN f.path")
}
`;

console.log("graph clock:");

run(
  "a millisecond day beside a graph time field is caught",
  { "apps/shell/src/tile.rs": `${READS_GRAPH}\nconst DAY: i64 = 86_400_000;\n` },
  (code, out) => code === 1 && out.includes("two clocks"),
);

run(
  "a millisecond week is caught too",
  { "apps/shell/src/files.rs": `${READS_GRAPH}\nconst WEEK: i64 = 7 * 86_400_000;\n` },
  (code) => code === 1,
);

run(
  "the microsecond day is not read as the millisecond one inside it",
  { "apps/shell/src/tile.rs": `${READS_GRAPH}\nconst DAY: i64 = 86_400_000_000;\n` },
  (code) => code === 0,
);

run(
  "a millisecond duration with no graph field is left alone",
  { "apps/shell/src/poll.rs": "const TTL: i64 = 3_600_000;\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a file that reads graph time and keeps to microseconds passes",
  {
    "daemons/knowledge/src/read.rs": `${READS_GRAPH}\nconst DAY_US: i64 = 86_400_000_000;\n`,
  },
  (code) => code === 0,
);

run(
  "the multiplied spelling of a millisecond day is caught",
  {
    "apps/shell/src/plugin.rs": `${READS_GRAPH}\nconst DAY: i64 = 24 * 60 * 60 * 1000;\n`,
  },
  (code) => code === 1,
);

run(
  "a tree with no graph time field at all reports that it read nothing",
  { "apps/shell/src/ui.rs": "fn draw() {}\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a millisecond duration cannot sit beside a microsecond graph read");
