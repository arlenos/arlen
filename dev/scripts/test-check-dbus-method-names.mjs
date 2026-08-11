// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The seam this guards has no compiler across it, so the cases are about what
// counts as a mismatch: zbus renames snake to Pascal, foreign interfaces are not
// ours to check, and a file talking to several of our interfaces cannot be
// resolved per call.
//
// Run: node dev/scripts/test-check-dbus-method-names.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dbus-method-names.py");
const failures = [];

const IFACE = `#[zbus::interface(name = "org.arlen.Probe1")]
impl P {
    async fn set_alarm(&self, id: String) {}
    #[zbus(property)]
    async fn ticking(&self) -> bool { true }
}
`;

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-dbusname-"));
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

console.log("check-dbus-method-names:");

check(
  "a call matching the renamed method passes",
  {
    "daemons/probe/src/iface.rs": IFACE,
    "apps/probe/src/lib.rs":
      'const I: &str = "org.arlen.Probe1";\nproxy.call("SetAlarm", &(id,)).await;\n',
  },
  (code) => code === 0,
);

check(
  "a call naming a method that does not exist is caught",
  {
    "daemons/probe/src/iface.rs": IFACE,
    "apps/probe/src/lib.rs":
      'const I: &str = "org.arlen.Probe1";\nproxy.call("TakeResult", &()).await;\n',
  },
  (code, out) => code === 1 && out.includes("TakeResult"),
);

// The shape the harness is in: the old name survives the handover on the client.
check(
  "a lowercase call against a Pascal-renamed method is caught",
  {
    "daemons/probe/src/iface.rs": IFACE,
    "apps/probe/src/lib.rs":
      'const I: &str = "org.arlen.Probe1";\nproxy.call("set_alarm", &()).await;\n',
  },
  (code, out) => code === 1 && out.includes("set_alarm"),
);

// A property is read with Get, never called, so it must not count as a method.
check(
  "a property is not offered as a callable method",
  {
    "daemons/probe/src/iface.rs": IFACE,
    "apps/probe/src/lib.rs":
      'const I: &str = "org.arlen.Probe1";\nproxy.call("Ticking", &()).await;\n',
  },
  (code, out) => code === 1 && out.includes("Ticking"),
);

// BlueZ, UPower and logind define their own methods; reporting those would be
// reporting that a foreign API exists.
check(
  "a call on a foreign interface is not ours to check",
  {
    "daemons/probe/src/iface.rs": IFACE,
    "apps/probe/src/lib.rs":
      'const I: &str = "org.bluez.Adapter1";\nproxy.call("StartDiscovery", &()).await;\n',
  },
  (code) => code === 0,
);

check(
  "a file naming two of our interfaces is reported unchecked, not guessed",
  {
    "daemons/probe/src/iface.rs":
      IFACE + '\n#[zbus::interface(name = "org.arlen.Other1")]\nimpl Q { async fn ping_it(&self) {} }\n',
    "apps/probe/src/lib.rs":
      'const A: &str = "org.arlen.Probe1";\nconst B: &str = "org.arlen.Other1";\nproxy.call("Nonsense", &()).await;\n',
  },
  (code, out) => code === 0 && out.includes("more than one interface"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all dbus method-name cases passed");
