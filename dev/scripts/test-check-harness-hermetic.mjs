// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for check-harness-hermetic.py: it must catch a home variable a
// daemon reads and the harness does not set, must accept one named as
// deliberately left to the system, and must never report success on a tree it
// could not read.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-harness-hermetic.py");

let failures = 0;
function bad(message) {
  console.error(`FAIL ${message}`);
  failures += 1;
}

function tree({ daemon, harness }) {
  const root = mint("hermetic-");
  mkdirSync(join(root, "daemons/probe/src"), { recursive: true });
  mkdirSync(join(root, "dev/integration/src"), { recursive: true });
  writeFileSync(join(root, "daemons/probe/src/main.rs"), daemon);
  writeFileSync(join(root, "dev/integration/src/lib.rs"), harness);
  return root;
}

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const harnessSetting = (names) => `
impl EphemeralStack {
    pub fn base_env(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
${names.map((n) => `            ("${n}".to_string(), p("x")),`).join("\n")}
        ])
    }

    pub fn next_thing(&self) {}
}
`;

const daemonReading = (names) =>
  names.map((n) => `    let _ = std::env::var_os("${n}");`).join("\n");

// 1. The fault: the daemon reads a variable the harness never sets.
{
  const root = tree({
    daemon: daemonReading(["XDG_STATE_HOME", "HOME"]),
    harness: harnessSetting(["XDG_STATE_HOME"]),
  });
  const { code, out } = run(root);
  if (code !== 1) bad(`an unset home variable must fail, got ${code}: ${out}`);
  if (!out.includes("HOME")) bad(`the report must name HOME: ${out}`);
  if (!out.includes("daemons/probe/src/main.rs")) bad(`and where it is read: ${out}`);
  cleanup(root);
}

// 2. The fix: everything read is set.
{
  const root = tree({
    daemon: daemonReading(["XDG_STATE_HOME", "HOME"]),
    harness: harnessSetting(["XDG_STATE_HOME", "HOME", "XDG_DATA_HOME"]),
  });
  const { code, out } = run(root);
  if (code !== 0) bad(`a fully-set harness must pass, got ${code}: ${out}`);
  cleanup(root);
}

// 3. A variable named as deliberately left to the system is not a finding.
{
  const root = tree({
    daemon: daemonReading(["XDG_DATA_DIRS"]),
    harness: harnessSetting(["HOME"]),
  });
  const { code, out } = run(root);
  if (code !== 0) bad(`a named read-only system var must pass, got ${code}: ${out}`);
  cleanup(root);
}

// 4. Reading nothing is an error, never a pass.
{
  const root = tree({ daemon: "fn main() {}\n", harness: harnessSetting(["HOME"]) });
  const { code } = run(root);
  if (code !== 2) bad(`a tree with no reads must be an error, got ${code}`);
  cleanup(root);
}

if (failures === 0) console.log("check-harness-hermetic: control ok");
process.exit(failures ? 1 : 0);
