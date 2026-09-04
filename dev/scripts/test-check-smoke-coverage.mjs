// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A smoke script cannot notice a daemon nobody added to it: the new one is
// simply absent and the run still says OK. The script's own header once carried
// its exclusions in prose, named eight of the twenty-four it actually excluded,
// and asserted in the same breath that skipping silently is worse than not
// testing at all - right about the principle, wrong about itself.
//
// So the classification is data and this compares it to the tree. The fourth
// case is the one with teeth: a crate that builds TWO binaries has to classify
// both, which is how `arlen-timeline` was found unclassified beside
// `arlen-graph-daemon` in the same Cargo.toml.
//
// Run: node dev/scripts/test-check-smoke-coverage.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-smoke-coverage.py");

const failures = [];

const smoke = (array, started, skipped) =>
  `#!/usr/bin/env bash\n${array}=(\n${started.map((e) => `  "${e}"`).join("\n")}\n)\n` +
  `SKIPPED=(\n${skipped.map((e) => `  "${e}"`).join("\n")}\n)\n`;

function check(name, { manifest, started, skipped }, expect) {
  const dir = mint("arlen-smokecov-");
  const write = (rel, body) => {
    mkdirSync(join(dir, dirname(rel)), { recursive: true });
    writeFileSync(join(dir, rel), body);
  };
  write("daemons/probe/Cargo.toml", manifest);
  write("daemons/probe/src/main.rs", "fn main() {}\n");
  write("dev/scripts/smoke-daemons.sh", smoke("DAEMONS", started, skipped));
  // The apps half has to be populated even when the case is about daemons. The
  // check runs `binaries()` per smoke and refuses a smoke whose roots hold none -
  // correctly, since that means the layout moved - so an apps-less fixture exits
  // "found no binaries at all" before any daemon case is reached. Learned by
  // writing this file without one and watching all five cases fail identically.
  write("apps/probe-ui/src-tauri/Cargo.toml", `[package]\nname = "probe-ui"\nversion = "0.1.0"\n`);
  write("apps/probe-ui/src-tauri/src/main.rs", "fn main() {}\n");
  write("dev/scripts/smoke-apps.sh", smoke("APPS", ["probe-ui|a window"], []));

  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

const ONE_BIN = `[package]\nname = "probe"\nversion = "0.1.0"\n`;
const TWO_BINS =
  `[package]\nname = "probe"\nversion = "0.1.0"\n\n` +
  `[[bin]]\nname = "arlen-probe"\npath = "src/main.rs"\n\n` +
  `[[bin]]\nname = "arlen-probe-helper"\npath = "src/helper.rs"\n`;

console.log("check-smoke-coverage:");

check(
  "a daemon the smoke starts is covered",
  { manifest: ONE_BIN, started: ["probe|/run/probe.sock"], skipped: [] },
  (code) => code === 0,
);

check(
  "a daemon skipped with a reason is covered",
  { manifest: ONE_BIN, started: [], skipped: ["probe|needs a display"] },
  (code) => code === 0,
);

check(
  "a daemon in neither list is named",
  { manifest: ONE_BIN, started: [], skipped: [] },
  (code, out) => code === 1 && out.includes("probe"),
);

// The `arlen-timeline` case. Taking only the first `[[bin]]` meant the second
// binary was never demanded to be classified - the check failing in the
// direction that says nothing.
check(
  "the second binary of a crate has to be classified too",
  {
    manifest: TWO_BINS,
    started: ["arlen-probe|/run/probe.sock"],
    skipped: [],
  },
  (code, out) => code === 1 && out.includes("arlen-probe-helper"),
);

check(
  "and classifying both passes",
  {
    manifest: TWO_BINS,
    started: ["arlen-probe|/run/probe.sock"],
    skipped: ["arlen-probe-helper|a FUSE helper, needs a mount"],
  },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all smoke-coverage cases passed");
