// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A Tauri setup hook runs before the async runtime exists, so anything reaching
// for tokio there panics the main thread at startup. This check has to find that
// reach wherever it is written.
//
// The first case is why this file exists. The gate extracted the setup body,
// collected the function NAMES called from it, and scanned those functions - never
// the body itself. So `tokio::spawn` written straight into the closure, which
// needs no helper and is the shortest way to write the defect, was the one form it
// could not see. Found on 11 August by putting one there and watching it pass.
//
// Run: node dev/scripts/test-check-setup-runtime.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-setup-runtime.py");

const failures = [];

function check(name, lib, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-setuprt-"));
  const src = join(dir, "apps/probe/src-tauri/src");
  mkdirSync(src, { recursive: true });
  writeFileSync(join(src, "lib.rs"), lib);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const HOOK = (inside) => `pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
${inside}
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run");
}
`;

const HELPER_BAD = `
fn start_watcher() {
    tokio::spawn(async { });
}
`;

const HELPER_OK = `
fn start_watcher() {
    tauri::async_runtime::spawn(async { });
}
`;

console.log("check-setup-runtime:");

// The case the call-graph-only version passed.
check(
  "a tokio reach in the hook's own body is caught",
  HOOK("            tokio::spawn(async { });"),
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "a tokio reach in a function the hook calls is caught",
  `${HOOK("            start_watcher();")}${HELPER_BAD}`,
  (code, out) => code === 1 && out.includes("start_watcher"),
);

check(
  "a hook that uses the tauri runtime passes",
  `${HOOK("            tauri::async_runtime::spawn(async { });")}${HELPER_OK}`,
  (code) => code === 0,
);

// `tauri::async_runtime::spawn` ends in `tokio::spawn`'s own name; the pattern
// has a lookbehind for exactly that, and a false positive here would push people
// away from the correct call.
check(
  "the tauri runtime spawn is not mistaken for the tokio one",
  HOOK("            tauri::async_runtime::spawn(async { });"),
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all setup-runtime cases passed");
