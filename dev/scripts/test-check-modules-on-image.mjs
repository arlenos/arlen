// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the modules-on-image check: watch it fail on each shape it
// claims to catch, and pass on the shape it must not.
//
// It runs against fixture trees rather than the repo, so it keeps working when a
// module is added or excused. The repo is only asked one thing, at the end: that
// the check has something to read at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-modules-on-image.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** A tree with one module and whatever staging step the caller wants. */
function tree({ id = "core.example", step = null } = {}) {
  const root = mint("modules-on-image-");
  mkdirSync(join(root, "modules/example"), { recursive: true });
  writeFileSync(join(root, "modules/example/Cargo.toml"), "[package]\nname = \"x\"\n");
  writeFileSync(join(root, "modules/example/manifest.toml"), `[module]\nid = "${id}"\n`);
  mkdirSync(join(root, "dev/mkosi/mkosi.build.d"), { recursive: true });
  if (step !== null) {
    writeFileSync(join(root, "dev/mkosi/mkosi.build.d/08t2-modules.sh.chroot"), step);
  }
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const STAGED = `cargo build --manifest-path modules/example/Cargo.toml
install -Dm644 x "$DESTDIR/usr/share/arlen/modules/core.example/module.wasm"
`;

{
  const root = tree({ step: STAGED });
  const r = run(root);
  check("a staged module passes", r.code === 0);
  cleanup(root);
}

{
  const root = tree({ step: null });
  const r = run(root);
  check("a module no step names fails", r.code === 1 && r.out.includes("modules/example"));
  cleanup(root);
}

// The half the app check had to learn: a step that builds a thing vouches for it
// unless the install is required too.
{
  const root = tree({ step: "cargo build --manifest-path modules/example/Cargo.toml\n" });
  const r = run(root);
  check("building without installing is not staging", r.code === 1);
  cleanup(root);
}

// The id is read from the manifest, so a staging line that installs under some
// other name cannot agree with it by being copied from it.
{
  const root = tree({ id: "core.renamed", step: STAGED });
  const r = run(root);
  check(
    "a manifest id no step stages fails",
    r.code === 1 && r.out.includes("core.renamed"),
  );
  cleanup(root);
}

// A scan pointed at a tree with no modules would otherwise print a pass.
{
  const root = mint("modules-on-image-empty-");
  const r = run(root);
  check("an empty scan is a failure, not a pass", r.code === 1);
  cleanup(root);
}

{
  const r = run(ROOT);
  check("and the repo itself passes", r.code === 0);
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe check fails when it should");
process.exit(failures ? 1 : 0);
