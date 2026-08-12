// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate that keeps this directory honest, and it had never been shown to
// fail. Written on the night it caught its own author twice: giving two probes a
// justfile recipe made their `CANNOT_BE_WIRED` entries untrue, and the staleness
// guard said so both times. A rule that good deserves to be pinned rather than
// trusted.
//
// Run: node dev/scripts/test-check-wired.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-wired.py");

const failures = [];

// The gate's exemption list is hardcoded, and it checks BOTH directions - an
// entry naming a script that no longer exists is as stale as one naming a script
// that is run. So a fixture that expects a clean pass has to carry the exempted
// scripts, or the gate correctly reports them missing. Learned by writing the
// test and watching the "clean" case go red for a reason that was not its own.
// This mirrors the gate's own CANNOT_BE_WIRED keys, which couples the test to a
// list in the file it tests. Deliberate, and the coupling is the cheap direction:
// if somebody adds an exemption without adding the stub here, THIS test goes red
// with "no longer exists" and they find out immediately. Written after including
// a script the list no longer names - the entry was deleted earlier the same
// evening - and watching the clean case fail for that reason.
const EXEMPTED = {
  "dev/scripts/probe-webview-sandbox.sh": "#!/bin/sh\necho hi\n",
};

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-wired-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // The gate enumerates callers with `git ls-files`, so the fixture is a repo.
  spawnSync("git", ["init", "-q"], { cwd: dir });
  spawnSync("git", ["add", "-A"], { cwd: dir });
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const CHECK = "dev/scripts/check-probe.py";
const script = "#!/usr/bin/env python3\nprint('ok')\n";

console.log("check-wired:");

// A gate whose whole subject is "silence reads like success" had the shape it
// checks for: pointed at a tree with no scripts directory it printed "nothing to
// check" and exited 0, and pointed at an empty one it exited 0 having examined
// nothing. Both are a wrong root, since the directory is committed source.
check(
  "no scripts directory at all is refused, not skipped",
  { "README.md": "no scripts here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a scripts directory holding nothing this gate reads is refused too",
  { "dev/scripts/README.md": "prose, not a check\n" },
  (code, out) => code === 2 && out.includes("no check, probe or smoke script"),
);

check(
  "a check nothing runs is caught",
  { [CHECK]: script, "dev/justfile": "lint:\n    echo nothing\n" },
  (code, out) => code === 1 && out.includes("check-probe.py"),
);

check(
  "the same check named by a recipe passes",
  {
    ...EXEMPTED,
    [CHECK]: script,
    "dev/justfile": "gates:\n    python3 dev/scripts/check-probe.py\n",
  },
  (code) => code === 0,
);

// The other half of the same rule, and the reason the fixture above needs those
// two files at all: an exemption for a script that has been deleted is an excuse
// outliving its subject.
check(
  "an exemption for something that no longer exists is caught",
  {
    [CHECK]: script,
    "dev/justfile": "gates:\n    python3 dev/scripts/check-probe.py\n",
  },
  (code, out) => code === 1 && out.includes("no longer exists"),
);

// A mention is not a run: the distinction the gate was written for, after four
// checks stayed quiet while being named in another gate's prose.
check(
  "being talked about in a docstring is not being run",
  {
    [CHECK]: script,
    "dev/scripts/check-other.py":
      "'''See check-probe.py for the part this cannot do.'''\n",
    "dev/justfile": "gates:\n    python3 dev/scripts/check-other.py\n",
  },
  (code, out) => code === 1 && out.includes("check-probe.py"),
);

// The exemption-decay direction, which is the one that fired on me twice in an
// evening: an excuse that has quietly become untrue.
check(
  "an exemption for something that IS run is caught",
  {
    "dev/scripts/probe-webview-sandbox.sh": "#!/bin/sh\necho hi\n",
    "dev/justfile": "probe:\n    bash dev/scripts/probe-webview-sandbox.sh\n",
  },
  (code, out) => code === 1 && out.includes("delete the entry"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("an unrun check is caught, a mention is not a run, and a stale excuse is caught too");
