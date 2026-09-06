// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-erased-refusal.py`.
//
// The fixtures below are the three real instances reduced to their shape - the
// generic config store, its reset sibling and the screen filter - plus the forms
// that must NOT fire. The check was written after the fixes, so it has never been
// red on the tree; this is the only thing standing between it and being decorative.

import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "check-erased-refusal.py"), "utf8");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

function run(body, name = "store.ts") {
  const root = mint("erased-refusal-");
  try {
    const dir = join(root, "apps", "demo", "src", "lib");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, name), body);
    mkdirSync(join(root, "dev", "scripts"), { recursive: true });
    const copy = join(root, "dev", "scripts", "check-erased-refusal.py");
    writeFileSync(copy, source);
    const r = spawnSync("python3", [copy], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

console.log("the gate catches a rollback that erases the failure it recovers from");

// 1. The shape as it actually stood in the config store.
{
  const { code, out } = run(`
async function load() {
  inner.update((s) => ({ ...s, loading: true, error: null }));
  const data = await invoke("config_get");
  inner.set({ data, error: null });
}
async function setValue(key, value) {
  applyLocal(key, value);
  try {
    await invoke("config_set", { key, value });
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    await load();
  }
}
`);
  ok("a catch that records and then awaits its loader is caught", code === 1);
  ok("and the finding names the loader and the field", out.includes("load()") && out.includes("error"));
}

// 2. The fix: record AFTER the rollback.
{
  const { code } = run(`
async function load() {
  inner.update((s) => ({ ...s, loading: true, error: null }));
}
async function setValue(key, value) {
  try {
    await invoke("config_set", { key, value });
  } catch {
    await load();
    inner.update((s) => ({ ...s, writeFailed: true }));
  }
}
`);
  ok("recording after the rollback passes", code === 0);
}

// 3. The other fix: a field the loader has no reason to touch.
{
  const { code } = run(`
async function load() {
  inner.update((s) => ({ ...s, loading: true, error: null }));
}
async function setValue(key, value) {
  try {
    await invoke("config_set", { key, value });
  } catch {
    inner.update((s) => ({ ...s, writeFailed: true }));
    await load();
  }
}
`);
  ok("a field the loader does not clear passes", code === 0);
}

// 4. A catch that records and awaits nothing at all.
{
  const { code } = run(`
async function load() {
  inner.update((s) => ({ ...s, error: null }));
}
async function setValue() {
  try {
    await invoke("config_set");
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
  }
}
`);
  ok("a catch that records and rolls nothing back passes", code === 0);
}

// 5. A rollback that clears a DIFFERENT field.
{
  const { code } = run(`
async function load() {
  inner.update((s) => ({ ...s, loading: false }));
}
async function setValue() {
  try {
    await invoke("config_set");
  } catch (e) {
    inner.update((s) => ({ ...s, error: String(e) }));
    await load();
  }
}
`);
  ok("a loader that clears something else passes", code === 0);
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
