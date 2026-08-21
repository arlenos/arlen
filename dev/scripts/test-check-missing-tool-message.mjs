// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for check-missing-tool-message: it has to go red when the errno is
// put back, and stay green for the shapes that are deliberately not findings.
//
// Driven over a throwaway tree with its own runtime-deps.tsv and its own git
// index, because the check reads both.

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const CHECK = join(dirname(fileURLToPath(import.meta.url)), "check-missing-tool-message.py");
let failed = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed++;
    if (detail) console.log(`       ${detail}`);
  }
}

function tree(rust) {
  const dir = mkdtempSync(join(tmpdir(), "missing-tool-"));
  mkdirSync(join(dir, "dev/scripts"), { recursive: true });
  writeFileSync(
    join(dir, "dev/scripts/runtime-deps.tsv"),
    "# tool\tpackage\tcomponent\tstate\tnote\nnmcli\tnetwork-manager\tshell\tabsent\tthe network popover\n",
  );
  mkdirSync(join(dir, "src"), { recursive: true });
  writeFileSync(join(dir, "src/thing.rs"), rust);
  execFileSync("git", ["init", "-q"], { cwd: dir });
  execFileSync("git", ["add", "-A"], { cwd: dir });
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [CHECK, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// The fault: the errno becomes the message.
{
  const r = run(
    tree(`fn f() -> Result<(), String> {
    let out = std::process::Command::new("nmcli")
        .args(["dev", "wifi"])
        .output()
        .map_err(|e| format!("nmcli failed: {e}"))?;
    let _ = out;
    Ok(())
}`),
  );
  check("the errno as the message is caught", r.code === 1 && r.out.includes("thing.rs:2"), r.out);
}

// Handled: the code can tell absent from failing.
{
  const r = run(
    tree(`fn f() -> Result<(), String> {
    let out = std::process::Command::new("nmcli")
        .args(["dev", "wifi"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "the network cannot be changed: nmcli is not installed".to_string()
            } else {
                format!("nmcli could not be run ({e})")
            }
        })?;
    let _ = out;
    Ok(())
}`),
  );
  check("a NotFound branch passes", r.code === 0, r.out);
}

// Not a finding: the error never reaches anybody.
{
  const r = run(
    tree(`fn f() -> String {
    let out = match std::process::Command::new("nmcli").arg("x").output() {
        Ok(o) => o,
        _ => return String::new(),
    };
    String::from_utf8_lossy(&out.stdout).to_string()
}`),
  );
  check("a swallowed spawn is not a finding", r.code === 0, r.out);
}

// Not a finding: the map_err belongs to the NEXT statement, which is the false
// positive the first version of the check reported.
{
  const r = run(
    tree(`fn f() -> Result<(), String> {
    let _ = std::process::Command::new("nmcli").arg("unblock").output();

    let conn = connect().map_err(|e| format!("system bus: {e}"))?;
    let _ = conn;
    Ok(())
}
fn connect() -> Result<(), std::io::Error> { Ok(()) }`),
  );
  check("a neighbouring statement's map_err is not attributed to the spawn", r.code === 0, r.out);
}

// A tool the image DOES carry is out of scope: failing to start it is a fault,
// not the ordinary state, and the errno is the useful thing to say.
{
  const r = run(
    tree(`fn f() -> Result<(), String> {
    std::process::Command::new("sh")
        .arg("-c")
        .output()
        .map_err(|e| format!("sh failed: {e}"))?;
    Ok(())
}`),
  );
  check("a tool the image carries is out of scope", r.code === 0, r.out);
}

console.log(
  failed === 0
    ? "\nthe errno is caught, and neither a swallow nor a neighbour is mistaken for it"
    : `\n${failed} case(s) failed`,
);
process.exit(failed === 0 ? 0 : 1);
