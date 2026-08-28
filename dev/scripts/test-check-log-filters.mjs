// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-log-filters: plant both defects it exists for -
// the blanket that swept zbus payloads into the journal, and the bare init that
// left four apps mute - and watch it refuse each.

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-log-filters.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function tree(apps) {
  const dir = mint("log-filters-");
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
  cleanup(d);
}

// The privacy defect: a level for every crate, which is how zbus frames got in.
{
  const d = tree({ blanket: BLANKET });
  const r = run(d);
  check("a blanket level is refused", r.code === 1);
  check("and the message names the dependency sweep", r.out.includes("EVERY crate"));
  cleanup(d);
}

// The other direction: the app cannot be heard at all.
{
  const d = tree({ mute: BARE });
  const r = run(d);
  check("a bare env_logger::init() is refused", r.code === 1);
  check("and the message says the app is mute", r.out.includes("mute"));
  cleanup(d);
}

// The gate's own advice is usually written above the call, quoting the bad form.
// Reading that back as a defect is how it first failed every app I had fixed.
{
  const d = tree({
    documented: "// A bare `env_logger::init()` defaults to `error`, so we do not use it.\n" + GOOD,
  });
  check("the bad form quoted in a comment is not a defect", run(d).code === 0);
  cleanup(d);
}

// An app that does no logging at all is not a subject.
{
  const d = tree({ quiet: NO_LOGGING });
  check("an app with no logging is not a subject", run(d).code === 0);
  cleanup(d);
}

// Daemons, which this check could not see until 18 August. The last component in
// the tree still calling a bare `env_logger::init()` was the eBPF sensor, and it
// sat outside the window: its journal carried four systemd lines and nothing of
// its own, so which of its tracepoints attached was unreadable on every boot. A
// daemon written that way tomorrow has to be caught here, not by someone reading
// an empty journal in three months.
{
  const dir = mint("log-filters-daemon-");
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
  cleanup(dir);
}

// The `tracing` spelling of the same blanket, in a component under `daemons/`.
// The rule knew only `env_logger` until 22 August, and 24 components carried this
// one where nothing could see it. Named `planted` so no queue entry excuses it.
{
  const d = mint("log-filters-tracing-");
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
  cleanup(d);
}

// A frontend under `daemons/` is its own component, not part of its parent. The
// picker's Rust lives in `src-tauri/src` beside the portal daemon's `src`, and
// merged into one entry its filter was excused by the queue entry for the daemon.
{
  const d = mint("log-filters-frontend-");
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
  cleanup(d);
}

// A named filter has to name crates that exist. The tree's manifests sit in two
// places - an app keeps its under `src-tauri/`, a daemon at its root - and
// reading only the root made this rule skip all nineteen frontends in silence.
{
  const d = mint("log-filters-names-");
  const src = join(d, "apps", "one", "src-tauri", "src");
  mkdirSync(src, { recursive: true });
  writeFileSync(
    join(src, "lib.rs"),
    'fn run() { env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,not_a_crate=info")).init(); }\n',
  );
  writeFileSync(
    join(d, "apps", "one", "src-tauri", "Cargo.toml"),
    '[package]\nname = "one-app"\n\n[lib]\nname = "one_lib"\n',
  );
  const r = run(d);
  check(
    "a filter naming a crate that does not exist is caught",
    r.code === 1 && r.out.includes("not_a_crate"),
  );
  cleanup(d);
}

// The trap the daemon queue is deferred on: two crates speak, the filter names
// one, and the other half is mute with no symptom but a quiet journal.
{
  const d = mint("log-filters-half-");
  const src = join(d, "daemons", "two", "src");
  mkdirSync(src, { recursive: true });
  writeFileSync(
    join(src, "main.rs"),
    'fn main() { tracing_subscriber::EnvFilter::new("warn,two_bin=info"); tracing::info!("from the binary"); }\n',
  );
  writeFileSync(join(src, "lib.rs"), "pub mod work;\n");
  writeFileSync(join(src, "work.rs"), 'pub fn go() { tracing::info!("from the library"); }\n');
  writeFileSync(
    join(d, "daemons", "two", "Cargo.toml"),
    '[package]\nname = "two-lib"\n\n[[bin]]\nname = "two-bin"\n',
  );
  const app = join(d, "apps", "one", "src-tauri", "src");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "lib.rs"), GOOD);
  const r = run(d);
  check(
    "a filter that leaves a speaking crate unnamed is caught",
    r.code === 1 && r.out.includes("the library"),
  );
  cleanup(d);
}

// A package's extra binaries live in `src/bin/` and are their own crates. The
// knowledge daemon's timeline helper kept a blanket level through the whole pass
// that fixed twenty-four others, because nothing here looked in that directory.
{
  const d = mint("log-filters-bin-");
  const app = join(d, "apps", "one", "src-tauri", "src");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "lib.rs"), GOOD);
  const bin = join(d, "daemons", "three", "src", "bin");
  mkdirSync(bin, { recursive: true });
  writeFileSync(join(d, "daemons", "three", "src", "main.rs"), "fn main() {}\n");
  writeFileSync(
    join(bin, "helper.rs"),
    'fn main() { tracing_subscriber::EnvFilter::new("info"); }\n',
  );
  const r = run(d);
  check(
    "an extra binary under src/bin is read, under its own name",
    r.code === 1 && r.out.includes("helper"),
  );
  cleanup(d);
}

// Pointed somewhere with no apps, "nothing wrong" would describe a scan that
// read nothing.
{
  const d = mint("log-filters-empty-");
  check("a tree with no apps is an error, not a pass", run(d).code === 2);
  cleanup(d);
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
