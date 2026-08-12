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

import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-opener-args.py");

const failures = [];

function check(name, files, expect, entry) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-opener-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // `entry` drives a COPY of the gate carrying one fixture ACKNOWLEDGED entry.
  // The real list is empty and should stay empty, but the excuse mechanism is
  // where a file-wide hole lived once, so it stays covered - against an entry
  // this test owns rather than against whatever the tree happens to excuse. No
  // env override on the gate itself: a production-reachable way to inject an
  // excuse is a worse thing to add than a copied file in a temp dir.
  let gate = GATE;
  if (entry) {
    gate = join(dir, "gate.py");
    writeFileSync(
      gate,
      readFileSync(GATE, "utf8").replace(
        "ACKNOWLEDGED: dict[str, tuple[str, str]] = {}",
        `ACKNOWLEDGED = {${JSON.stringify(entry.file)}: (${JSON.stringify(entry.witness)}, ${JSON.stringify(entry.reason)})}`,
      ),
    );
  }
  const r = spawnSync("python3", [gate, dir], { encoding: "utf8" });
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

// ACKNOWLEDGED is empty, so there is nothing to carry. It held one entry until
// 12 Aug - the harness's `spawn_xdg_open` - and the fixtures each planted a stub
// bearing that entry's witness, or the staleness guard fired for an entry that
// was perfectly healthy. The harness asks the launch socket now, the entry went
// with the call, and the stubs went with the entry. If a future entry is added,
// the stub comes back with it: adding one without a stub turns THIS test red,
// which is the cheap end of the coupling and the reason it is done this way.

console.log("check-opener-args:");

check(
  "a call a dash-leading name would defeat is caught",
  { "apps/probe/src-tauri/src/lib.rs": open("        .arg(path)\n") },
  (code, out) => code === 1 && out.includes("probe"),
);

// This case used to assert the opposite, and asserting it is what kept the
// mistake alive: `--` was the remedy this gate ASKED for, and xdg-utils rejects
// it ("unexpected option '--'", measured 12 Aug). The advice came from a dev
// machine's personal `xdg-open` shim, which execs `handlr` and whose clap parser
// does honour the marker. Reported rather than failed while the packaging
// question is open, so the assertion is on the warning and on exit 0.
check(
  "a marker is reported as breaking rather than accepted",
  { "apps/probe/src-tauri/src/lib.rs": open('        .arg("--")\n        .arg(path)\n') },
  (code, out) => code === 0 && out.includes("xdg-utils REJECTS"),
);

// And the fix that does work: absolute by construction, which is now the guard
// itself rather than a reason to be excused from one.
check(
  "an argument made absolute first is accepted",
  { "apps/probe/src-tauri/src/lib.rs": open("        .arg(abs(&path))\n") },
  (code) => code === 0,
);

// The rule the header records from 9 August: an excuse belongs to a CALL that
// carries its witness, not to a file. A second, unguarded call added to an
// excused file inherited the excuse and the check said nothing - so both halves
// of that pairing are worth holding down.
check(
  "an excused file still passes the call its reason was written for",
  {
    "apps/probe/src-tauri/src/lib.rs": open("        .arg(arg)\n"),
  },
  (code, out) => code === 0 && out.includes("canonicalizes first"),
  {
    file: "apps/probe/src-tauri/src/lib.rs",
    witness: ".arg(arg)",
    reason: "the caller canonicalizes first, so the argument is absolute already",
  },
);

check(
  "a different call in that same file does not inherit the excuse",
  {
    "apps/probe/src-tauri/src/lib.rs":
      open("        .arg(arg)\n") + "\n" + open("        .arg(other_no_witness)\n"),
  },
  (code, out) => code === 1 && out.includes("apps/probe"),
  {
    file: "apps/probe/src-tauri/src/lib.rs",
    witness: ".arg(arg)",
    reason: "the caller canonicalizes first, so the argument is absolute already",
  },
);

// The defect this test was written for. A count of zero is only honest when
// there was something to count; no Rust sources at all means the check went
// quiet, and quiet must not read as clean.
// The staleness half. An entry says a specific call is unguarded and why that is
// tolerable; the day the call gains its `--`, the sentence describes a hole that
// is closed, which reads as a known problem still open. Here the fixture's call
// is made absolute, so the entry that excuses it excuses nothing.
check(
  "an entry whose call has since been fixed is caught",
  {
    "apps/probe/src-tauri/src/lib.rs": open("        .arg(abs(&path))\n"),
  },
  (code, out) => code === 1 && out.includes("no such call is there now"),
  {
    file: "apps/probe/src-tauri/src/lib.rs",
    witness: ".arg(arg)",
    reason: "the caller canonicalizes first, so the argument is absolute already",
  },
);

// xdg-mime came into scope on 12 Aug, measured rather than assumed. Both halves:
// it is checked at all, and the absolute step is found where it actually reads -
// on the line BEFORE the call, not inside the builder chain.
check(
  "an xdg-mime call with a bare path is caught too",
  {
    "apps/probe/src-tauri/src/lib.rs":
      "pub fn kind(path: &str) -> Option<String> {\n" +
      '    Command::new("xdg-mime")\n' +
      '        .args(["query", "filetype", path])\n' +
      "        .output()\n" +
      "        .ok()?;\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "an absolute step on the line before the call is accepted",
  {
    "apps/probe/src-tauri/src/lib.rs":
      "pub fn kind(path: &str) -> Option<String> {\n" +
      "    let real = std::fs::canonicalize(path).ok()?;\n" +
      '    Command::new("xdg-mime")\n' +
      '        .args(["query", "filetype", real.as_os_str()])\n' +
      "        .output()\n" +
      "        .ok()?;\n" +
      "}\n",
  },
  (code) => code === 0,
);

// The opposite rule, for the tool that takes the opposite fix. Both directions,
// because a gate that enforced one rule on both families would break whichever it
// got wrong - and it very nearly did: the `--` this case requires was pushed out
// of the scan window by a long comment above it, and the gate reported the call it
// had just been taught to accept.
check(
  "a gtk-launch call without the marker is caught",
  {
    "apps/probe/src-tauri/src/lib.rs":
      "pub fn go(entry: &str) {\n" +
      '    Command::new("gtk-launch")\n' +
      "        .arg(entry)\n" +
      "        .spawn()\n" +
      "        .ok();\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("gtk-launch"),
);

check(
  "a gtk-launch call with the marker passes, and a long comment does not hide it",
  {
    "apps/probe/src-tauri/src/lib.rs":
      "pub fn go(entry: &str) {\n" +
      '    Command::new("gtk-launch")\n' +
      ("        // padding that used to push the argument out of the window.\n".repeat(12)) +
      '        .arg("--")\n' +
      "        .arg(entry)\n" +
      "        .spawn()\n" +
      "        .ok();\n" +
      "}\n",
  },
  (code) => code === 0,
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
