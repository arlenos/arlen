#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-serde-nesting.py.
//
// The defect it was written for is planted back here, in the shape it actually
// had: a camelCase tick carrying a struct that forgot the attribute, so one
// field went out as `per_core` and the pane that read `perCore` rendered
// nothing. A green run on the real tree proves nothing about a checker; these
// cases do.
//
// Run: node dev/scripts/test-check-serde-nesting.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-serde-nesting.py");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// A tree with one Rust file under a crate root the gate reads.
function tree(source) {
  const dir = mint("serde-nesting-");
  mkdirSync(join(dir, "apps/thing/src"), { recursive: true });
  writeFileSync(join(dir, "apps/thing/src/lib.rs"), source);
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

console.log("check-serde-nesting:");

// The defect, as it actually happened on 18 August.
{
  const d = tree(`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemTick {
    pub cpu_pct: f64,
    pub load: Option<LoadAverage>,
}

#[derive(Serialize)]
pub struct LoadAverage {
    pub one: f64,
    pub per_core: f64,
}
`);
  const r = run(d);
  check(
    "a nested struct that forgot the attribute is caught",
    r.code === 1 && /per_core/.test(r.out) && /LoadAverage/.test(r.out),
    r.out,
  );
  cleanup(d);
}

// The fix has to clear it.
{
  const d = tree(`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemTick {
    pub load: Option<LoadAverage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAverage {
    pub per_core: f64,
}
`);
  const r = run(d);
  check("the same pair with the attribute passes", r.code === 0, r.out);
  cleanup(d);
}

// A per-field rename is the other legitimate fix, and must not be nagged at.
{
  const d = tree(`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outer {
    pub inner: Inner,
}

#[derive(Serialize)]
pub struct Inner {
    #[serde(rename = "perCore")]
    pub per_core: f64,
}
`);
  const r = run(d);
  check("an explicit per-field rename is accepted", r.code === 0, r.out);
  cleanup(d);
}

// Not a campaign for camelCase: a snake_case struct nobody camelCase reaches is
// nobody's business. Half this tree's Serialize structs are TOML.
{
  const d = tree(`
#[derive(Serialize)]
pub struct Config {
    pub watch_directories: Vec<String>,
}

#[derive(Serialize)]
pub struct Also {
    pub some_field: u32,
}
`);
  const r = run(d);
  check("a snake_case struct no camelCase parent reaches is left alone", r.code === 0, r.out);
  cleanup(d);
}

// `Vec<T>` and `Option<T>` reach T just as a bare field does.
{
  const d = tree(`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outer {
    pub many: Vec<Row>,
}

#[derive(Serialize)]
pub struct Row {
    pub row_id: u32,
}
`);
  const r = run(d);
  check("a struct reached through Vec is checked too", r.code === 1 && /row_id/.test(r.out), r.out);
  cleanup(d);
}

// A single-word field cannot disagree with anything, so it is not a finding.
{
  const d = tree(`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outer {
    pub inner: Inner,
}

#[derive(Serialize)]
pub struct Inner {
    pub level: String,
    pub some10: f64,
}
`);
  const r = run(d);
  check("single-word fields are not a disagreement", r.code === 0, r.out);
  cleanup(d);
}

// Pointed at a tree with nothing to read, "no findings" would describe a scan
// that read nothing.
{
  const d = mint("serde-nesting-empty-");
  mkdirSync(join(d, "apps"), { recursive: true });
  check("a tree with no Serialize structs is an error, not a pass", run(d).code === 2);
  cleanup(d);
}

console.log(failures ? `\n${failures} case(s) failed` : "\na nested struct cannot quietly disagree with its parent");
process.exit(failures ? 1 : 0);
