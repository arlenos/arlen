#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-daemon-stop.py. The repository passes because ten daemons are on the
// baseline, so the interesting question is whether the check can fail AT ALL - and whether the
// exclusions (oneshot, D-Bus activation) hold, since a check that quietly excuses everything
// reads exactly like a clean board.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-daemon-stop.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

const LONG_RUNNING = `[Unit]
Description=A daemon

[Service]
Type=simple
ExecStart=/usr/lib/arlen/libexec/arlen-demo

[Install]
WantedBy=default.target
`;

const ONESHOT = LONG_RUNNING.replace("Type=simple", "Type=oneshot");

// A D-Bus activation file: no [Service] ExecStart in the systemd sense.
const DBUS_ACTIVATION = `[D-BUS Service]
Name=org.arlen.Demo1
Exec=/usr/lib/arlen/libexec/arlen-demo
SystemdService=arlen-demo.service
`;

const WITH_HANDLER = `fn main() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
}
`;
const WITHOUT_HANDLER = `fn main() {
    tokio::signal::ctrl_c();
}
`;

function tree({ unit, unitName = "arlen-demo.service", source }) {
  const root = mint("daemon-stop-");
  mkdirSync(join(root, "daemons/demo/dist"), { recursive: true });
  mkdirSync(join(root, "daemons/demo/src"), { recursive: true });
  writeFileSync(join(root, "daemons/demo/dist", unitName), unit);
  writeFileSync(join(root, "daemons/demo/src/main.rs"), source);
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

{
  const root = tree({ unit: LONG_RUNNING, source: WITHOUT_HANDLER });
  const rc = run(root);
  rc === 1
    ? ok("a long-running unit with no SIGTERM handler is caught")
    : bad("a long-running unit with no SIGTERM handler is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree({ unit: LONG_RUNNING, source: WITH_HANDLER });
  const rc = run(root);
  rc === 0
    ? ok("handling SIGTERM passes")
    : bad("handling SIGTERM passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A oneshot is SUPPOSED to run and exit; demanding a handler would push noise
  // into units that are correct. Paired with a well-behaved long-running daemon
  // so the run has something real to judge - on its own it would trip the
  // "no long-running unit found" guard and pass this case for the wrong reason.
  const root = tree({ unit: LONG_RUNNING, source: WITH_HANDLER });
  mkdirSync(join(root, "daemons/batch/dist"), { recursive: true });
  mkdirSync(join(root, "daemons/batch/src"), { recursive: true });
  writeFileSync(join(root, "daemons/batch/dist/arlen-batch.service"), ONESHOT);
  writeFileSync(join(root, "daemons/batch/src/main.rs"), WITHOUT_HANDLER);
  const rc = run(root);
  rc === 0
    ? ok("a oneshot is not demanded")
    : bad("a oneshot is not demanded", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A D-Bus activation file names a bus, not a service lifetime.
  const root = tree({
    unit: DBUS_ACTIVATION,
    unitName: "org.arlen.Demo1.service",
    source: WITHOUT_HANDLER,
  });
  const rc = run(root);
  // With only an activation file there is no long-running unit at all, so the
  // check has nothing plausible to look at and says so rather than passing.
  rc === 1
    ? ok("an activation file alone is not treated as a service")
    : bad("an activation file alone is not treated as a service", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const rc = run(join(here, "..", ".."));
  rc === 0
    ? ok("the repository itself passes, on its baseline")
    : bad("the repository itself passes", `expected 0, got ${rc}`);
}

if (failures) {
  console.log(`\n${failures} case(s) failed`);
  process.exit(1);
}
console.log("a service that never sees a stop fails the check");
