// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-log-filters: plant both defects it exists for -
// the blanket that swept zbus payloads into the journal, and the bare init that
// left four apps mute - and watch it refuse each.

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-log-filters.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function tree(apps) {
  const dir = mkdtempSync(join(tmpdir(), "log-filters-"));
  for (const [app, body] of Object.entries(apps)) {
    const src = join(dir, "apps", app, "src-tauri", "src");
    mkdirSync(src, { recursive: true });
    writeFileSync(join(src, "lib.rs"), body);
  }
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const GOOD = 'fn run() { env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,arlen_x_lib=info")).init(); }\n';
const MUTE = "fn run() { env_logger::init(); }\n";
const BLANKET = 'fn run() { env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init(); }\n';
const BARE = "fn run() { env_logger::init(); }\n";
const NO_LOGGING = "fn run() { println!(\"hi\"); }\n";

{
  const d = tree({ good: GOOD });
  check("a filter naming its own crate passes", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// The privacy defect: a level for every crate, which is how zbus frames got in.
{
  const d = tree({ blanket: BLANKET });
  const r = run(d);
  check("a blanket level is refused", r.code === 1);
  check("and the message names the dependency sweep", r.out.includes("EVERY crate"));
  rmSync(d, { recursive: true, force: true });
}

// The other direction: the app cannot be heard at all.
{
  const d = tree({ mute: BARE });
  const r = run(d);
  check("a bare env_logger::init() is refused", r.code === 1);
  check("and the message says the app is mute", r.out.includes("mute"));
  rmSync(d, { recursive: true, force: true });
}

// The gate's own advice is usually written above the call, quoting the bad form.
// Reading that back as a defect is how it first failed every app I had fixed.
{
  const d = tree({
    documented: "// A bare `env_logger::init()` defaults to `error`, so we do not use it.\n" + GOOD,
  });
  check("the bad form quoted in a comment is not a defect", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// An app that does no logging at all is not a subject.
{
  const d = tree({ quiet: NO_LOGGING });
  check("an app with no logging is not a subject", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// Daemons, which this check could not see until 18 August. The last component in
// the tree still calling a bare `env_logger::init()` was the eBPF sensor, and it
// sat outside the window: its journal carried four systemd lines and nothing of
// its own, so which of its tracepoints attached was unreadable on every boot. A
// daemon written that way tomorrow has to be caught here, not by someone reading
// an empty journal in three months.
{
  const dir = mkdtempSync(join(tmpdir(), "log-filters-daemon-"));
  // Both layouts a daemon uses: src straight under it, and a crate one level down
  // (`kernel-layer/kernel-layer/src`), which is the shape the real offender had.
  const flat = join(dir, "daemons", "flatd", "src");
  const nested = join(dir, "daemons", "nestd", "nestd", "src");
  mkdirSync(flat, { recursive: true });
  mkdirSync(nested, { recursive: true });
  mkdirSync(join(dir, "apps", "keep", "src-tauri", "src"), { recursive: true });
  writeFileSync(join(dir, "apps", "keep", "src-tauri", "src", "lib.rs"), GOOD);
  writeFileSync(join(flat, "main.rs"), GOOD);
  writeFileSync(join(nested, "main.rs"), MUTE);
  const r = run(dir);
  check("a daemon crate one level down is read, not skipped", r.code !== 0 && r.out.includes("nestd"));
  check("and a daemon with a sound filter is not flagged", !r.out.includes("flatd"));
  rmSync(dir, { recursive: true, force: true });
}

// The `tracing` spelling of the same blanket, in a component under `daemons/`.
// The rule knew only `env_logger` until 22 August, and 24 components carried this
// one where nothing could see it. Named `planted` so no queue entry excuses it.
{
  const d = mkdtempSync(join(tmpdir(), "log-filters-tracing-"));
  const app = join(d, "apps", "one", "src-tauri", "src");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "lib.rs"), GOOD);
  const dmn = join(d, "daemons", "planted", "src");
  mkdirSync(dmn, { recursive: true });
  writeFileSync(
    join(dmn, "main.rs"),
    'fn main() { tracing_subscriber::EnvFilter::new("info"); }\n',
  );
  const r = run(d);
  check("a bare tracing EnvFilter level is caught", r.code === 1 && r.out.includes("planted"));
  rmSync(d, { recursive: true, force: true });
}

// A frontend under `daemons/` is its own component, not part of its parent. The
// picker's Rust lives in `src-tauri/src` beside the portal daemon's `src`, and
// merged into one entry its filter was excused by the queue entry for the daemon.
{
  const d = mkdtempSync(join(tmpdir(), "log-filters-frontend-"));
  const app = join(d, "apps", "one", "src-tauri", "src");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "lib.rs"), GOOD);
  const front = join(d, "daemons", "some-portal", "its-ui", "src-tauri", "src");
  mkdirSync(front, { recursive: true });
  writeFileSync(
    join(front, "lib.rs"),
    'fn run() { tracing_subscriber::EnvFilter::new("info"); }\n',
  );
  const r = run(d);
  check(
    "a daemon's frontend is read, and under its own name",
    r.code === 1 && r.out.includes("its-ui"),
  );
  rmSync(d, { recursive: true, force: true });
}

// Pointed somewhere with no apps, "nothing wrong" would describe a scan that
// read nothing.
{
  const d = mkdtempSync(join(tmpdir(), "log-filters-empty-"));
  check("a tree with no apps is an error, not a pass", run(d).code === 2);
  rmSync(d, { recursive: true, force: true });
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
