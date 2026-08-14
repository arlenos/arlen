// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the event-bus scope gate, both halves.
//
// This one is worth pinning in both directions more than most, because the thing
// it guards fails SILENTLY on both sides: a subscription the profile does not
// grant is dropped rather than refused, and a denied publish is dropped with
// nothing said to the producer. A gate that quietly passed everything would look
// exactly like a healthy tree. The cases below are the two that matter - a
// forgotten grant must be caught, and a correct profile must pass - plus the
// vacuous-pass shapes, since a check that reads nothing and reports OK is the
// same failure wearing the gate's clothes.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-subscribe-scope.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(files) {
  const dir = mkdtempSync(join(tmpdir(), "subscribe-scope-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const APP = `
pub async fn watch(consumer: &C) {
    let mut events = consumer.subscribe(vec!["accessibility.state".into()]).await.unwrap();
    while let Some(e) = events.recv().await { drop(e); }
}
`;

console.log("subscribe scope:");

{
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\nsubscribe = ["window.*"]\n',
    "apps/demo/src/watch.rs": APP,
  });
  check(
    "a subscription the profile does not grant is caught",
    r.code === 1 && r.out.includes("accessibility.state"),
  );
}
{
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\nsubscribe = ["accessibility.state"]\n',
    "apps/demo/src/watch.rs": APP,
  });
  check("the granted case passes", r.code === 0);
}
{
  // A prefix registration (`window.`) against a `window.*` grant. The two spell
  // the same idea differently and the bus admits it; a gate that did not would
  // send somebody chasing a profile that is already correct.
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\nsubscribe = ["window.*", "config.*"]\n',
    "apps/demo/src/bus.rs": 'const SUBSCRIPTIONS: &str = "window.,config.";\n',
  });
  check("a prefix registration matches its .* grant", r.code === 0);
}
{
  // Declaring no scope at all is not this gate's business: the bus does not hold
  // such a component to anything. Flagging it would make the gate an opinion
  // about which components deserve profiles, which is the profile work's call.
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]: '[info]\napp_id = "dev.arlen.demo"\n',
    "apps/demo/src/watch.rs": APP,
  });
  check("a profile with no event-bus section is left alone", r.code === 0);
}
{
  // The vacuous-pass shape. A grant whose app subscribes to nothing is reported
  // but does not fail: extra scope is permissive, nothing goes quiet, and
  // removing a grant can break a consumer the gate cannot see.
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\nsubscribe = ["window.*"]\n',
    "apps/demo/src/quiet.rs": "pub fn nothing() {}\n",
  });
  check(
    "a grant with no subscription behind it is reported, not failed",
    r.code === 0 && r.out.includes("no subscription was found"),
  );
}
{
  // The publish half, which is the same silence in the other direction: the
  // event is dropped and the fire-and-forget producer is never told.
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\npublish = []\n',
    "apps/demo/src/emit.rs":
      'pub fn go() { emit_to_event_bus("audio.state", payload()); }\n',
  });
  check(
    "an emit the profile does not grant is caught",
    r.code === 1 && r.out.includes("audio.state"),
  );
}
{
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\npublish = ["audio.state"]\n',
    "apps/demo/src/emit.rs":
      'pub fn go() { emit_to_event_bus("audio.state", payload()); }\n',
  });
  check("the granted emit passes", r.code === 0);
}
{
  // A Tauri window event is not a bus topic. The tree spells those with a
  // scheme (`arlen://x`, `terminal://frame`), and flagging them would send
  // somebody to add nonsense to a profile - the kind of false positive that
  // gets a check switched off.
  const r = run({
    [`${PROFILES}/dev.arlen.demo.toml`]:
      '[info]\napp_id = "dev.arlen.demo"\n\n[event_bus]\npublish = []\n',
    "apps/demo/src/win.rs": 'pub fn go(app: &App) { let _ = app.emit("arlen://ready", 1); }\n',
  });
  check("a Tauri window event is not read as a bus topic", r.code === 0);
}
{
  const r = run({ "README.md": "no profiles here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
