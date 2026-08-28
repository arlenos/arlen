#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-linked-libraries.py.
//
// The defect it was written for was invisible for weeks precisely because
// nothing looked: libheif was in no package list and a comment said it was. So
// the cases here plant each half of that back and require the check to say so,
// rather than trusting that a green run on the real tree means it works.
//
// Run: node dev/scripts/test-check-linked-libraries.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-linked-libraries.py");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// A tree with the two package lists and one crate manifest.
function tree({ deps, packages, manifest, at = "apps/viewers/decode-heic" }) {
  const dir = mint("linked-libs-");
  mkdirSync(join(dir, "dev/mkosi/mkosi.build.d"), { recursive: true });
  writeFileSync(join(dir, "dev/mkosi/mkosi.build.d/01-install-deps.sh"), deps);
  writeFileSync(
    join(dir, "dev/mkosi/mkosi.conf"),
    `[Distribution]\nDistribution=debian\n\n[Content]\nPackages=\n${packages}\n\n[Host]\nQemuMem=4G\n`,
  );
  mkdirSync(join(dir, at), { recursive: true });
  writeFileSync(join(dir, at, "Cargo.toml"), manifest);
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const HEIC = '[package]\nname = "x"\n\n[dependencies]\nlibheif-rs = { version = "2" }\n';
const GOOD_DEPS = "apt-get install -y libssl-dev libheif-dev\n";
const GOOD_PKGS = "        libheif1\n        libheif-plugin-libde265\n        libheif-plugin-dav1d";

console.log("check-linked-libraries:");

{
  const d = tree({ deps: GOOD_DEPS, packages: GOOD_PKGS, manifest: HEIC });
  const r = run(d);
  check("both sides declared is a pass", r.code === 0, r.out);
  cleanup(d);
}

// The half that actually happened, and the one that stopped the build.
{
  const d = tree({ deps: "apt-get install -y libssl-dev\n", packages: GOOD_PKGS, manifest: HEIC });
  const r = run(d);
  check(
    "a missing dev package is caught",
    r.code === 1 && r.out.includes("libheif-dev"),
    r.out,
  );
  cleanup(d);
}

// The worse half: this one builds fine and fails in front of a person.
{
  const d = tree({ deps: GOOD_DEPS, packages: "        libgtk-3-0", manifest: HEIC });
  const r = run(d);
  check(
    "a missing runtime library is caught",
    r.code === 1 && r.out.includes("libheif1"),
    r.out,
  );
  cleanup(d);
}

// Naming the container library is not enough when the codecs are plugins.
{
  const d = tree({ deps: GOOD_DEPS, packages: "        libheif1", manifest: HEIC });
  const r = run(d);
  check(
    "a codec plugin left out is still a finding",
    r.code === 1 && r.out.includes("libheif-plugin-libde265"),
    r.out,
  );
  cleanup(d);
}

// So a new C-linking crate cannot arrive with nobody noticing, which is the whole
// failure mode: the table only helps for what is in it.
{
  const d = tree({
    deps: GOOD_DEPS,
    packages: GOOD_PKGS,
    manifest: '[package]\nname = "x"\n\n[dependencies]\nfoobar-sys = "1"\n',
  });
  const r = run(d);
  check(
    "an undeclared -sys dependency is refused",
    r.code === 1 && r.out.includes("foobar-sys"),
    r.out,
  );
  cleanup(d);
}

// The roots beyond apps/ and daemons/, widened 18 August. A claim about scope is
// worth exactly as much as a fixture that lands outside the old one.
for (const at of ["sdk/net-guard", "contracts/capsule", "forage/store"]) {
  const d = tree({
    deps: GOOD_DEPS,
    packages: GOOD_PKGS,
    manifest: '[package]\nname = "x"\n\n[dependencies]\nfoobar-sys = "1"\n',
    at,
  });
  const r = run(d);
  check(`a -sys dependency under ${at} is seen`, r.code === 1 && r.out.includes("foobar-sys"), r.out);
  cleanup(d);
}

// A name inside a features list is not a dependency; reading it as one would make
// the check cry wolf and get itself ignored.
{
  const d = tree({
    deps: GOOD_DEPS,
    packages: GOOD_PKGS,
    manifest: '[package]\nname = "x"\n\n[dependencies]\nserde = { version = "1", features = ["foobar-sys"] }\n',
  });
  const r = run(d);
  check("a features entry is not read as a dependency", r.code === 0, r.out);
  cleanup(d);
}

// Pointed at a tree with no manifests, "nothing wrong" would describe a scan that
// read nothing.
{
  const d = mint("linked-libs-empty-");
  mkdirSync(join(d, "dev/mkosi/mkosi.build.d"), { recursive: true });
  writeFileSync(join(d, "dev/mkosi/mkosi.build.d/01-install-deps.sh"), GOOD_DEPS);
  writeFileSync(join(d, "dev/mkosi/mkosi.conf"), "Packages=\n        libheif1\n");
  check("a tree with no crates is an error, not a pass", run(d).code === 2);
  cleanup(d);
}

console.log(failures ? `\n${failures} case(s) failed` : "\nboth sides of a linked library are held");
process.exit(failures ? 1 : 0);
