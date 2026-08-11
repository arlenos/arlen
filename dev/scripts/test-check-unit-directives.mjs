// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A directive systemd does not recognise is dropped with a log line, so the unit
// ships looking hardened and runs without it. This check exists for that, and
// these cases are what make its silence mean something.
//
// The typo case is the whole point: `ProtectSytem` differs from `ProtectSystem`
// by one letter in the middle of a word, `check-packaged-units.sh` compares the
// two copies of a unit and passes when both carry it, and reading the line is how
// it got there. Only systemd's own parser has an opinion.
//
// Run: node dev/scripts/test-check-unit-directives.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-unit-directives.py");

if (!spawnSync("which", ["systemd-analyze"], { encoding: "utf8" }).stdout.trim()) {
  console.log("check-unit-directives: systemd-analyze absent, cases not run");
  process.exit(0);
}

const failures = [];

const UNIT = (extra) => `[Unit]
Description=A probe daemon

[Service]
Type=simple
ExecStart=/usr/lib/arlen/libexec/arlen-probe
${extra}

[Install]
WantedBy=default.target
`;

function check(name, body, expect, tree = "user") {
  const dir = mkdtempSync(join(tmpdir(), "arlen-unitdir-"));
  const d = join(dir, `dev/mkosi/mkosi.extra/usr/lib/systemd/${tree}`);
  mkdirSync(d, { recursive: true });
  writeFileSync(join(d, "arlen-probe.service"), body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-unit-directives:");

check(
  "a unit whose directives systemd knows passes",
  UNIT("ProtectSystem=strict\nPrivateTmp=yes"),
  (code) => code === 0,
);

// The defect: accepted by the file, ignored by systemd, invisible to a diff of
// two copies that both have it.
check(
  "a mistyped hardening directive is caught",
  UNIT("ProtectSytem=strict"),
  (code, out) => code === 1 && out.includes("ProtectSytem"),
);

check(
  "a value systemd cannot parse is caught",
  UNIT("PrivateTmp=maybe"),
  (code) => code === 1,
);

// The image ships two unit trees and this check read one of them for its first
// hours, which is the same hole the peer-identity gate turned out to have on the
// same day. A mistyped hardening key in a system unit is exactly as silent as in a
// user unit, and there are five of them.
check(
  "a mistyped directive in a SYSTEM unit is caught too",
  UNIT("ProtectSytem=strict"),
  (code, out) => code === 1 && out.includes("ProtectSytem"),
  "system",
);

// The missing binary is expected wherever this runs - the units point into the
// image. `check-shipped-units.py` is what holds ExecStart honest.
check(
  "a missing ExecStart binary is not what this check is about",
  UNIT("ProtectSystem=strict"),
  (code, out) => code === 0 && !out.includes("not executable"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all unit-directive cases passed");
