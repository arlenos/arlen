#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The Appearance > System field-to-path map exists twice, and the two must agree.
//
// `theme_set_system` in Rust turns a field name (`ansi9`, `sndError`) into a path
// in `theme.toml` (`terminal.ansi.bright_red`, `sounds.error`). The store keeps the
// same map because CLEARING a field goes through the generic `config_reset`, which
// takes a path rather than a field name.
//
// Two copies of a mapping drift, and this one drifts silently. A wrong path on the
// clear side does not throw: it deletes nothing. The row goes back to the theme's
// value on screen while the file keeps the override, and the next launch brings the
// old value back - which reads as "the reset button works sometimes".
//
// Compared by reading both sources rather than by running either, because a check
// that needs a built Tauri binary is a check nobody runs.
//
// Run: node dev/scripts/check-system-paths-agree.mjs

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const RUST = join(ROOT, "apps/settings/src-tauri/src/commands/theme.rs");
const STORE = join(ROOT, "apps/settings/src/lib/stores/themeSystem.ts");

/// Every `"key" => "path",` arm of `system_key_path`.
function rustPaths() {
  const src = readFileSync(RUST, "utf8");
  const start = src.indexOf("fn system_key_path");
  if (start < 0) throw new Error("system_key_path is gone from the Rust side");
  const body = src.slice(start, src.indexOf("\n}", start));
  return Object.fromEntries([...body.matchAll(/"([A-Za-z0-9]+)" => "([a-z_.]+)"/g)].map((m) => [m[1], m[2]]));
}

/// Every `key: "path",` entry of `SYSTEM_PATHS`.
function storePaths() {
  const src = readFileSync(STORE, "utf8");
  const start = src.indexOf("const SYSTEM_PATHS");
  if (start < 0) throw new Error("SYSTEM_PATHS is gone from the store");
  const body = src.slice(start, src.indexOf("\n};", start));
  return Object.fromEntries([...body.matchAll(/(\w+):\s*"([a-z_.]+)"/g)].map((m) => [m[1], m[2]]));
}

const rust = rustPaths();
const store = storePaths();
const findings = [];

// A regex that stopped matching would make every comparison below vacuous and the
// check would pass, so the first thing asserted is that something was read at all.
if (Object.keys(rust).length === 0) findings.push("read no arms from system_key_path");
if (Object.keys(store).length === 0) findings.push("read no entries from SYSTEM_PATHS");

for (const key of new Set([...Object.keys(rust), ...Object.keys(store)])) {
  if (!(key in rust)) findings.push(`${key}: the store clears \`${store[key]}\` and the backend writes no such field`);
  else if (!(key in store)) findings.push(`${key}: the backend writes \`${rust[key]}\` and the store cannot clear it`);
  else if (rust[key] !== store[key])
    findings.push(`${key}: backend writes \`${rust[key]}\`, store clears \`${store[key]}\``);
}

if (findings.length) {
  console.error(`the System field map disagrees in ${findings.length} place(s):\n`);
  for (const f of findings) console.error(`  - ${f}`);
  process.exit(1);
}

console.log(
  `${Object.keys(rust).length} System field(s): the path the backend writes is the ` +
    `path the store clears. A disagreement here deletes nothing and looks like a ` +
    `reset button that works sometimes.`,
);
