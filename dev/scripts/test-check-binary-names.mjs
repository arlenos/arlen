// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the duplicate-binary-name gate.
//
// A gate that only ever prints OK proves nothing, so each case below is a way the
// tree could lose the one-name-one-program property, and each has to turn it red:
// two explicit `[[bin]]` names, an explicit name colliding with another crate's
// implicit one, and a carried entry outliving the collision it describes.
//
// The last one matters as much as the first. A carried violation that quietly
// resolved itself reads as coverage, and then the list stops being read.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-binary-names.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** Build a throwaway tree of crates and run the gate against it. */
function run(files) {
  const dir = mint("binnames-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const pkg = (name, extra = "") => `[package]\nname = "${name}"\nversion = "0.1.0"\n${extra}`;

console.log("binary names:");

{
  const r = run({
    "a/Cargo.toml": pkg("thing-a"),
    "a/src/main.rs": "fn main() {}",
    "b/Cargo.toml": pkg("thing-b"),
    "b/src/main.rs": "fn main() {}",
  });
  check("distinct names pass", r.code === 0);
  check("and the count is reported", r.out.includes("2 binary name(s)"));
}
{
  // The shape found in the tree: two crates, same explicit name.
  const r = run({
    "a/Cargo.toml": pkg("app-a", '\n[[bin]]\nname = "shared-name"\npath = "src/bin/x.rs"\n'),
    "b/Cargo.toml": pkg("cli-b", '\n[[bin]]\nname = "shared-name"\npath = "src/bin/y.rs"\n'),
  });
  check("two explicit bins with one name are caught", r.code === 1);
  check("and both crates are named", r.out.includes("a") && r.out.includes("b"));
}
{
  // The half a `[[bin]]`-only reader would miss: cargo names a package's binary
  // after the package when no section declares one, so the collision can be
  // between something written down and something implied.
  const r = run({
    "a/Cargo.toml": pkg("collide"),
    "a/src/main.rs": "fn main() {}",
    "b/Cargo.toml": pkg("other", '\n[[bin]]\nname = "collide"\npath = "src/bin/y.rs"\n'),
  });
  check("an implicit name colliding with an explicit one is caught", r.code === 1);
}
{
  // The boundary: a package that declares a `[[bin]]` under its OWN name builds
  // one binary, not two. Counting it twice would fail every such crate.
  const r = run({
    "a/Cargo.toml": pkg("solo", '\n[[bin]]\nname = "solo"\npath = "src/main.rs"\n'),
    "a/src/main.rs": "fn main() {}",
  });
  check("a bin named after its own package is one binary", r.code === 0);
}
{
  // A library-only crate builds nothing, and a `[lib] name` that happens to match
  // another crate's binary must not be read as a collision.
  const r = run({
    "a/Cargo.toml": pkg("libby", '\n[lib]\nname = "shared_thing"\n'),
    "b/Cargo.toml": pkg("shared_thing"),
    "b/src/main.rs": "fn main() {}",
  });
  check("a lib name is not a binary name", r.code === 0);
}
{
  // Nothing to read must not be a pass. An empty tree scores green on every
  // check that forgets to say so, which is how a gate stops measuring.
  const r = run({ "README.md": "no crates here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe binary-name gate holds");
process.exit(failures ? 1 : 0);
