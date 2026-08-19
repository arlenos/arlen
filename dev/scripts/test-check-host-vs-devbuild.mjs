#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for `asks_the_build` in check-host-vs-devbuild.py - the decision
// "is this DEV check standing in for a question about the backend".
//
// The predicate has to be narrow in one direction and strict in the other. A
// debug log behind `import.meta.env.DEV` really is about the build and must pass,
// or the gate becomes a ban on a legitimate Vite feature and gets suppressed. A
// fixture branch in a file that calls a command must fail, because that is the
// shape that put invented printers on the page under `tauri dev` and an invented
// error in every headless screenshot.
//
// Run: node dev/scripts/test-check-host-vs-devbuild.mjs

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-host-vs-devbuild.py");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// Ask the predicate about one file's text, without the gate's own tree walk or
/// its hand-kept baseline - both of which would bury the answer.
function asksTheBuild(text) {
  const py = `
import importlib.util, sys
spec = importlib.util.spec_from_file_location("g", ${JSON.stringify(GATE)})
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
print("YES" if m.asks_the_build(sys.stdin.read()) else "NO")
`;
  const r = spawnSync("python3", ["-c", py], { input: text, encoding: "utf8" });
  return (r.stdout || "").trim() === "YES";
}

console.log("check-host-vs-devbuild (asks_the_build):");

check(
  "a fixture branch in a file that calls a command is caught",
  asksTheBuild(`
import { invoke } from "@tauri-apps/api/core";
export async function load() {
  try { items.set(await invoke("list_things")); }
  catch { if (import.meta.env.DEV) { items.set(FIXTURE); } else { failed.set(true); } }
}
`),
);

check(
  "a swallowed write in a file that calls a command is caught too",
  asksTheBuild(`
import { invoke } from "@tauri-apps/api/core";
export async function write(k, v) {
  try { await invoke("set_thing", { k, v }); }
  catch { if (import.meta.env.DEV) return; revert(k); }
}
`),
);

check(
  "a debug log behind a DEV check, in a file with no command, passes",
  !asksTheBuild(`
export function trace(msg) {
  if (import.meta.env.DEV) console.debug(msg);
}
`),
);

check(
  "a file that calls a command but never asks the build passes",
  !asksTheBuild(`
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";
export async function load() {
  if (!tauriAvailable) { unavailable.set(true); return; }
  items.set(await invoke("list_things"));
}
`),
);

check(
  "the word invoke in prose does not count as a call",
  !asksTheBuild(`
/// We used to invoke the daemon here; the page is static now.
export const enabled = import.meta.env.DEV;
`),
);

// End to end, against the real tree: settings and files were the sweep, so
// neither may appear in the baseline. A regression there is the one this gate
// exists to stop.
{
  const src = spawnSync("cat", [GATE], { encoding: "utf8" }).stdout;
  const baseline = src.slice(src.indexOf("BASELINE = {"), src.indexOf("}", src.indexOf("BASELINE = {")));
  check(
    "the swept apps are not in the baseline",
    !baseline.includes("apps/settings/") && !baseline.includes("apps/files/"),
    "settings and files were fixed; listing them again would hide a regression",
  );
  const r = spawnSync("python3", [GATE], { encoding: "utf8" });
  check("the gate passes on the tree as it stands", r.status === 0, (r.stderr || "").trim());
}

console.log(
  failures
    ? `\n${failures} case(s) failed`
    : "\na build-mode check cannot pose as a backend check, and a debug log is still allowed",
);
process.exit(failures ? 1 : 0);
