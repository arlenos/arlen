// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the invoke-shape gate.
//
// It had none, and that is how a parser gap survived long enough to accuse an
// innocent caller: a parameter documented with `//` shredded on the commas in
// its own prose, so the command looked like it did not declare something it
// declares, and the report named the CALLER as the party at fault. A gate that
// can point at the wrong file is worse than one that says nothing, because
// somebody edits the file it names.
//
// So both directions: a documented parameter must be seen, and a genuinely
// undeclared key must still be caught - the fix must not have bought silence.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-invoke-shape.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(files) {
  const dir = mkdtempSync(join(tmpdir(), "invoke-shape-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const CALL = `
import { invoke } from "@tauri-apps/api/core";
export async function go() {
  return await invoke("demo_login", { profileId: "p", screenReader: true });
}
`;

console.log("invoke shape:");

{
  // The case that bit. The comment carries commas, which is what shredded the
  // parameter list, and the declaration it hides is a real one.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
fn demo_login(
    profile_id: String,
    // The toggle, handed on, but only when somebody operated it - and this
    // sentence has commas in it, which is the whole point of the case.
    screen_reader: Option<bool>,
) -> Result<(), String> { Ok(()) }
`,
    "apps/demo/src/lib/call.ts": CALL,
  });
  check("a documented parameter is seen", r.code === 0);
}
{
  // ...and the gate must still catch a key nothing declares, or the comment fix
  // would have bought quiet rather than accuracy.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
fn demo_login(profile_id: String) -> Result<(), String> { Ok(()) }
`,
    "apps/demo/src/lib/call.ts": CALL,
  });
  check(
    "a key the command does not declare is still caught",
    r.code === 1 && r.out.includes("screen_reader"),
  );
}
{
  // A required parameter the caller omits is the other asymmetry: `Option<T>`
  // may be left out, anything else may not.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
fn demo_login(profile_id: String, secret: String) -> Result<(), String> { Ok(()) }
`,
    "apps/demo/src/lib/call.ts": `
import { invoke } from "@tauri-apps/api/core";
export async function go() { return await invoke("demo_login", { profileId: "p" }); }
`,
  });
  check("a missing required argument is caught", r.code === 1 && r.out.includes("secret"));
}
{
  const r = run({ "README.md": "nothing here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
