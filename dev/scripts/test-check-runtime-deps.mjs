// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-runtime-deps, including the one the directive
// named: removing a package from the image must turn it red.

import { mkdirSync, writeFileSync, cpSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "../..");
const GATE = join(HERE, "check-runtime-deps.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// A tree the gate can read: a Packages= block and some Rust that spawns things.
// The TSV is NOT copied - the gate reads the real one beside itself, which is the
// point: the list is the repo's, only the tree it is checked against varies.
function tree({ packages, calls }) {
  const dir = mint("runtime-deps-");
  mkdirSync(join(dir, "dev/mkosi"), { recursive: true });
  writeFileSync(
    join(dir, "dev/mkosi/mkosi.conf"),
    "[Content]\nPackages=\n" + packages.map((p) => `        ${p}`).join("\n") + "\n\n[Host]\n"
  );
  mkdirSync(join(dir, "daemons/x/src"), { recursive: true });
  writeFileSync(
    join(dir, "daemons/x/src/main.rs"),
    calls.map((c) => `fn f() { Command::new("${c}"); }`).join("\n") + "\n"
  );
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// The real tree passes.
check("the repository as it stands passes", run(ROOT).code === 0);

// The done-when the directive asked for: drop a needed package, go red. `fuse3`
// stands in for polkitd - same shape, and it is one the TSV marks `ships`.
{
  const listed = ["bubblewrap", "fontconfig"]; // fuse3 deliberately omitted
  // One call, and one the TSV already lists, so the scan is not empty and the
  // completeness half has nothing to say - leaving the shipping half alone.
  const d = tree({ packages: listed, calls: ["sh"] });
  const r = run(d);
  check("removing a needed package from the image turns it red", r.code === 1);
  check("and it names the package that went missing", r.out.includes("fuse3"));
  cleanup(d);
}

// A shell-out nobody classified: the defect that let twenty-one binaries go
// missing quietly in the first place.
{
  const d = tree({
    packages: ["bubblewrap", "fontconfig", "fuse3"],
    calls: ["some-tool-nobody-listed"],
  });
  const r = run(d);
  check("an unlisted Command::new is caught", r.code === 1);
  check("and the message names the binary", r.out.includes("some-tool-nobody-listed"));
  cleanup(d);
}

// A tree with no Packages= block at all must not pass by finding nothing to check.
{
  const dir = mint("runtime-deps-nopkg-");
  mkdirSync(join(dir, "daemons/x/src"), { recursive: true });
  writeFileSync(join(dir, "daemons/x/src/main.rs"), 'fn f() { Command::new("sh"); }\n');
  const r = run(dir);
  check("a tree with no Packages block is refused, not passed", r.code === 1);
  cleanup(dir);
}

// And a tree with no Rust at all is a scan that read nothing.
{
  const dir = mint("runtime-deps-empty-");
  check("a tree with no sources is an error, not a pass", run(dir).code === 2);
  cleanup(dir);
}

{
  // The two checks ask opposite directions of one question - is every spawn
  // classified, is every classification still spawned - so a tree only one of
  // them reads is a place where a tool can slip through either way. That is not
  // hypothetical: `appstreamcli` was classified in one and missing from the
  // other because `forage` was in one list and not the other.
  const scan = readFileSync(join(HERE, "check-runtime-deps.py"), "utf8");
  const trees = readFileSync(join(HERE, "check-spawned-tools-classified.py"), "utf8");
  const one = [...(scan.match(/SCAN = \(([^)]*)\)/)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  const two = [...(trees.match(/TREES = \[([^\]]*)\]/)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (one.length === 0 || one.join(",") !== two.join(",")) {
    console.log(`       runtime-deps: ${one.join(",")}`);
    console.log(`       classified:   ${two.join(",")}`);
  }
  check("both checks scan the same trees", one.length > 0 && one.join(",") === two.join(","));
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
