// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the plugin-subscription check: watch it fail on each shape it
// claims to catch, including the exact one that shipped.
//
// Fixture trees, so it keeps working as apps and profiles come and go. The repo
// is asked one thing at the end: that the scan finds something to read.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-plugin-subscriptions.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** One app that links the plugin, with whatever profile body the caller wants. */
function tree({ links = true, profile = null } = {}) {
  const root = mint("plugin-subs-");
  const app = join(root, "apps/reader/src-tauri");
  mkdirSync(app, { recursive: true });
  writeFileSync(
    join(app, "Cargo.toml"),
    links
      ? '[dependencies]\ntauri-plugin-arlen-shell = { path = "../../../sdk/tauri-plugin-shell" }\n'
      : "[dependencies]\nserde = \"1\"\n",
  );
  mkdirSync(join(root, PROFILES), { recursive: true });
  if (profile !== null) {
    writeFileSync(join(root, PROFILES, "dev.arlen.reader.toml"), profile);
  }
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const GRANTED = `[info]
app_id = "dev.arlen.reader"
[event_bus]
publish = []
subscribe = ["app.toolbar.action_invoked", "app.shortcut.action_invoked"]
`;

{
  const root = tree({ profile: GRANTED });
  check("a linking app granted both patterns passes", run(root).code === 0);
  cleanup(root);
}

// The exact shape that shipped: a careful profile with no bus section at all.
{
  const root = tree({
    profile: '[info]\napp_id = "dev.arlen.reader"\n[filesystem]\nread_only = ["/home/$USER"]\n',
  });
  const r = run(root);
  check(
    "a profile with no event_bus section fails",
    r.code === 1 && r.out.includes("no `[event_bus].subscribe`"),
  );
  cleanup(root);
}

{
  const root = tree({
    profile: '[info]\napp_id = "dev.arlen.reader"\n[event_bus]\nsubscribe = ["app.toolbar.action_invoked"]\n',
  });
  const r = run(root);
  check(
    "half the pair is still a failure, and it names the missing one",
    r.code === 1 && r.out.includes("app.shortcut.action_invoked"),
  );
  cleanup(root);
}

{
  const root = tree({ profile: null });
  const r = run(root);
  check("a linking app with no profile at all fails", r.code === 1 && r.out.includes("no shipped profile"));
  cleanup(root);
}

// An app that does not link the plugin is not asked for the grant - the check
// must not demand a subscription nobody makes.
{
  const root = tree({ links: false, profile: '[info]\napp_id = "dev.arlen.reader"\n' });
  const r = run(root);
  check("an app that does not link the plugin is not asked", r.code === 1 && r.out.includes("pointed wrong"));
  cleanup(root);
}

{
  const r = run(ROOT);
  check("and the repo itself passes", r.code === 0 && /app\(s\) link the plugin/.test(r.out));
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe check fails when it should");
process.exit(failures ? 1 : 0);
