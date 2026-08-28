// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Four things describe what gets built - the CI matrix, the justfile lists, the
// crate roots on disk, and the package.json files with a test script - and
// nothing made them agree. The justfile fell NINE crates behind CI, so
// `just test` reported a green CI did not share; and a crate added without a
// matrix edit is simply never built by anyone.
//
// The cases below plant each of those drifts separately, because the check has
// to name which list is behind. A single "they differ" would leave whoever reads
// it diffing two long arrays by eye, which is the state this replaced.
//
// Run: node dev/scripts/test-check-crate-coverage.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-crate-coverage.py");

const failures = [];

/// `rust` and `front` are the four lists; `crates` are the roots on disk.
function check(name, { rustCi, rustJust, frontCi, frontJust, crates, fronts }, expect) {
  const dir = mint("arlen-cratecov-");
  const write = (rel, body) => {
    mkdirSync(join(dir, dirname(rel)), { recursive: true });
    writeFileSync(join(dir, rel), body);
  };

  write(
    ".github/workflows/ci.yml",
    `jobs:\n  matrix:\n    steps:\n      - run: |\n` +
      `          RUST_ALL='${JSON.stringify(rustCi)}'\n` +
      `          FRONT_ALL='${JSON.stringify(frontCi)}'\n`,
  );
  write(
    "dev/justfile",
    `RUST_CRATES := "${rustJust.join(" ")}"\nFRONTENDS := "${frontJust.join(" ")}"\n`,
  );
  for (const c of crates) {
    write(`${c}/Cargo.toml`, `[package]\nname = "probe"\nversion = "0.1.0"\n`);
  }
  for (const f of fronts ?? []) {
    write(`${f}/package.json`, JSON.stringify({ scripts: { test: "vitest run" } }));
  }

  // `git ls-files` is how the check finds crate roots, so the fixture is a repo
  // or nothing is found and every case fails for a reason unrelated to its
  // subject. The same trap as `test-check-proto-drift`.
  const git = (...a) => spawnSync("git", ["-C", dir, ...a], { encoding: "utf8" });
  git("init", "-q");
  git("config", "user.email", "t@example.invalid");
  git("config", "user.name", "t");
  git("add", "-A");

  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-crate-coverage:");

// Every fixture carries a frontend as well as a crate, because the check refuses
// an EMPTY list per list rather than only all-four-empty - a real tree has both,
// so an empty one means the pattern stopped matching. Learned by writing the
// first version of this file with no frontends and watching every case exit 2.
const agreed = {
  rustCi: ["contracts/probe"],
  rustJust: ["contracts/probe"],
  frontCi: ["apps/probe-ui"],
  frontJust: ["apps/probe-ui"],
  crates: ["contracts/probe"],
  fronts: ["apps/probe-ui"],
};

check("four lists that agree with the tree pass", agreed, (code) => code === 0);

check(
  "a crate on disk that CI never builds is named",
  // `contracts/other` is on disk too, so the lists stay non-empty and the crate
  // being reported is genuinely the uncovered one rather than the only one.
  {
    ...agreed,
    crates: ["contracts/probe", "contracts/other"],
    rustCi: ["contracts/other"],
    rustJust: ["contracts/other"],
  },
  (code, out) => code === 1 && out.includes("contracts/probe"),
);

// The drift that actually happened: CI grew and the justfile did not, so the
// local command reported a green the pipeline did not share.
check(
  "a crate CI builds and the justfile does not is named",
  {
    ...agreed,
    crates: ["contracts/probe", "contracts/other"],
    rustCi: ["contracts/probe", "contracts/other"],
    rustJust: ["contracts/other"],
  },
  (code, out) => code === 1 && out.includes("contracts/probe"),
);

check(
  "an excluded crate root is not asked to be in the matrix",
  { ...agreed, crates: ["contracts/probe", "apps/probe/src-tauri"] },
  (code) => code === 0,
);

// The case the check already guards, pinned so it stays guarded: four empty
// lists agree with each other perfectly and with an empty tree, so the day a
// regex stops matching - a renamed variable, a reformatted array - this would
// otherwise report full coverage of nothing.
check(
  "four empty lists are a broken scan, not full coverage",
  { rustCi: [], rustJust: [], frontCi: [], frontJust: [], crates: [] },
  (code) => code !== 0,
);

// Found by this control: a matrix entry whose crate is not on disk crashed with a
// FileNotFoundError instead of naming it. That is the most ordinary way these
// lists rot - rename a crate, leave the line behind.
check(
  "a matrix entry with no crate behind it is named, not a traceback",
  { ...agreed, rustCi: ["contracts/probe", "contracts/ghost"], rustJust: ["contracts/probe", "contracts/ghost"] },
  (code, out) => code === 1 && out.includes("contracts/ghost") && !out.includes("Traceback"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all crate-coverage cases passed");
