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
  // A scalar read as an object. The command answers with a JSON STRING and the
  // call annotates an interface, so the page holds a string and reads fields
  // off it, all undefined. The two shapes never meet, so nothing else sees it.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
async fn demo_report() -> String { "[]".into() }
`,
    "apps/demo/src/lib/call.ts": `
import { invoke } from "@tauri-apps/api/core";
export interface Report { total: number }
export async function go() { return await invoke<Report>("demo_report"); }
`,
  });
  check(
    "a String read as a declared interface is caught",
    r.code === 1 && r.out.includes("demo_report"),
  );
}
{
  // ...and `serde_json::Value` is NOT, because it crosses the bridge as
  // whatever it holds. The annotation is unchecked, which is a different and
  // much smaller thing than wrong.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
async fn demo_state() -> Result<serde_json::Value, String> { Ok(serde_json::json!({})) }
`,
    "apps/demo/src/lib/call.ts": `
import { invoke } from "@tauri-apps/api/core";
export interface State { total: number }
export async function go() { return await invoke<State>("demo_state"); }
`,
  });
  check("a Value read as an interface is unchecked, not wrong", r.code === 0);
}
{
  // A doc comment BETWEEN the attribute and the signature. Legal Rust, and the
  // scanner read it as no command at all - so a real, registered, compiling
  // command reported as "invoked but nothing defines it". I wrote exactly that
  // shape into `config_get` while documenting it, and the gate cried about a
  // function that was working the whole time.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
/// Notes that landed on the wrong side of the attribute.
fn demo_read() -> String { String::new() }
`,
    "apps/demo/src/lib/call.ts": `
import { invoke } from "@tauri-apps/api/core";
export async function go() { return await invoke("demo_read"); }
`,
  });
  check("a doc comment under the attribute still declares the command", r.code === 0);
}
{
  // A nested return generic made the whole CALL invisible, not just its return
  // type: the old pattern stopped the generic at the first `>`, so
  // `invoke<ReadOutcome<{ id: string }>>("x", { ... })` matched nothing and its
  // arguments were never compared. Nine live calls were in that shape.
  const r = run({
    "apps/demo/src-tauri/src/lib.rs": `
#[tauri::command]
fn demo_rows(kind: String) -> String { kind }
`,
    "apps/demo/src/lib/call.ts": `
import { invoke } from "@tauri-apps/api/core";
export async function go() {
  return await invoke<ReadOutcome<{ id: string; label: string }>>(
    "demo_rows",
    { wrong: 1 },
  );
}
`,
  });
  check(
    "a call behind a nested generic is still checked",
    r.code === 1 && /demo_rows/.test(r.out),
    r.out,
  );
}
{
  const r = run({ "README.md": "nothing here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
