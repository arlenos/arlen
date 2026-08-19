#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-lockfiles-current.py: put the fault back and watch it fail.
//
// The fault is a real one, staged the way it actually happened - a manifest
// declaring a dependency its lockfile never recorded - rather than a corrupted
// file, which would fail for the wrong reason and prove nothing.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-lockfiles-current.py");
let failures = 0;
const ok = (name) => console.log(`  ok   ${name}`);
const bad = (name, detail) => {
  console.log(`  FAIL ${name}`);
  console.log(`       ${detail}`);
  failures += 1;
};

/// A tiny crate with no dependencies, so the check never needs the network.
function crate(root, name, { extraDep = false } = {}) {
  const dir = join(root, name);
  mkdirSync(join(dir, "src"), { recursive: true });
  writeFileSync(join(dir, "src", "lib.rs"), "pub fn f() {}\n");
  const dep = extraDep ? 'other = { path = "../other" }\n' : "";
  writeFileSync(
    join(dir, "Cargo.toml"),
    `[package]\nname = "${name}"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n${dep}\n[workspace]\n`,
  );
  return dir;
}

/// A git tree, because the check reads `git ls-files` rather than globbing -
/// an uncommitted lockfile is not one the tree is making a promise about.
function stage({ withDrift = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "lockfiles-"));
  crate(root, "other");
  const main = crate(root, "main");
  execFileSync("git", ["init", "-q"], { cwd: root });
  // Resolve once so the lock is honest, then optionally make it lie.
  execFileSync("cargo", ["metadata", "--manifest-path", join(main, "Cargo.toml"), "--format-version", "1"], {
    cwd: root,
    stdio: "ignore",
  });
  if (withDrift) {
    // The 19 August shape exactly: the manifest gains a dependency and the
    // lockfile beside it is left describing the world before that.
    const toml = readFileSync(join(main, "Cargo.toml"), "utf8");
    writeFileSync(join(main, "Cargo.toml"), toml.replace("[dependencies]\n", '[dependencies]\nother = { path = "../other" }\n'));
  }
  execFileSync("git", ["add", "-A"], { cwd: root });
  execFileSync("git", ["-c", "user.email=t@e", "-c", "user.name=t", "commit", "-qm", "x"], { cwd: root });
  return root;
}

function run(root) {
  try {
    const out = execFileSync("python3", [check, root], { encoding: "utf8" });
    return { code: 0, out };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

{
  const root = stage();
  const r = run(root);
  r.code === 0
    ? ok("a lockfile that matches its manifest passes")
    : bad("a lockfile that matches its manifest passes", `expected 0, got ${r.code}: ${r.out}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The case this exists for.
  const root = stage({ withDrift: true });
  const r = run(root);
  r.code === 1 && /no longer describes/.test(r.out)
    ? ok("a manifest whose lockfile never recorded its dependency is caught")
    : bad(
        "a manifest whose lockfile never recorded its dependency is caught",
        `expected 1, got ${r.code}: ${r.out}`,
      );
  rmSync(root, { recursive: true, force: true });
}

{
  // Reading nothing must not read as a pass.
  const root = mkdtempSync(join(tmpdir(), "lockfiles-empty-"));
  execFileSync("git", ["init", "-q"], { cwd: root });
  const r = run(root);
  r.code === 2
    ? ok("finding no lockfile at all is not a pass")
    : bad("finding no lockfile at all is not a pass", `expected 2, got ${r.code}: ${r.out}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const r = run(join(here, "..", ".."));
  r.code === 0
    ? ok("the repository itself passes")
    : bad("the repository itself passes", `got ${r.code}: ${r.out}`);
}

console.log(
  failures === 0
    ? "a committed lockfile has to describe the manifest beside it"
    : `\n${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
