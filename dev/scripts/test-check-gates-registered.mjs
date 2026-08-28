// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-gates-registered.
//
// The defect is real and dated: `check-image-contents.sh` sat in dev/scripts
// correct and unread until 11 August, when it turned out to be the one file
// nothing invoked. `run-ci-gates.sh` finds its work by grepping the workflow, so
// an unregistered check is invisible to the local runner and to CI at once.
//
// Run: node dev/scripts/test-check-gates-registered.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-gates-registered.py");
const failures = [];

function run(name, files, expect) {
  const dir = mint("arlen-gates-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  cleanup(dir);
}

const WORKFLOW = (...names) =>
  `jobs:\n  gates:\n    steps:\n${names.map((n) => `      - run: dev/scripts/${n}\n`).join("")}`;

console.log("gates registered:");

run(
  "a check the workflow never names is caught",
  {
    "dev/scripts/check-alpha.py": "#!/usr/bin/env python3\n",
    "dev/scripts/check-beta.py": "#!/usr/bin/env python3\n",
    ".github/workflows/ci.yml": WORKFLOW("check-alpha.py"),
  },
  (code, out) => code === 1 && out.includes("check-beta.py"),
);

run(
  "every check registered passes",
  {
    "dev/scripts/check-alpha.py": "#!/usr/bin/env python3\n",
    "dev/scripts/check-beta.mjs": "// control\n",
    ".github/workflows/ci.yml": WORKFLOW("check-alpha.py", "check-beta.mjs"),
  },
  (code) => code === 0,
);

run(
  "a recorded exception the workflow now names is reported as left behind",
  {
    "dev/scripts/check-profile-case.sh": "#!/usr/bin/env bash\n",
    ".github/workflows/ci.yml": WORKFLOW("check-profile-case.sh"),
  },
  (code, out) => code === 1 && out.includes("left behind"),
);

run(
  "a recorded exception the workflow does not name is allowed",
  {
    "dev/scripts/check-profile-case.sh": "#!/usr/bin/env bash\n",
    ".github/workflows/ci.yml": WORKFLOW(),
  },
  (code) => code === 0,
);

run(
  "a tree with no checks at all reports that it read nothing",
  { ".github/workflows/ci.yml": WORKFLOW() },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a control is not mistaken for a check",
  {
    "dev/scripts/check-alpha.py": "#!/usr/bin/env python3\n",
    "dev/scripts/test-check-alpha.mjs": "// names dev/scripts/check-alpha.py\n",
    ".github/workflows/ci.yml": WORKFLOW("check-alpha.py"),
  },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a check that ships is a check that runs");
