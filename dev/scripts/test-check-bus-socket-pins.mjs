// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-bus-socket-pins. Two of its cases are the check's
// OWN first-draft bugs, kept as fixtures: it demanded a consumer pin from a pure
// producer, and treated a struct field named `event_bus` as bus usage. A gate that
// over-reports gets ignored, so both directions are pinned down here.

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const GATE = join(dirname(fileURLToPath(import.meta.url)), "check-bus-socket-pins.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// A tree with one user unit and one crate that builds its binary.
function tree({ unitEnv = [], source }) {
  const dir = mkdtempSync(join(tmpdir(), "bus-pins-"));
  const units = join(dir, "dev/mkosi/mkosi.extra/usr/lib/systemd/user");
  mkdirSync(units, { recursive: true });
  writeFileSync(
    join(units, "arlen-thing.service"),
    "[Service]\n" +
      unitEnv.map((e) => `Environment=${e}\n`).join("") +
      "ExecStart=/usr/lib/arlen/libexec/arlen-thing\n"
  );
  const crate = join(dir, "daemons/thing");
  mkdirSync(join(crate, "src"), { recursive: true });
  writeFileSync(join(crate, "Cargo.toml"), '[package]\nname = "arlen-thing"\n');
  writeFileSync(join(crate, "src/main.rs"), source);
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const PRODUCER = "use os_sdk::UnixEventEmitter;\nfn f() { let _ = UnixEventEmitter::new(p); }\n";
const CONSUMER = 'fn f() { let s = consumer_socket(); }\n';
const NEITHER = "struct S { event_bus: u8 }\nfn f() {}\n";

// The defect: a daemon that publishes, on a user unit, with no pin.
{
  const d = tree({ source: PRODUCER });
  const r = run(d);
  check("a publisher with no producer pin is caught", r.code === 1);
  check("and the message names the missing pin", r.out.includes("ARLEN_PRODUCER_SOCKET"));
  rmSync(d, { recursive: true, force: true });
}

// Pinned, so it passes.
{
  const d = tree({
    source: PRODUCER,
    unitEnv: ["ARLEN_PRODUCER_SOCKET=/run/arlen/event-bus-producer.sock"],
  });
  check("a pinned publisher passes", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// The check's own first bug: a PURE PRODUCER does not need a consumer pin, and
// demanding one teaches people to add a line that means nothing.
{
  const d = tree({
    source: PRODUCER,
    unitEnv: ["ARLEN_PRODUCER_SOCKET=/run/arlen/event-bus-producer.sock"],
  });
  check("a pure producer is not asked for a consumer pin", !run(d).out.includes("CONSUMER"));
  rmSync(d, { recursive: true, force: true });
}

// A subscriber needs the other direction, and only that one.
{
  const d = tree({ source: CONSUMER });
  const r = run(d);
  check("a subscriber with no consumer pin is caught", r.code === 1);
  check("and is not asked for a producer pin", !r.out.includes("PRODUCER"));
  rmSync(d, { recursive: true, force: true });
}

// The check's second bug: a struct field named `event_bus` is not bus usage.
{
  const d = tree({ source: NEITHER });
  check("a crate that only names `event_bus` is not a subject", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// A unit whose binary no crate builds means this check silently skipped a unit,
// which is the shape it exists to refuse.
{
  const dir = mkdtempSync(join(tmpdir(), "bus-pins-orphan-"));
  const units = join(dir, "dev/mkosi/mkosi.extra/usr/lib/systemd/user");
  mkdirSync(units, { recursive: true });
  writeFileSync(join(units, "x.service"), "[Service]\nExecStart=/usr/bin/arlen-ghost\n");
  const r = run(dir);
  check("a unit whose binary no crate builds is refused", r.code === 1);
  rmSync(dir, { recursive: true, force: true });
}

// No units at all is a scan that read nothing.
{
  const dir = mkdtempSync(join(tmpdir(), "bus-pins-empty-"));
  check("a tree with no user units is an error, not a pass", run(dir).code === 2);
  rmSync(dir, { recursive: true, force: true });
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
