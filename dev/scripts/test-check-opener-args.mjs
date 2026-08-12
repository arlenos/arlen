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

// Both ACKNOWLEDGED entries are claims about THIS tree, and the staleness guard
// added on 12 Aug reports an entry that excused nothing. A fixture has neither
// file, so every fixture carries a stub bearing the witness its reason names, or
// the guard correctly fires for entries that are perfectly healthy. Same trade as
// `test-check-spawned-binaries`: the coupling to a list in the file under test
// runs the cheap way, since adding an entry without a stub turns THIS test red.
const CARRIED = {
  "apps/files/src-tauri/src/lib.rs": open("        .arg(abs(&path))\n"),
  "apps/harness/src-tauri/src/file_ref.rs": open("        .arg(path)\n"),
};

console.log("check-opener-args:");

check(
  "a call a dash-leading name would defeat is caught",
  { ...CARRIED, "apps/probe/src-tauri/src/lib.rs": open("        .arg(path)\n") },
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "the same call with an end-of-options marker passes",
  { ...CARRIED, "apps/probe/src-tauri/src/lib.rs": open('        .arg("--")\n        .arg(path)\n') },
  (code) => code === 0,
);

// The rule the header records from 9 August: an excuse belongs to a CALL that
// carries its witness, not to a file. A second, unguarded call added to an
// excused file inherited the excuse and the check said nothing - so both halves
// of that pairing are worth holding down.
check(
  "an excused file still passes the call its reason was written for",
  {
    ...CARRIED,
    "apps/files/src-tauri/src/lib.rs": open('        .arg(abs(&path))\n'),
  },
  (code, out) => code === 0 && out.includes("Checked the function, not the name"),
);

check(
  "a different call in that same file does not inherit the excuse",
  {
    ...CARRIED,
    "apps/files/src-tauri/src/lib.rs":
      open('        .arg(abs(&path))\n') + "\n" + open("        .arg(other)\n"),
  },
  (code, out) => code === 1 && out.includes("apps/files"),
);

// The defect this test was written for. A count of zero is only honest when
// there was something to count; no Rust sources at all means the check went
// quiet, and quiet must not read as clean.
// The staleness half. An entry says a specific call is unguarded and why that is
// tolerable; the day the call gains its `--`, the sentence describes a hole that
// is closed, which reads as a known problem still open. Here the harness stub is
// given the marker, so its entry excuses nothing.
check(
  "an entry whose call has since been fixed is caught",
  {
    ...CARRIED,
    "apps/harness/src-tauri/src/file_ref.rs": open('        .arg("--")\n        .arg(path)\n'),
  },
  (code, out) => code === 1 && out.includes("apps/harness"),
);

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
