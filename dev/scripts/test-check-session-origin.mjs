// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the session-origin gate must catch, and what it must leave alone.
//
// The shape it exists for is the one that actually happened on 10 August: the
// producers were taught to read `ARLEN_SESSION_ID` and the launchers were not
// updated to supply it, so ten integration tests went red and all three dev
// stacks were silently in the same state. A gate for that is only worth having if
// it has been seen to fail, so every case below is built and run.
//
// Run: node dev/scripts/test-check-session-origin.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-session-origin.py");

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-session-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  // Both streams, always: a case that asserted on stdout alone would compare
  // against an empty string whenever the gate wrote to stderr, and pass for the
  // wrong reason. That has bitten the sibling gate tests twice.
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

const WITHOUT = `version: "0.5"

environment:
  - "ARLEN_RUNTIME_DIR=\${XDG_RUNTIME_DIR}/arlen"

processes:
  event-bus:
    command: cargo run --bin event-bus
`;

const WITH = WITHOUT.replace(
  'environment:\n',
  'environment:\n  - "ARLEN_SESSION_ID=dev-\${USER}"\n',
);

// No processes at all: a fragment that starts nothing launches no producer, and
// flagging it would teach people to add an id where it means nothing.
const NO_PROCESSES = `version: "0.5"

environment:
  - "ARLEN_RUNTIME_DIR=\${XDG_RUNTIME_DIR}/arlen"
`;

const SESSION_SCRIPT = `#!/bin/sh
ARLEN_SESSION_ID="$(systemd-id128 new)"
export ARLEN_SESSION_ID
`;

console.log("check-session-origin:");

check(
  "a stack that starts processes without the id is caught",
  tree({
    "dev/process-compose.yaml": WITHOUT,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-session": SESSION_SCRIPT,
  }),
  (code, out) => code === 1 && out.includes("empty origin"),
);

check(
  "a stack that supplies it passes",
  tree({
    "dev/process-compose.yaml": WITH,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-session": SESSION_SCRIPT,
  }),
  (code) => code === 0,
);

check(
  "every stack is checked, not just the first",
  tree({
    "dev/process-compose.yaml": WITH,
    "dev/process-compose.shell.yaml": WITHOUT,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-session": SESSION_SCRIPT,
  }),
  (code, out) => code === 1 && out.includes("process-compose.shell.yaml"),
);

check(
  "a file that starts nothing is left alone",
  tree({
    "dev/process-compose.yaml": NO_PROCESSES,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-session": SESSION_SCRIPT,
  }),
  (code) => code === 0,
);

check(
  "the session script losing the mint is caught",
  tree({
    "dev/process-compose.yaml": WITH,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-session": "#!/bin/sh\nexec cosmic-comp\n",
  }),
  (code, out) => code === 1 && out.includes("no longer mints"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all session-origin gate cases passed");
