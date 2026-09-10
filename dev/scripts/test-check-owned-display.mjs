// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-owned-display.py`. The fault is a script that starts an
// Xvfb itself; the near-misses it must not report are a script that only CHECKS
// the binary is installed, a comment naming it, and the helper that is allowed to
// be the one caller.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const gate = path.join(here, "check-owned-display.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failed += 1;
    console.log(`  FAIL ${name}`);
    if (detail) console.log(`       ${detail}`);
  }
}

//: `files` is name -> contents, written under `dev/`. The helper is written for
//: every case unless `withHelper` says otherwise, because the check refuses a
//: tree where the file it names is missing.
function gateOver(files, withHelper = true) {
  const dir = mint("arlen-owned-display-");
  try {
    const lib = path.join(dir, "dev", "screenshot", "lib");
    mkdirSync(lib, { recursive: true });
    if (withHelper) {
      writeFileSync(
        path.join(lib, "own-display.sh"),
        'own_display() {\n  xvfb-run -n "$n" --server-args="$1" "$@"\n}\n',
        "utf8",
      );
    }
    for (const [name, body] of Object.entries(files)) {
      const p = path.join(dir, "dev", name);
      mkdirSync(path.dirname(p), { recursive: true });
      writeFileSync(p, body, "utf8");
    }
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("owned-display:");

{
  const r = gateOver({
    "screenshot/shoot.sh": '. lib/own-display.sh\nown_display "-screen 0 800x600x24" true\n',
  });
  check("a script that goes through the helper passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver({
    "screenshot/shoot.sh": 'xvfb-run -a --server-args="-screen 0 800x600x24" true\n',
  });
  check("a script that races for a display is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the file and line", r.out.includes("screenshot/shoot.sh:1"));
}

{
  const r = gateOver({
    "screenshot/shoot.sh":
      'command -v xvfb-run >/dev/null || exit 1\nfor bin in xvfb-run xdotool; do :; done\n# xvfb-run -a is what this used to do\n. lib/own-display.sh\n',
  });
  check("checking the binary exists is not invoking it", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver({
    "screenshot/drive.sh": 'out=$(xvfb-run -a --server-args="-screen 0 800x600x24" true)\n',
  });
  check("an invocation inside a substitution is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

{
  const r = gateOver({ "screenshot/shoot.sh": ". lib/own-display.sh\n" }, false);
  check("a tree with no helper is a refusal, not a pass", r.code === 1, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("every Xvfb starts on a display its caller owns");
