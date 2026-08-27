// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The sensing switch is read in three places - Settings, the xdg portal and the
// compositor - and the table of vectors they are held to exists twice, once per
// repository. Each repo's own tests answer its own copy, so a copy that drifts
// still passes on both sides while the two now describe different rules. A master
// switch enforced on two paths out of three is not a master switch.
//
// The fourth case is the one that keeps the check honest rather than convenient:
// with no compositor checked out this cannot compare anything, and it has to SAY
// so rather than exit 0 quietly. A skip that reads like a pass is how a check
// stops meaning anything without anyone noticing.
//
// Run: node dev/scripts/test-check-sensing-vectors.mjs

import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-sensing-vectors.sh");

const failures = [];

// The script finds its own table at `<script>/../fixtures/sensing-vectors`, so
// the fixture is a copy of the script with a table beside it. Nothing about the
// comparison changes; only where the two tables live.
function check(name, { ours, theirs }, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-sensing-"));
  const comp = join(dir, "compositor");

  mkdirSync(join(dir, "scripts"), { recursive: true });
  const script = join(dir, "scripts", "check-sensing-vectors.sh");
  copyFileSync(GATE, script);
  chmodSync(script, 0o755);

  const writeTable = (base, files) => {
    if (files === null) return;
    mkdirSync(base, { recursive: true });
    for (const [f, body] of Object.entries(files)) writeFileSync(join(base, f), body);
  };
  writeTable(join(dir, "fixtures", "sensing-vectors"), ours);
  writeTable(join(comp, "dev", "fixtures", "sensing-vectors"), theirs);

  const r = spawnSync("bash", [script], {
    encoding: "utf8",
    env: { ...process.env, COMPOSITOR_PATH: comp },
  });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const TABLE = { "off__explicit.toml": 'sensing = "off"\n' };

console.log("check-sensing-vectors:");

check(
  "two copies that agree pass",
  { ours: TABLE, theirs: TABLE },
  (code, out) => code === 0 && out.includes("agree across both repositories"),
);

check(
  "a case whose content differs is caught",
  { ours: TABLE, theirs: { "off__explicit.toml": 'sensing = "on"\n' } },
  (code, out) => code === 1 && out.includes("diverged"),
);

// The drift that costs the most and is easiest to make: one repo gains a case and
// the other does not, so a rule exists on one path and not the others.
check(
  "a case one repo has and the other does not is caught",
  { ours: { ...TABLE, "on__explicit.toml": 'sensing = "on"\n' }, theirs: TABLE },
  // The missing case by name: an exit-code-only expectation would also pass if
  // the gate fell over before it compared anything.
  (code, out) => code === 1 && out.includes("on__explicit"),
);

// Deliberate: the README is prose and lives in one repo, so it is excluded. If it
// were compared, keeping the two in sync would mean keeping two copies of an
// explanation identical, which is a cost with no safety in it.
check(
  "the README is prose and is not held to the comparison",
  {
    ours: { ...TABLE, "README.md": "ours\n" },
    theirs: { ...TABLE, "README.md": "theirs, and longer\n" },
  },
  (code) => code === 0,
);

check(
  "no compositor checked out says so rather than passing quietly",
  { ours: TABLE, theirs: null },
  (code, out) => code === 0 && out.includes("unchecked"),
);

check(
  "a missing local table is a failure, not an empty comparison",
  { ours: null, theirs: TABLE },
  (code, out) => code === 1 && out.includes("no vector table"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all sensing-vector cases passed");
