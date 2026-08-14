// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the shared-handoff-name gate.
//
// The case that matters is the one-sided rename, because that is the whole
// reason the check exists: it compiles, it passes every test, and the symptom
// lands on a person rather than on a build. The other cases pin the shapes that
// would make the gate useless - passing when a side is missing, or failing when
// the tree is fine.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-shared-env-names.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// The gate reads its SHARED table from its own source, so a temp tree has to
// reproduce the real handoff's three crates for the recorded entry to apply.
function tree(greeterValue) {
  return {
    "apps/greeter/core/src/lib.rs": `pub const A11Y_SCREEN_READER_ENV: &str = "${greeterValue}";\n`,
    "daemons/session/src/env.rs": 'pub const A11Y_SCREEN_READER: &str = "ARLEN_A11Y_SCREEN_READER";\n',
    "apps/desktop-shell/src-tauri/src/a.rs": 'const HANDOFF_ENV: &str = "ARLEN_A11Y_SCREEN_READER";\n',
  };
}

function run(files) {
  const dir = mkdtempSync(join(tmpdir(), "shared-env-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("shared handoff names:");

{
  const r = run(tree("ARLEN_A11Y_SCREEN_READER"));
  check("all three sides spelling it the same passes", r.code === 0);
}
{
  const r = run(tree("ARLEN_A11Y_READER"));
  check(
    "a one-sided rename is caught",
    r.code === 1 && r.out.includes("apps/greeter"),
  );
}
{
  // A side deleted entirely, not renamed. Same silence, and the message has to
  // name who went missing rather than only that something is wrong.
  const files = tree("ARLEN_A11Y_SCREEN_READER");
  delete files["daemons/session/src/env.rs"];
  const r = run(files);
  check("a side that disappeared is caught by name", r.code === 1 && r.out.includes("daemons/session"));
}
{
  // A name two crates share that nobody recorded: reported so it can be added
  // or dismissed, but not failed - it may be coincidence, and a gate that fails
  // on coincidence gets switched off.
  const files = tree("ARLEN_A11Y_SCREEN_READER");
  files["apps/files/src-tauri/src/x.rs"] = 'const A: &str = "ARLEN_SOMETHING";\n';
  files["daemons/clock/src/y.rs"] = 'const B: &str = "ARLEN_SOMETHING";\n';
  const r = run(files);
  check(
    "an unrecorded shared name is reported, not failed",
    r.code === 0 && r.out.includes("ARLEN_SOMETHING"),
  );
}
{
  const r = run({ "README.md": "no rust here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
