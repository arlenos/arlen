#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-unit-copies.py`: the same unit in two places must act the
// same, may explain itself differently, and a settled entry must leave the list.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-unit-copies.py");
const REPO = join(HERE, "..", "..");

function tree({ shipped, crate }) {
  const root = mint("unit-copies-");
  const write = (rel, body) => {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  };
  write("dev/mkosi/mkosi.extra/usr/lib/systemd/user/demo.service", shipped);
  if (crate !== undefined) write("daemons/demo/dist/demo.service", crate);
  return root;
}

/// The activation-file half: same shape, a different directory, and the one whose
/// missing `SystemdService=` line once meant a daemon started outside its unit.
function busTree({ shipped, crate }) {
  const root = mint("unit-copies-bus-");
  const write = (rel, body) => {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  };
  // A shipped unit as well, so the run has something to read either way.
  write("dev/mkosi/mkosi.extra/usr/lib/systemd/user/demo.service", UNIT);
  write("dev/mkosi/mkosi.extra/usr/share/dbus-1/services/org.demo.Thing.service", shipped);
  write("daemons/demo/dist/org.demo.Thing.service", crate);
  return root;
}

function gateOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const UNIT = "[Service]\nExecStart=/usr/lib/arlen/libexec/demo\nProtectHome=yes\n";

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "two copies with the same directives pass",
    () => tree({ shipped: UNIT, crate: UNIT }),
    (code) => code === 0,
    true,
  ],
  [
    "a directive only one copy carries is caught, and named",
    () =>
      tree({
        shipped: UNIT,
        crate: "[Service]\nExecStart=/usr/lib/arlen/libexec/demo\n",
      }),
    (code, out) => code === 1 && out.includes("ProtectHome=yes"),
    true,
  ],
  [
    "a different explanation of the same directives is not a finding",
    () =>
      tree({
        shipped: `# it runs as a user service\n${UNIT}`,
        crate: `# next to the code that explains it\n${UNIT}`,
      }),
    (code) => code === 0,
    true,
  ],
  [
    "a unit the image ships and no crate keeps is not compared",
    () => tree({ shipped: UNIT }),
    (code) => code === 0,
    true,
  ],
  [
    "an activation file that drops its SystemdService line is caught",
    () =>
      busTree({
        shipped:
          "[D-BUS Service]\nName=org.demo.Thing\nExec=/usr/lib/arlen/libexec/demo\nSystemdService=demo.service\n",
        crate: "[D-BUS Service]\nName=org.demo.Thing\nExec=/usr/lib/arlen/libexec/demo\n",
      }),
    (code, out) => code === 1 && out.includes("SystemdService=demo.service"),
    true,
  ],
  [
    "no shipped unit at all refuses rather than passing with nothing read",
    () => mint("unit-copies-empty-"),
    (code, out) => code !== 0 && (out.includes("NOTHING WAS READ") || out.includes("missing")),
    true,
  ],
];

let failed = 0;
for (const [name, build, expect, disposable] of cases) {
  const root = build();
  const { code, out } = gateOn(root);
  if (disposable) cleanup(root);
  const ok = expect(code, out);
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed += 1;
    console.log(`     exit ${code}\n     ${out.trim().split("\n").slice(0, 3).join("\n     ")}`);
  }
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log(`\nall ${cases.length} cases behaved`);
