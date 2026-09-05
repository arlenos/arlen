// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the packaged-unit gate still see a directive that drifted?
//
// The last of the ~130 checks without a control, as of 5 September. What it
// guards is a real deployment class: a daemon's canonical `dist/*.service` and
// the hand-maintained copy the image ships under `mkosi.extra` are separate
// files, so a hardening directive added to one and not the other deploys a unit
// that differs from the reviewed one. Its own header says that shape has already
// shipped an unaudited producer and a broken peer-auth sandbox.
//
// It compares DIRECTIVE lines only, so the two cases that matter are a comment
// reword (must pass) and a changed directive (must fail). Over a fixture tree -
// the gate takes its root as `$1`, and the hook runs the gates concurrently, so a
// control may not touch the real one.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-packaged-units.sh");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// A tree with one daemon's canonical unit and the image's copy of it.
function gateOver(canonical, packaged) {
  const dir = mint("arlen-packaged-units-");
  try {
    const dist = path.join(dir, "daemons", "probe", "dist");
    const extra = path.join(dir, "dev", "mkosi", "mkosi.extra", "usr", "lib", "systemd", "user");
    mkdirSync(dist, { recursive: true });
    mkdirSync(extra, { recursive: true });
    writeFileSync(path.join(dist, "arlen-probe.service"), canonical, "utf8");
    writeFileSync(path.join(extra, "arlen-probe.service"), packaged, "utf8");
    // A CHECKOUT, because part of this gate enumerates units through git and says
    // so: over a plain directory it prints "skipping the git-enumerated unit gates
    // (not a checkout)" and exits 0. Without this the passing cases here were
    // green over a gate that had declined to run, which the count pin below caught.
    execFileSync("git", ["init", "-q"], { cwd: dir });
    execFileSync("git", ["add", "-A"], { cwd: dir });
    try {
      return { code: 0, out: execFileSync("bash", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

const UNIT = (extra = "") =>
  `[Unit]\nDescription=Probe\n\n[Service]\nType=simple\n` +
  `ExecStart=/usr/bin/arlen-probe\nProtectSystem=strict\n${extra}\n[Install]\nWantedBy=default.target\n`;

console.log("packaged units:");

// The real tree, read-only.
{
  let r;
  try {
    r = { code: 0, out: execFileSync("bash", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const same = UNIT();
  const r = gateOver(same, same);
  check("two identical units pass", r.code === 0, r.out.trim().split("\n").pop());
  // Pinned on the COUNT, not on the unit name: a passing run does not name the
  // units it compared, and "0 packaged unit(s) match" also exits 0. Two earlier
  // versions of this case were green over a gate that had skipped entirely.
  check("and the gate actually compared the pair",
        r.out.includes("1 packaged unit(s) match their dist/ canonical"),
        r.out.trim().split("\n").find((l) => l.includes("packaged unit")) ?? r.out.trim().split("\n")[0]);
}

{
  // A comment reword must NOT fail: the whole reason this compares directives is
  // that prose in these files is where the reasoning lives and it changes often.
  const r = gateOver(UNIT(), `# a reworded note about why\n${UNIT()}`);
  check("a comment reword passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  // The defect: a hardening directive in the canonical that the image's copy
  // does not carry, so the deployed unit is not the reviewed one.
  const r = gateOver(UNIT("NoNewPrivileges=yes\n"), UNIT());
  check("a directive present in one and not the other is caught", r.code === 1,
        r.out.trim().split("\n").slice(-1)[0]);
  check("and the finding names the unit", r.out.includes("arlen-probe.service"));
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate sees a drifted directive and ignores a reworded comment");
