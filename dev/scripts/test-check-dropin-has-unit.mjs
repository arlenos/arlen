// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the drop-in gate see a drop-in directory that names no unit?
//
// The fault it was written for is the first case: after the event bus was split
// into a system unit and a user unit, the drop-in that switches enforcement on
// kept the old name in the system scope, matched neither, and was applied to
// nothing for three months.
//
// Run: node dev/scripts/test-check-dropin-has-unit.mjs

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-dropin-has-unit.py");
const SD = "dev/mkosi/mkosi.extra/usr/lib/systemd";
const UNIT = "[Service]\nExecStart=/usr/bin/event-bus\n";
const DROPIN = "[Service]\nEnvironment=ARLEN_EVENT_BUS_ENFORCE=1\n";

let failed = 0;
function check(name, files, expect) {
  const dir = mint("arlen-dropin-");
  try {
    for (const [rel, body] of Object.entries(files)) {
      mkdirSync(path.join(dir, path.dirname(rel)), { recursive: true });
      writeFileSync(path.join(dir, rel), body, "utf8");
    }
    const r = spawnSync("python3", [gate, dir], { encoding: "utf8" });
    const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
    const ok = expect(r.status ?? 1, out);
    console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
    if (!ok) { console.log(`       exit ${r.status}\n${out}`); failed++; }
  } finally {
    cleanup(dir);
  }
}

console.log("check-dropin-has-unit:");

check(
  "a drop-in named after a unit that only exists in the other scope is named",
  {
    [`${SD}/system/arlen-event-bus.service.d/10-enforce.conf`]: DROPIN,
    [`${SD}/system/arlen-event-bus-system.service`]: UNIT,
    [`${SD}/user/arlen-event-bus.service`]: UNIT,
  },
  (code, out) => code === 1 && out.includes("arlen-event-bus.service.d") && out.includes("user scope"),
);

check(
  "a drop-in beside its unit passes",
  {
    [`${SD}/system/arlen-event-bus-system.service.d/10-enforce.conf`]: DROPIN,
    [`${SD}/system/arlen-event-bus-system.service`]: UNIT,
  },
  (code) => code === 0,
);

check(
  "a unit a build phase installs counts as there",
  {
    [`${SD}/user/arlen-demod.service.d/10-x.conf`]: DROPIN,
    "dev/mkosi/mkosi.build.d/08-demo.sh.chroot":
      'install -m644 demo.service "$DESTDIR/usr/lib/systemd/user/arlen-demod.service"\n',
  },
  (code) => code === 0,
);

check(
  "a build phase installing it in the other scope does not count",
  {
    [`${SD}/system/arlen-demod.service.d/10-x.conf`]: DROPIN,
    "dev/mkosi/mkosi.build.d/08-demo.sh.chroot":
      'install -m644 demo.service "$DESTDIR/usr/lib/systemd/user/arlen-demod.service"\n',
  },
  (code, out) => code === 1 && out.includes("arlen-demod.service.d"),
);

check(
  "the distribution's unit and a daemon's conf.d are left alone",
  {
    [`${SD}/system/greetd.service.d/10-x.conf`]: DROPIN,
    "dev/mkosi/mkosi.extra/etc/systemd/logind.conf.d/10-x.conf": "[Login]\n",
  },
  (code) => code === 0,
);

check(
  "a tree with no drop-in at all is a scan pointed wrong, not a pass",
  { [`${SD}/system/arlen-event-bus-system.service`]: UNIT },
  (code) => code === 2,
);

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log("all drop-in cases passed");
