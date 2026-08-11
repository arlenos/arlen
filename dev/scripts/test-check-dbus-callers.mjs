// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A positive control for an AUTHORISATION gate, which is why it is worth its own
// commit rather than the interleave: this check is the reason we can say every
// D-Bus method can see who called it. If it were blind, that sentence would be a
// security claim resting on nothing, and every clear it has ever given would
// un-clear at once.
//
// It had no such test for a mechanical reason - its scan root was hardcoded, so
// nobody could hand it a method that fails. The root is an argument now.
//
// Run: node dev/scripts/test-check-dbus-callers.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dbus-callers.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-dbusc-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // The gate enumerates candidates with `git grep`, which needs a repository -
  // so the fixture is one. Keeping the gate on git grep rather than a filesystem
  // walk is deliberate: it is what makes the real run skip `target/` and every
  // vendored tree for free, and a positive control should exercise the gate as it
  // actually runs.
  spawnSync("git", ["init", "-q"], { cwd: dir });
  spawnSync("git", ["add", "-A"], { cwd: dir });
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-dbus-callers:");

check(
  "a method that cannot see its caller is caught",
  {
    "daemons/probe/src/iface.rs":
      '#[zbus::interface(name = "org.arlen.Probe1")]\n' +
      "impl Probe {\n" +
      "    async fn delete_everything(&self) -> zbus::fdo::Result<()> {\n" +
      "        Ok(())\n" +
      "    }\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("delete_everything"),
);

check(
  "the same method taking the header passes",
  {
    "daemons/probe/src/iface.rs":
      '#[zbus::interface(name = "org.arlen.Probe1")]\n' +
      "impl Probe {\n" +
      "    async fn delete_everything(\n" +
      "        &self,\n" +
      "        #[zbus(header)] header: zbus::message::Header<'_>,\n" +
      "    ) -> zbus::fdo::Result<()> {\n" +
      "        Ok(())\n" +
      "    }\n" +
      "}\n",
  },
  (code) => code === 0,
);

// I first wrote this case expecting a `#[zbus(property)]` to be excused
// automatically, and the gate disagreed - correctly. Its rule is NOT "always take
// the header"; it is that a header-less method must be LISTED with the reason it
// is safe. A property is not automatically safe: read-only says nothing about
// whether what it returns is scoped to the user, and the two real leaks this
// check was written after were both read-only answers. So a property is caught
// like anything else until someone writes down why it need not know its caller.
check(
  "a property is not automatically excused either",
  {
    "daemons/probe/src/iface.rs":
      '#[zbus::interface(name = "org.arlen.Probe1")]\n' +
      "impl Probe {\n" +
      "    #[zbus(property)]\n" +
      "    async fn my_recent_files(&self) -> Vec<String> {\n" +
      "        vec![]\n" +
      "    }\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("my_recent_files"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("the blind method is caught and the attested one passes");
