// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does run-ci-gates.sh keep to ARLEN_GATE_JOBS, and keep its report in CI's order?
//
// A fixture repository with eight fake gates listed in a fake ci.yml. Each gate
// counts itself in and out through a lock file while it sleeps, so the most that
// ever ran at once is recorded. Until 8 October the runner started every gate at
// once, 177 processes per commit, and nothing measured it.

import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

const GATE = `#!/usr/bin/env bash
# counts itself in, sleeps, counts itself out; records the peak
d="$(dirname "$0")/../../run"
exec 9>"$d/lock"
flock 9; n=$(( $(cat "$d/now") + 1 )); echo $n > "$d/now"
[ "$n" -gt "$(cat "$d/peak")" ] && echo $n > "$d/peak"; flock -u 9
sleep 0.3
flock 9; echo $(( $(cat "$d/now") - 1 )) > "$d/now"; flock -u 9
`;

function run(jobs) {
  const root = mint("run-ci-gates-");
  try {
    mkdirSync(join(root, "dev/scripts"), { recursive: true });
    mkdirSync(join(root, ".github/workflows"), { recursive: true });
    mkdirSync(join(root, "run"));
    writeFileSync(join(root, "run/now"), "0\n");
    writeFileSync(join(root, "run/peak"), "0\n");
    copyFileSync(join(here, "run-ci-gates.sh"), join(root, "dev/scripts/run-ci-gates.sh"));
    const names = [];
    for (let i = 0; i < 8; i++) {
      const n = `gate-${String.fromCharCode(104 - i)}.sh`; // listed h..a, so not alphabetical
      writeFileSync(join(root, "dev/scripts", n), GATE);
      names.push(`dev/scripts/${n}`);
    }
    writeFileSync(
      join(root, ".github/workflows/ci.yml"),
      `jobs:\n  x:\n    steps:\n      - run: |\n${names.map((n) => `          bash ${n}\n`).join("")}`,
    );
    const env = { ...process.env, ARLEN_GATE_JOBS: String(jobs) };
    const r = spawnSync("bash", [join(root, "dev/scripts/run-ci-gates.sh")], { encoding: "utf8", env });
    const peak = Number(readFileSync(join(root, "run/peak"), "utf8").trim());
    const order = r.stdout.split("\n").filter((l) => l.startsWith("dev/scripts/gate-")).map((l) => l.split(/\s+/)[0]);
    return { code: r.status, peak, order, names, out: r.stdout + r.stderr };
  } finally {
    cleanup(root);
  }
}

let r = run(2);
check("all eight gates pass and the runner exits 0", r.code === 0 && r.order.length === 8, r.out);
check("no more than two ran at once", r.peak >= 1 && r.peak <= 2, `peak ${r.peak}`);
check("the report keeps CI's order, not finishing order", JSON.stringify(r.order) === JSON.stringify(r.names), r.order.join(" "));

r = run(8);
check("with eight slots they do run together", r.peak > 2, `peak ${r.peak}`);

r = run("nonsense");
check("a value that is not a number falls back to four", r.code === 0 && r.peak <= 4, `peak ${r.peak}`);

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log("run-ci-gates: control cases pass.");
