// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both directions for `check-fixture-answers-whole.py`.
//
// Builds a tree - one app with a command and a struct, one host fixture - and
// points the check at it with a root argument. The first cut edited the REAL
// fixtures instead, which passes alone and fails at random under the pre-commit
// hook, where the gates run concurrently; its sibling control did exactly that.

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-fixture-answers-whole.py");

let failed = 0;
function ok(name, cond) {
  console.log(`  ${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) failed++;
}

const RUST = `
#[derive(Serialize)]
pub struct MessageDto {
    pub subject: String,
    pub text: String,
    pub to: Vec<String>,
    pub sealed: Option<String>,
}

#[tauri::command]
fn mail_open(id: String) -> MessageDto {
    todo!()
}
`;

/// One app, one fixture, then run the check against that tree.
function run(answer) {
  const root = mint("fixture-answers-");
  try {
    const src = join(root, "apps", "mail", "src-tauri", "src");
    mkdirSync(src, { recursive: true });
    writeFileSync(join(src, "lib.rs"), RUST);
    const hosts = join(root, "dev", "screenshot", "hosts");
    mkdirSync(hosts, { recursive: true });
    writeFileSync(
      join(hosts, "demo.js"),
      `invoke: function (cmd) {\n  if (cmd === "mail_open") return Promise.resolve(${answer});\n}\n`,
    );
    const r = spawnSync("python3", [check, root], { encoding: "utf8" });
    return { code: r.status, out: `${r.stdout}${r.stderr}` };
  } finally {
    cleanup(root);
  }
}

console.log("the gate catches a fixture answering less than the shape it promises");

// 1. The mail case: the fields the picture needed, and not the rest.
{
  const { code, out } = run(`{ subject: "s", text: "t" }`);
  ok("an answer missing a required field is caught", code === 1);
  ok("and the finding names the command and the field", out.includes("mail_open") && out.includes("to"));
}

// 2. Whole answer passes. `sealed` is `Option`, which a real host omits.
{
  const { code } = run(`{ subject: "s", text: "t", to: ["a"] }`);
  ok("naming every non-optional field passes", code === 0);
  ok("and an Option field is not demanded", run(`{ subject: "s", text: "t", to: [] }`).code === 0);
}

// 3. An array of the same shape is read the same way.
{
  const { code } = run(`[{ subject: "s", text: "t" }]`);
  ok("an array of objects is read too", code === 1);
}

// 4. An answer this cannot read is skipped, not guessed at.
{
  const { code, out } = run("ROWS");
  ok("a named constant it cannot resolve is skipped", code === 0);
  ok("and the run says how many it did not read", out.includes("not read"));
}

console.log(failed === 0 ? "\nboth directions hold" : `\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
