// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Written the moment this gate's own defect surfaced. Giving it a root argument
// was meant to be housekeeping; pointing it at an empty directory made it answer
// "0 calls checked" and exit 0. It had been reporting green for a tree it never
// read, and nobody could have seen that while it could only ever run against the
// one tree it was written against.
//
// So the last case here is not decoration - it is the defect, pinned.
//
// Run: node dev/scripts/test-check-opener-args.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-opener-args.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-opener-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const open = (chain) =>
  "pub fn show(path: &str) {\n" +
  '    Command::new("xdg-open")\n' +
  chain +
  "        .spawn()\n" +
  "        .ok();\n" +
  "}\n";

console.log("check-opener-args:");

check(
  "a call a dash-leading name would defeat is caught",
  { "apps/probe/src-tauri/src/lib.rs": open("        .arg(path)\n") },
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "the same call with an end-of-options marker passes",
  { "apps/probe/src-tauri/src/lib.rs": open('        .arg("--")\n        .arg(path)\n') },
  (code) => code === 0,
);

// The rule the header records from 9 August: an excuse belongs to a CALL that
// carries its witness, not to a file. A second, unguarded call added to an
// excused file inherited the excuse and the check said nothing - so both halves
// of that pairing are worth holding down.
check(
  "an excused file still passes the call its reason was written for",
  {
    "apps/files/src-tauri/src/lib.rs": open('        .arg(abs(&path))\n'),
  },
  (code, out) => code === 0 && out.includes("Checked the function, not the name"),
);

check(
  "a different call in that same file does not inherit the excuse",
  {
    "apps/files/src-tauri/src/lib.rs":
      open('        .arg(abs(&path))\n') + "\n" + open("        .arg(other)\n"),
  },
  (code, out) => code === 1 && out.includes("apps/files"),
);

// The defect this test was written for. A count of zero is only honest when
// there was something to count; no Rust sources at all means the check went
// quiet, and quiet must not read as clean.
check(
  "an empty tree is a moved layout, not a pass",
  { "README.md": "nothing here\n" },
  (code, out) => code === 1 && out.includes("layout moved"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("the unguarded call is caught, the excuse stays with its own call, silence is not a pass");
