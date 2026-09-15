// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the unit-has-a-binary gate see a unit for a daemon nothing builds?
//
// The fault is put in and taken out over a FIXTURE tree, not this one: the gates
// run concurrently, so a control writing into `daemons/` would be visible to its
// neighbours mid-run.
//
// Three near-misses carry the weight, and each is a real thing this tree does:
// a binary built by the cross-build TABLE rather than a chroot phase (the event
// bus, which a first measurement got wrong and reported three units for), an
// `ExecStart` that belongs to the distribution (`busctl`, in installd's trash
// timer), and a unit that is deliberately ahead of the image and says so.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-unit-has-a-binary.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// A fixture tree with one unit, plus whatever image build is described.
function gateOver(unit, { phase = "", table = "" } = {}) {
  const dir = mint("arlen-unit-binary-");
  try {
    const dist = path.join(dir, "daemons", "demo", "dist");
    mkdirSync(dist, { recursive: true });
    writeFileSync(path.join(dist, "arlen-demod.service"), unit, "utf8");
    const mkosi = path.join(dir, "dev", "mkosi", "mkosi.build.d");
    mkdirSync(mkosi, { recursive: true });
    writeFileSync(path.join(mkosi, "10-demo.sh.chroot"), phase || "# nothing\n", "utf8");
    writeFileSync(path.join(dir, "dev", "mkosi", "build-image.sh"), table || "# nothing\n", "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

const UNIT = (exec, extra = "") =>
  `[Unit]\nDescription=Demo\n${extra}\n[Service]\nExecStart=${exec}\n`;

console.log("unit has a binary:");

{
  const r = gateOver(UNIT("/usr/lib/arlen/libexec/arlen-demod"));
  check("a unit whose binary nothing builds is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the binary", r.out.includes("arlen-demod"), r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(UNIT("/usr/lib/arlen/libexec/arlen-demod"), {
    phase: 'install -Dm755 target/release/arlen-demod "$BUILDROOT/usr/lib/arlen/libexec/arlen-demod"\n',
  });
  check("a chroot phase that installs it passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // The event bus's shape: built by the cross-build table, not by a phase.
  const r = gateOver(UNIT("/usr/bin/arlen-demod"), {
    table: "daemons/demo:arlen-demod:/usr/bin/arlen-demod\n",
  });
  check("the cross-build table counts as building it", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // installd's trash timer: the ExecStart is the distribution's.
  const r = gateOver(UNIT("/usr/bin/busctl"));
  check("a distribution binary is not this check's business", r.code === 0,
        r.out.trim().split("\n").pop());
}

{
  const r = gateOver(UNIT("/usr/lib/arlen/libexec/arlen-demod",
    "# NOT ON THE IMAGE: waiting on the per-uid sockets PR-R1 builds.\n"));
  check("a unit that says why it is ahead of the image passes", r.code === 0,
        r.out.trim().split("\n").pop());
  check("and the reason is printed where a person reads it",
        r.out.includes("per-uid sockets"), r.out.trim().split("\n")[0]);
}

{
  // A tree with no units at all, and one with no build: both are the scan
  // pointed wrong rather than a clean run.
  const dir = mint("arlen-unit-binary-empty-");
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  } finally {
    cleanup(dir);
  }
  check("an empty tree is refused, not called clean", r.code === 1, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate finds a unit for a binary the image does not build");
