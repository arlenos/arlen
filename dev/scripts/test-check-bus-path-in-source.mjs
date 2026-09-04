// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the hardcoded-bus-path check.
//
// The red case is what shipped: a `const` naming the bus's old system path,
// invisible to every unit-level check because it is a string in the source. Eight
// of them survived removing eighteen `Environment=` pins, and the boot found them
// rather than any gate.
//
// The green cases are the two shapes that must stay allowed, because banning them
// would push people back to writing the path out: a comment explaining the
// history, and a test asserting that the pinned branch still resolves.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-bus-path-in-source.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(body, rel = "apps/thing/src/lib.rs") {
  const dir = mint("buspath-");
  const p = join(dir, rel);
  mkdirSync(dirname(p), { recursive: true });
  writeFileSync(p, body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("bus path in source:");

{
  const r = run('const S: &str = "/run/arlen/event-bus-consumer.sock";\n');
  check("a hardcoded consumer path is caught", r.code === 1);
  check("and the message points at the SDK", r.out.includes("socket_path"));
}
{
  const r = run('fn f() { let _ = Path::new("/run/arlen/event-bus-producer.sock"); }\n');
  check("the producer path is caught too", r.code === 1);
}
{
  const r = run(
    'fn f() { os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock"); }\n'
  );
  check("asking the SDK passes", r.code === 0);
}
{
  // The explanation of the defect must survive the ban on the defect.
  const r = run('// This said "/run/arlen/event-bus-consumer.sock" until 15 Aug.\nfn f() {}\n');
  check("a comment naming the old path is not the defect", r.code === 0);
}
{
  // A test that asserts the pinned branch resolves to the pin is checking the
  // resolver, not hardcoding a destination.
  const r = run(
    'fn f() {}\n#[cfg(test)]\nmod t {\n  #[test]\n  fn p() {\n' +
      '    assert_eq!(r(Some("/run/arlen/event-bus-producer.sock")), "/run/arlen/event-bus-producer.sock");\n' +
      "  }\n}\n"
  );
  check("a test asserting the pinned branch is allowed", r.code === 0);
}
{
  const r = run("fn f() {}\n", "README.md");
  check("a tree with no Rust source refuses rather than passing", r.code === 2);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
