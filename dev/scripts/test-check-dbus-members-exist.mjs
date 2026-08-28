// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-dbus-members-exist.
//
// The first case is the defect it was written from, reduced: a Tauri command
// that exists, compiles and is registered, dialling a D-Bus member no interface
// serves. `check-invoke-exists` passes it, because the command is there.
//
// Run: node dev/scripts/test-check-dbus-members-exist.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-dbus-members-exist.py");
const failures = [];

function run(name, files, expect) {
  const dir = mint("arlen-dbus-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  cleanup(dir);
}

const IFACE = `
#[zbus::interface(name = "org.arlen.Thing1")]
impl Thing {
    #[zbus(name = "working_set")]
    async fn working_set(&self) -> String { String::new() }
}
`;

run(
  "a member nobody serves is caught",
  {
    "daemons/thing/src/iface.rs": IFACE,
    "apps/demo/src-tauri/src/lib.rs":
      'async fn go() { let _ = proxy.call::<_, _, String>("absent_member", &()).await; }\n',
  },
  (code, out) => code === 1 && out.includes("absent_member"),
);

run(
  "a member the tree serves passes",
  {
    "daemons/thing/src/iface.rs": IFACE,
    "apps/demo/src-tauri/src/lib.rs":
      'async fn go() { let _ = proxy.call::<_, _, String>("working_set", &()).await; }\n',
  },
  (code) => code === 0,
);

// BlueZ, NetworkManager, logind and the tray all name members in PascalCase, and
// we cannot check a service we do not own. The case of the name is the whole
// rule, so it is worth a control in both directions.
run(
  "a foreign PascalCase member is out of scope",
  {
    "daemons/thing/src/iface.rs": IFACE,
    "apps/demo/src-tauri/src/lib.rs":
      'async fn go() { let _ = proxy.call_method("StartDiscovery", &()).await; }\n',
  },
  (code) => code === 0,
);

// The member passed to a helper rather than dialled in place. This is how the
// defect this was written from actually reaches its proxy, and the first cut of
// the regex missed exactly this shape.
run(
  "a member passed through a helper is read too",
  {
    "daemons/thing/src/iface.rs": IFACE,
    "apps/demo/src-tauri/src/lib.rs":
      'async fn go() { let _ = try_call_string(BUS, PATH, "ghost_member").await; }\n',
  },
  (code, out) => code === 1 && out.includes("ghost_member"),
);

// A local helper named `call` is not a proxy. code-indexer's tests define one,
// and eight of its fixture names read as dialled members until the rule asked
// for a method call or a `*call_string`.
run(
  "a local helper named call is not a proxy",
  {
    "daemons/thing/src/iface.rs": IFACE,
    "daemons/thing/src/other.rs":
      "#[cfg(test)]\nmod tests {\n  fn call(name: &str, line: u32) -> Ref { todo!() }\n"
      + '  #[test]\n  fn t() { let _ = call("bar", 1); }\n}\n',
    // One real call so the tree is not empty; the refusal case is its own control.
    "apps/demo/src-tauri/src/lib.rs":
      'async fn go() { let _ = proxy.call::<_, _, String>("working_set", &()).await; }\n',
  },
  (code) => code === 0,
);

run(
  "a tree with no member calls at all refuses rather than passing",
  { "README.md": "nothing here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

for (const f of failures) console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
if (failures.length) process.exit(1);
console.log("a dialled member must be one this tree serves, and the list only shrinks");
