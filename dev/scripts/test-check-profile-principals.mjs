// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-profile-principals.
//
// The defect it guards is one that happened: `arlen-compositor.toml` sat on the
// image for weeks addressed to a principal the resolver never produces, so the
// subscribe list inside it was inert and nothing said so. The cases below are the
// three ways that recurs, plus the refusals.
//
// Run: node dev/scripts/test-check-profile-principals.mjs

import { mkdirSync, writeFileSync, cpSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = "dev/scripts/check-profile-principals.py";
const SIBLING = "dev/scripts/check-admitted-ids-exist.py";
const RESOLVER = "sdk/permissions/src/identity.rs";
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";
const PHASE = "dev/mkosi/mkosi.build.d/04-demo.sh.chroot";
const UNITGATE = "dev/scripts/check-shipped-units.py";
const failures = [];

/// A tree the gate can read: both scripts, a resolver, a build phase, profiles.
function tree({ profiles = [], phase = "", withResolver = true } = {}) {
  const dir = mint("arlen-principals-");
  for (const rel of [GATE, SIBLING, UNITGATE]) {
    mkdirSync(join(dir, dirname(rel)), { recursive: true });
    cpSync(join(ROOT, rel), join(dir, rel));
  }
  if (withResolver) {
    mkdirSync(join(dir, dirname(RESOLVER)), { recursive: true });
    cpSync(join(ROOT, RESOLVER), join(dir, RESOLVER));
  }
  mkdirSync(join(dir, dirname(PHASE)), { recursive: true });
  writeFileSync(join(dir, PHASE), phase);
  mkdirSync(join(dir, PROFILES), { recursive: true });
  for (const name of profiles) {
    writeFileSync(join(dir, PROFILES, `${name}.toml`), "[info]\n");
  }
  return dir;
}

function run(name, opts, expect) {
  const dir = tree(opts);
  const r = spawnSync("python3", [join(dir, GATE)], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  cleanup(dir);
}

const DAEMON = 'install -Dm755 "$CARGO_TARGET_DIR/release/x" "$DESTDIR/usr/bin/arlen-widget"\n';
const APP = 'install -Dm755 "$out" "$DESTDIR/usr/lib/arlen/apps/dev.arlen.thing/bin/arlen-thing"\n';

console.log("profile principals:");

run("the tree as it stands passes", {}, () => {
  const r = spawnSync("python3", [join(ROOT, GATE)], { encoding: "utf8" });
  const ok = r.status === 0;
  if (!ok) console.log(`${r.stdout ?? ""}${r.stderr ?? ""}`);
  return ok;
});

run(
  "a profile named after the binary rather than the id is caught",
  { profiles: ["arlen-widget"], phase: DAEMON },
  (code, out) => code === 1 && out.includes("arlen-widget.toml"),
);

run(
  "a profile named after the app directory's binary is caught too",
  { profiles: ["arlen-thing"], phase: APP },
  (code, out) => code === 1 && out.includes("arlen-thing.toml"),
);

run(
  "the ids the resolver does produce pass",
  { profiles: ["widget", "dev.arlen.thing"], phase: DAEMON + APP },
  (code) => code === 0,
);

run(
  "a tree with no profiles refuses rather than passing",
  { phase: DAEMON },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a tree that resolves no id at all refuses rather than reporting every profile",
  { profiles: ["widget"], phase: "" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a profile addressed to a principal the image cannot produce is caught");
