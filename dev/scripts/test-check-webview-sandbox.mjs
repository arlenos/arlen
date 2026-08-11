// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The rule: every Tauri app asks for WebKit's containment rather than inheriting
// whatever the default is. It was decided app by app precisely so there would be
// no exemption list decaying into everything being exempt - which makes a blind
// check here the exemption list arriving by the back door.
//
// Written while the confinement work had this file's subject open: a confined
// launch now DECLINES the inner sandbox (the outer one forbids the nested
// namespace it needs), and that made it worth proving the unconfined rule still
// speaks.
//
// Run: node dev/scripts/test-check-webview-sandbox.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-webview-sandbox.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-webkit-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

const ASKS =
  "fn main() {\n" +
  '    if std::env::var_os("WEBKIT_FORCE_SANDBOX").is_none() {\n' +
  '        std::env::set_var("WEBKIT_FORCE_SANDBOX", "1");\n' +
  "    }\n" +
  "    arlen_probe_lib::run()\n" +
  "}\n";

const SILENT = "fn main() {\n    arlen_probe_lib::run()\n}\n";

console.log("check-webview-sandbox:");

check(
  "an app that never asks for containment is caught",
  { "apps/probe/src-tauri/src/main.rs": SILENT },
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "the same app asking for it passes",
  { "apps/probe/src-tauri/src/main.rs": ASKS },
  (code) => code === 0,
);

// One silent app among several must not be averaged away: the defect this was
// written after was twelve apps with it and a thirteenth without.
check(
  "one silent app among several is still caught",
  {
    "apps/one/src-tauri/src/main.rs": ASKS,
    "apps/two/src-tauri/src/main.rs": ASKS,
    "apps/three/src-tauri/src/main.rs": SILENT,
  },
  (code, out) => code === 1 && out.includes("three") && !out.includes("apps/one"),
);

// The fail-closed shape the check states for itself: no app entry points at all
// means the layout moved, not that every app is fine.
check(
  "an empty tree is a moved layout, not a pass",
  { "README.md": "nothing here\n" },
  (code, out) => code === 1 && out.includes("layout moved"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("a silent app is caught, even beside apps that ask");
