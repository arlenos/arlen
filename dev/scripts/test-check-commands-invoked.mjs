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

// A Tauri command does not cross an app boundary, and this is the case that was
// wrong until 8 September: two apps register the same name, ONE of them calls it,
// and the pooled matcher let that one call vouch for both. Really happened, twice
// - `night_light_set` (shell + Settings, called from Settings) and `register_menu`
// (shell + harness, called from harness).
{
  const root = mint("commands-invoked-per-app-");
  for (const [app, frontend] of [["caller", 'invoke("shared_cmd");\n'], ["other", "// nothing\n"]]) {
    const dir = join(root, "apps", app);
    mkdirSync(join(dir, "src-tauri/src"), { recursive: true });
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(
      join(dir, "src-tauri/src/lib.rs"),
      "fn main() { builder.invoke_handler(tauri::generate_handler![\n  commands::shared_cmd,\n]); }\n",
    );
    writeFileSync(join(dir, "src/page.ts"), frontend);
  }
  const r = run(root);
  check(
    "one app's call does not vouch for another app's copy",
    r.code === 1 && r.out.includes("other") && !r.out.includes("- caller:"),
  );
  cleanup(root);
}

// And the same for the marker: a name is not one command.
{
  const root = mint("commands-invoked-marker-scope-");
  for (const [app, lib] of [
    ["marked", "/// NO CALLER: explained here.\n#[tauri::command]\nfn shared_cmd() {}\n"],
    ["unmarked", "#[tauri::command]\nfn shared_cmd() {}\n"],
  ]) {
    const dir = join(root, "apps", app);
    mkdirSync(join(dir, "src-tauri/src"), { recursive: true });
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(
      join(dir, "src-tauri/src/lib.rs"),
      lib + "\nfn main() { builder.invoke_handler(tauri::generate_handler![\n  commands::shared_cmd,\n]); }\n",
    );
    writeFileSync(join(dir, "src/page.ts"), "// nothing\n");
  }
  const r = run(root);
  check(
    "a marker in one app does not answer for another",
    r.code === 1 && r.out.includes("unmarked") && !r.out.includes("- marked:"),
  );
  cleanup(root);
}

// A command that answers for itself beside the code, which is where a reader
// goes to ask. This is how an entry leaves the carried list.
{
  const root = mint("commands-invoked-marked-");
  const app = join(root, "apps/example");
  mkdirSync(join(app, "src-tauri/src"), { recursive: true });
  mkdirSync(join(app, "src"), { recursive: true });
  writeFileSync(
    join(app, "src-tauri/src/lib.rs"),
    "/// NO CALLER: the surface it belongs to has not been built.\n" +
      "#[tauri::command]\nfn later() {}\n\n" +
      "fn main() { builder.invoke_handler(tauri::generate_handler![\n  commands::later,\n]); }\n",
  );
  writeFileSync(join(app, "src/page.ts"), "// nothing\n");
  check("a command that says why nothing calls it passes", run(root).code === 0);
  cleanup(root);
}

// And the marker has to be the command's own, not a neighbour's.
{
  const root = mint("commands-invoked-neighbour-");
  const app = join(root, "apps/example");
  mkdirSync(join(app, "src-tauri/src"), { recursive: true });
  mkdirSync(join(app, "src"), { recursive: true });
  writeFileSync(
    join(app, "src-tauri/src/lib.rs"),
    "/// NO CALLER: this one is explained.\n#[tauri::command]\nfn later() {}\n\n" +
      "/// This one is not.\n#[tauri::command]\nfn forgotten() {}\n\n" +
      "fn main() { builder.invoke_handler(tauri::generate_handler![\n  commands::later,\n  commands::forgotten,\n]); }\n",
  );
  writeFileSync(join(app, "src/page.ts"), "// nothing\n");
  const r = run(root);
  check(
    "a marker does not cover the command next to it",
    r.code === 1 && r.out.includes("forgotten") && !r.out.includes("later,"),
  );
  cleanup(root);
}

{
  const r = run(ROOT);
  check("and the repo itself passes", r.code === 0 && /command\(s\) across/.test(r.out));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe check fails when it should");
process.exit(failures ? 1 : 0);
