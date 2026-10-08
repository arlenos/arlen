// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Do the gates read what git knows about, and does the guard notice one that does not?
//
// Two halves. The guard (`check-gates-read-tracked.py`) must name a walking gate
// that forgot the import, over a fixture directory rather than this one. And
// `tracked_walk` itself must do what it says inside a real repository: skip an
// ignored directory - the image build's cache, on 8 October - while still finding
// a tracked file and a new one nobody has added yet.

import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const scripts = path.dirname(fileURLToPath(import.meta.url));
const gate = path.join(scripts, "check-gates-read-tracked.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

const run = (dir) => spawnSync("python3", [gate, dir], { encoding: "utf8" });

check("the gate passes on the tree as it stands", run(scripts).status === 0, run(scripts).stdout);

const fixture = mint("gates-tracked-");
try {
  writeFileSync(path.join(fixture, "check-good.py"), "import tracked_walk\nfrom pathlib import Path\nPath('.').rglob('*.rs')\n");
  writeFileSync(path.join(fixture, "check-bare.py"), "from pathlib import Path\nfor p in Path('.').rglob('*.rs'):\n    pass\n");
  writeFileSync(path.join(fixture, "check-flat.py"), "from pathlib import Path\nPath('.').glob('*.toml')\n");
  const red = run(fixture);
  check("a walking gate without the import is named", red.status === 1 && red.stdout.includes("check-bare.py"), red.stdout);
  check("a gate that imports it is not", !red.stdout.includes("check-good.py"), red.stdout);
  check("a one-level glob is not a walk", !red.stdout.includes("check-flat.py"), red.stdout);
  const empty = mint("gates-empty-");
  check("an empty directory is refused, not passed", run(empty).status === 1);
  cleanup(empty);
} finally {
  cleanup(fixture);
}

// The walk itself, in a throwaway repository.
const repo = mint("tracked-walk-");
try {
  const git = (...a) => execFileSync("git", ["-C", repo, ...a], { encoding: "utf8" });
  git("init", "-q");
  mkdirSync(path.join(repo, "src"), { recursive: true });
  mkdirSync(path.join(repo, "cache", "registry"), { recursive: true });
  writeFileSync(path.join(repo, ".gitignore"), "/cache/\n");
  writeFileSync(path.join(repo, "src", "tracked.rs"), "");
  writeFileSync(path.join(repo, "src", "new.rs"), "");
  writeFileSync(path.join(repo, "cache", "registry", "vendored.rs"), "");
  git("add", ".gitignore", "src/tracked.rs");
  const py = `
import sys, os, pathlib
sys.path.insert(0, ${JSON.stringify(scripts)})
import tracked_walk
root = pathlib.Path(${JSON.stringify(repo)})
print(sorted(str(p.relative_to(root)) for p in root.rglob("*.rs")))
print(sorted(os.path.relpath(os.path.join(d, f), root) for d, _, fs in os.walk(root) for f in fs if f.endswith(".rs")))
`;
  const out = spawnSync("python3", ["-c", py], { encoding: "utf8" });
  const [rg, wk] = out.stdout.trim().split("\n");
  const want = "['src/new.rs', 'src/tracked.rs']";
  check("rglob skips the ignored cache and keeps tracked and new files", rg === want, `${rg} ${out.stderr}`);
  check("os.walk does the same", wk === want, wk);
} finally {
  cleanup(repo);
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log("a gate reads what git knows about, and one that does not is named");
