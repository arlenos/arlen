// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-consent-input-ordering`: it has to fail on the shape it
// was written for, and it has to refuse rather than pass when it reads nothing.
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-consent-input-ordering.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

function tree(source) {
  const dir = mkdtempSync(join(tmpdir(), "consent-ordering-"));
  const at = join(dir, "apps/desktop-shell/src-tauri/src");
  mkdirSync(at, { recursive: true });
  writeFileSync(join(at, "consent_window.rs"), source);
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [GATE, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout || "") + (e.stderr || "") };
  }
}

const ARM = `
pub fn arm(app: &AppHandle) {
    let full = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767));
    gtk_window.input_shape_combine_region(Some(&full));
    gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
}
`;

{
  const d = tree(`
pub fn show(app: &AppHandle) {
    let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
    gtk_window.input_shape_combine_region(Some(&empty));
    gtk_window.set_keyboard_mode(KeyboardMode::None);
}
${ARM}`);
  const r = run(d);
  if (r.code === 0) ok("mapping with an empty region and arming separately passes");
  else bad("mapping with an empty region and arming separately passes", r.out);
  rmSync(d, { recursive: true, force: true });
}

{
  const d = tree(`
pub fn show(app: &AppHandle) {
    let full = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767));
    gtk_window.input_shape_combine_region(Some(&full));
}
${ARM}`);
  const r = run(d);
  if (r.code === 1 && r.out.includes("32767x32767"))
    ok("a full input region at map time is caught");
  else bad("a full input region at map time is caught", `exit ${r.code}: ${r.out}`);
  rmSync(d, { recursive: true, force: true });
}

{
  const d = tree(`
pub fn show(app: &AppHandle) {
    gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
}
${ARM}`);
  const r = run(d);
  if (r.code === 1 && r.out.includes("keyboard"))
    ok("taking the keyboard at map time is caught");
  else bad("taking the keyboard at map time is caught", `exit ${r.code}: ${r.out}`);
  rmSync(d, { recursive: true, force: true });
}

{
  // A surface that never becomes answerable is safe and useless, and a check that
  // only ever says "no input taken" would call that a pass.
  const d = tree(`
pub fn show(app: &AppHandle) {
    let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
}
`);
  const r = run(d);
  if (r.code === 1 && r.out.includes("no `arm`"))
    ok("a surface with no way to become answerable is caught too");
  else bad("a surface with no way to become answerable is caught too", `exit ${r.code}: ${r.out}`);
  rmSync(d, { recursive: true, force: true });
}

{
  const d = mkdtempSync(join(tmpdir(), "consent-ordering-empty-"));
  const r = run(d);
  if (r.code === 2) ok("a tree with no consent surface is an error, not a pass");
  else bad("a tree with no consent surface is an error, not a pass", `exit ${r.code}`);
  rmSync(d, { recursive: true, force: true });
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
