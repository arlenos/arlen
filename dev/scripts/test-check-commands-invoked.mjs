// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the dark-command check: watch it fail on each shape it claims
// to catch, and pass on the shape it must not.
//
// Fixture trees, not the repo, so it keeps working as the carried list shrinks.
// The repo is asked one thing at the end: that the scan reads it at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-commands-invoked.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** One app registering `cmds`, with `frontend` as its only source text. */
function tree(cmds, frontend) {
  const root = mint("commands-invoked-");
  const app = join(root, "apps/example");
  mkdirSync(join(app, "src-tauri/src"), { recursive: true });
  mkdirSync(join(app, "src"), { recursive: true });
  writeFileSync(
    join(app, "src-tauri/src/lib.rs"),
    `fn main() {\n  builder.invoke_handler(tauri::generate_handler![\n${cmds
      .map((c) => `            commands::${c},`)
      .join("\n")}\n  ]);\n}\n`,
  );
  writeFileSync(join(app, "src/page.ts"), frontend);
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

{
  const root = tree(["do_thing"], 'invoke("do_thing");\n');
  check("a command something calls passes", run(root).code === 0);
  cleanup(root);
}

{
  const root = tree(["do_thing", "forgotten"], 'invoke("do_thing");\n');
  const r = run(root);
  check("a command nobody names fails", r.code === 1 && r.out.includes("forgotten"));
  cleanup(root);
}

// Generous on purpose: a name built into a table or handed to a helper is still
// a call, and demanding the literal `invoke("x")` shape would report live
// commands as dead.
{
  const root = tree(["do_thing"], 'const table = { open: "do_thing" };\ninvoke(table.open);\n');
  check("a name reached through a table counts as called", run(root).code === 0);
  cleanup(root);
}

// The scan must not silently pass a tree it could not read.
{
  const root = mint("commands-invoked-empty-");
  check("an empty scan is a failure, not a pass", run(root).code === 1);
  cleanup(root);
}

{
  const r = run(ROOT);
  check("and the repo itself passes", r.code === 0 && /command\(s\) across/.test(r.out));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe check fails when it should");
process.exit(failures ? 1 : 0);
