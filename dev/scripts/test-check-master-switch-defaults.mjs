// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The gate must catch a real divergence and must not invent one. Both halves are
// checked against a COPY of the tree with the fault put back, because a checker
// that has only ever seen a passing repository is a checker nobody has tested -
// twice this week a green run turned out to mean the check could not see its own
// subject.
import { execFileSync } from "node:child_process";
import { cpSync, readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
// The delete refuses a path it has no record of minting. Written after a control
// in this directory passed the repository root to a recursive remove.
import { mint, cleanup } from "./lib/fixture.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const script = "dev/scripts/check-master-switch-defaults.py";
const BROKER = "daemons/config-broker/src/state.rs";
const SETTINGS = "apps/settings/src-tauri/src/commands/config.rs";

// Mutate ONLY inside `shipped_default()`. `impl Default` above it carries the
// same field names, and a plain `.replace` takes the first occurrence - so the
// first version of these controls edited the fail-closed floor instead of the
// seed, the gate correctly said nothing, and the control read as a gate bug.
// Same trap on the settings side: `access_level = 3` appears in the DOC COMMENT
// above DEFAULT_AI as well as in the literal, so anchor to the literal.
function inDefaultAi(text, from, to) {
  const at = text.indexOf('const DEFAULT_AI');
  if (at < 0) throw new Error("no DEFAULT_AI to mutate");
  return text.slice(0, at) + text.slice(at).replace(from, to);
}

function inShippedDefault(text, from, to) {
  const at = text.indexOf("fn shipped_default()");
  if (at < 0) throw new Error("no shipped_default() to mutate");
  return text.slice(0, at) + text.slice(at).replace(from, to);
}

let failed = 0;
const say = (name, ok, detail) => {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed = 1; }
};

// A sandbox holding just the two files the gate reads, at their real paths.
function sandbox() {
  const dir = mint("msd-");
  for (const f of [BROKER, SETTINGS]) {
    const dest = join(dir, f);
    execFileSync("mkdir", ["-p", dirname(dest)]);
    cpSync(join(root, f), dest);
  }
  cpSync(join(root, script), join(dir, script));
  execFileSync("mkdir", ["-p", join(dir, "dev/scripts")]);
  return dir;
}

function run(dir) {
  try {
    return { code: 0, out: execFileSync("python3", [join(dir, script)], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

console.log("check-master-switch-defaults:");

// 1. The repository as it stands passes. If this ever fails the two shipped
//    defaults have drifted, which is the whole point.
{
  const dir = sandbox();
  const r = run(dir);
  say("the repository itself passes", r.code === 0, r.out.trim());
  cleanup(dir);
}

// 2. A broker seed that turns the executor on while the shipped ai.toml leaves it
//    off. THE case: the switch that decides whether the assistant may write.
{
  const dir = sandbox();
  const p = join(dir, BROKER);
  writeFileSync(p, inShippedDefault(readFileSync(p, "utf8"), "executor_live: false,", "executor_live: true,"));
  const r = run(dir);
  say("an executor_live divergence is caught",
      r.code === 1 && r.out.includes("executor_live"), r.out.trim());
  cleanup(dir);
}

// 3. A read-scope divergence, to prove it is not one hardcoded field.
{
  const dir = sandbox();
  const p = join(dir, BROKER);
  writeFileSync(p, inShippedDefault(readFileSync(p, "utf8"), "access_level: 3,", "access_level: 4,"));
  const r = run(dir);
  say("an access_level divergence is caught",
      r.code === 1 && r.out.includes("access_level"), r.out.trim());
  cleanup(dir);
}

// 4. The other direction: the settings app moving while the broker stands still.
//    A gate that only reads one side would pass this.
{
  const dir = sandbox();
  const p = join(dir, SETTINGS);
  writeFileSync(p, inDefaultAi(readFileSync(p, "utf8"), "access_level = 3", "access_level = 2"));
  const r = run(dir);
  say("a divergence introduced on the settings side is caught too",
      r.code === 1 && r.out.includes("access_level"), r.out.trim());
  cleanup(dir);
}

// 5. It must not go red at a switch the shipped ai.toml simply omits. DEFAULT_AI
//    names no `action_mode`, and absence there means the floor, which is what the
//    broker seeds - so silence is agreement, not drift.
{
  const dir = sandbox();
  const r = run(dir);
  say("a switch absent from DEFAULT_AI reads as the floor, not as drift",
      r.code === 0 && !r.out.includes("action_mode"), r.out.trim());
  cleanup(dir);
}

if (failed) process.exit(1);
console.log("the two places that ship the AI master switches cannot drift apart unnoticed");
