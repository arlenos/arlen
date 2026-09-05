// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the invoke-exists gate actually catch an invoke with nothing behind it?
//
// It had no control until 5 September, which is the wrong one of the ~130 checks
// to leave unproven: it is the gate that answers "why does this surface do
// nothing", and on 27 August it found thirty-two commands that a frontend called
// and no host implemented. A gate that silently stopped matching would give the
// same green as a clean tree.
//
// EVERY CASE RUNS AGAINST A TEMPORARY TREE, never this one. The first version of
// this file wrote a probe into a real app's `src` and deleted it afterwards, and
// the pre-commit hook went red on three unrelated checks the moment it ran: the
// hook says at its top that "the gates run concurrently", so a control that edits
// the tracked tree is visible to every neighbour for as long as it takes. A
// control that breaks the run it is part of is worse than no control.
//
// The gate takes its root as `argv[1]`, so a fixture of four files is enough and
// nothing shared is touched.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
// The house fixture helper: `cleanup` refuses any path `mint` has no record of
// creating, and exits rather than throwing so a `try` further up cannot turn the
// refusal back into a warning. `check-fixture-deletes` sent me here after I wrote
// a bare recursive delete, which is the check written after one of these deleted
// the repository.
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-invoke-exists.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// Build a throwaway tree with one app, run the gate over it, tear it down.
///
/// `calls` are the command names the frontend invokes; `handlers` the ones its
/// host registers. Minted and cleaned through `lib/fixture.mjs`, so the delete
/// cannot be handed a path this function did not create.
function gateOver(calls, handlers, app = "clock") {
  const dir = mint("arlen-invoke-gate-");
  try {
    const src = path.join(dir, "apps", app, "src", "lib");
    const host = path.join(dir, "apps", app, "src-tauri", "src");
    mkdirSync(src, { recursive: true });
    mkdirSync(host, { recursive: true });
    // `package.json` is what makes a directory an app to this gate (it globs
    // `apps/*/package.json`). Without it the run reports "0 app(s) checked" and
    // PASSES - which the first version of this fixture did, so three cases were
    // green over a tree the gate had never looked at. A control that passes
    // vacuously is the failure it exists to catch.
    writeFileSync(path.join(dir, "apps", app, "package.json"), `{"name":"${app}"}\n`, "utf8");
    writeFileSync(
      path.join(src, "probe.ts"),
      `import { invoke } from "@tauri-apps/api/core";\n` +
        calls.map((c) => `export const p_${c} = () => invoke("${c}");\n`).join(""),
      "utf8",
    );
    writeFileSync(
      path.join(host, "lib.rs"),
      handlers
        .map((h) => `#[tauri::command]\nfn ${h}() {}\n`)
        .join("\n") +
        `\npub fn run() {\n    tauri::Builder::default()\n        .invoke_handler(tauri::generate_handler![\n` +
        handlers.map((h) => `            ${h},\n`).join("") +
        `        ])\n}\n`,
      "utf8",
    );
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("invoke-exists:");

// The baseline, over the real tree: every case below is only meaningful if the
// gate passes on a repository that is actually clean. Read-only.
{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

{
  const r = gateOver(["clock_state"], ["clock_state"]);
  check("an invoke whose command is registered passes", r.code === 0, r.out.trim().split("\n")[0]);
  // Pinned, because "0 app(s) checked" also exits 0. Every pass in this file is
  // worthless if the gate never saw the fixture, and that is exactly how the
  // first version of it was green.
  check("and the gate actually looked at the fixture", /\b1 app\(s\) checked/.test(r.out),
        r.out.trim().split("\n")[0]);
}

{
  const r = gateOver(["clock_command_that_does_not_exist"], ["clock_state"]);
  check("an invoke with no command behind it is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the command", r.out.includes("clock_command_that_does_not_exist"));
}

{
  // The inventory is what makes this gate usable: without it every gated seam
  // would be a permanent red and the check would be switched off. `knowledge_library`
  // is carried with a recorded reason, FOR THE KNOWLEDGE APP.
  const r = gateOver(["knowledge_library"], [], "knowledge");
  check("a command carried as known-missing does not fail the run", r.code === 0,
        r.out.trim().split("\n")[0]);
}

{
  // And the half worth having: the excuse is scoped to the app that owns the
  // surface, so the same name elsewhere is still a finding. I assumed the
  // inventory was global when writing this and the gate corrected me.
  const r = gateOver(["knowledge_library"], [], "clock");
  check("but the same name from another app still is one", r.code === 1,
        r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches an absent command, passes a registered one, and scopes its inventory by app");
