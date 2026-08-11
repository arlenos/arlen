// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// This gate had three rules written down and two of them implemented. The third -
// both callers actually going through `dev/check-crate.sh` - was asserted in the
// success line and checked nowhere, so renaming every reference in `ci.yml` left it
// passing while printing "both gates call it". It had no argument for a fixture
// tree and no test, and that is the same fact from the other side: nobody could
// watch it fail, so nobody noticed it could not.
//
// One case per rule, plus the shape the gate deliberately allows, because the
// narrowness is a decision and not an oversight.
//
// Run: node dev/scripts/test-check-gate-drift.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-gate-drift.py");

const failures = [];

const FLAGS = "-include cstdint";

// The arrangement the gate exists to keep: one owner of the per-crate commands,
// two callers that go through it, one CXXFLAGS value in three places.
const OWNER = `#!/usr/bin/env bash
case "$1" in
    daemons/knowledge) extra=(--test-threads=1) ;;
    apps/desktop-shell/src-tauri) extra=(--test-threads=1) ;;
    *) extra=() ;;
esac
cargo test --manifest-path "$1/Cargo.toml" "\${extra[@]}"
`;

const CI = `jobs:
  rust:
    env:
      CXXFLAGS: "${FLAGS}"
    steps:
      - run: dev/check-crate.sh "\${{ matrix.component }}"
`;

const JUSTFILE = `export CXXFLAGS := "${FLAGS}"

check:
    for c in $CRATES; do dev/check-crate.sh "$c"; done
`;

const CARGO = `[env]\nCXXFLAGS = "${FLAGS}"\n`;

const BASE = {
  "dev/check-crate.sh": OWNER,
  ".github/workflows/ci.yml": CI,
  "dev/justfile": JUSTFILE,
  ".cargo/config.toml": CARGO,
};

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-drift-"));
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

console.log("check-gate-drift:");

check("the intended arrangement passes", BASE, (code) => code === 0);

// Rule 1: the serial rule has one owner.
check(
  "a second copy of the serial rule in a caller is caught",
  { ...BASE, "dev/justfile": `${JUSTFILE}\n    daemons/knowledge) extra=(--test-threads=1) ;;\n` },
  (code, out) => code === 1 && out.includes("its own per-crate serial rule"),
);

// Rule 1's deliberate limit: a one-off run that passes the flag to a NAMED test is
// not the matrix rule, and flagging it would make the gate cry wolf on the nightly
// and on-host recipes. This case is why the regex wants a case arm.
check(
  "a one-off named test run passing the same flag is left alone",
  { ...BASE, "dev/justfile": `${JUSTFILE}\nnightly:\n    cargo test -p arlen-permissions -- --test-threads=1\n` },
  (code) => code === 0,
);

// Rule 3: the loud way to diverge - inline the loop's cargo call.
check(
  "a caller running cargo per crate itself is caught",
  { ...BASE, "dev/justfile": `${JUSTFILE}\n    cargo test --manifest-path "$c/Cargo.toml"\n` },
  (code, out) => code === 1 && out.includes("runs cargo per crate itself"),
);

// Rule 2, the one that was missing: the quiet way. Nothing inlined, the script
// simply not called - and the old success line said "both gates call it" anyway.
check(
  "a caller that never invokes the owner script is caught",
  { ...BASE, ".github/workflows/ci.yml": CI.replace("dev/check-crate.sh", "dev/gone.sh") },
  (code, out) => code === 1 && out.includes("never invokes"),
);

check(
  "the same for the other caller",
  { ...BASE, "dev/justfile": JUSTFILE.replace("dev/check-crate.sh", "dev/gone.sh") },
  (code, out) => code === 1 && out.includes("never invokes"),
);

// Rule 4: CXXFLAGS is duplicated on purpose for three audiences, so it is compared
// as a value - lbug's vendored thrift will not build without it.
check(
  "CXXFLAGS drifting in one of the three places is caught",
  { ...BASE, ".cargo/config.toml": `[env]\nCXXFLAGS = "${FLAGS} -DPROBE"\n` },
  (code, out) => code === 1 && out.includes("CXXFLAGS disagrees"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all gate-drift cases passed");
