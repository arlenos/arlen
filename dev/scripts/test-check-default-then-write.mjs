// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the defaulted-read-then-write gate.
//
// The cases that matter are the two the fix turns on: a read defaulted to empty
// before a write must be caught, and the correct shape - NotFound handled as the
// empty case, every other error refused - must pass. A check that flagged the
// correct shape too would be switched off within a day, so both directions are
// pinned here rather than only the finding.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-default-then-write.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(files) {
  const dir = mint("default-write-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("defaulted read then write:");

{
  const r = run({
    "apps/demo/src/save.rs": `
fn persist(path: &Path, item: String) -> std::io::Result<()> {
    let mut list: Vec<String> = std::fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default();
    list.push(item);
    std::fs::write(path, render(&list))
}
`,
  });
  check("a defaulted read feeding a write is caught", r.code === 1 && r.out.includes("persist"));
}
{
  // The shape the fix produces. Absent is the empty case and may write; anything
  // else refuses. This must pass, or the gate teaches people to avoid the fix.
  const r = run({
    "apps/demo/src/save.rs": `
fn persist(path: &Path, item: String) -> std::io::Result<()> {
    let mut list: Vec<String> = match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    list.push(item);
    std::fs::write(path, render(&list))
}
`,
  });
  check("telling absent from unreadable passes", r.code === 0);
}
{
  // A defaulted read with no write after it is a READ path, and reading a file
  // that is not there as empty is usually right. Flagging those would bury the
  // erases under hundreds of correct lines.
  const r = run({
    "apps/demo/src/show.rs": `
fn count(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().count()
}
`,
  });
  check("a defaulted read that writes nothing is left alone", r.code === 0);
}
{
  // Order matters: a write BEFORE the defaulted read is a different function
  // shape and not this defect.
  const r = run({
    "apps/demo/src/order.rs": `
fn stamp(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, "fresh")?;
    let _seen = std::fs::read_to_string(path).unwrap_or_default();
    Ok(())
}
`,
  });
  check("a write before the defaulted read is not the defect", r.code === 0);
}
{
  const r = run({ "README.md": "no rust here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
