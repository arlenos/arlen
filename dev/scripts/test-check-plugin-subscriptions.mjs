// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the plugin-subscription check.
//
// The red case is the one that actually happened three times: a profile grants
// the one topic the app's own code reads and thereby drops the two its plugin
// reads. The green cases are what keeps the check from becoming an opinion -
// a glob may cover them, and an app that declares nothing at all is exempt by
// tier and must not be nagged into writing a list it does not need.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-plugin-subscriptions.py");
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

const CARGO = '[dependencies]\ntauri-plugin-shell = { path = "../../../sdk/tauri-plugin-shell" }\n';

function run(files) {
  const dir = mkdtempSync(join(tmpdir(), "plugin-subs-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const profile = (sub) =>
  sub === null ? "[event_bus]\npublish = []\n" : `[event_bus]\npublish = []\nsubscribe = ${sub}\n`;

console.log("plugin subscriptions:");

{
  // The defect, verbatim: the terminal's accessibility grant with nothing else.
  const r = run({
    "apps/terminal/src-tauri/Cargo.toml": CARGO,
    [`${PROFILES}/dev.arlen.terminal.toml`]: profile('["accessibility.state"]'),
  });
  check(
    "a list that misses the plugin's topics is caught",
    r.code === 1 && r.out.includes("app.toolbar.action_invoked"),
  );
}
{
  const r = run({
    "apps/terminal/src-tauri/Cargo.toml": CARGO,
    [`${PROFILES}/dev.arlen.terminal.toml`]: profile(
      '["accessibility.state", "app.toolbar.action_invoked", "app.shortcut.action_invoked"]',
    ),
  });
  check("naming both passes", r.code === 0);
}
{
  // The shell covers them with `app.toolbar.*` / `app.shortcut.*`, which
  // pattern_matches treats as a prefix on a dot boundary.
  const r = run({
    "apps/desktop-shell/src-tauri/Cargo.toml": CARGO,
    [`${PROFILES}/dev.arlen.desktop-shell.toml`]: profile(
      '["window.*", "app.toolbar.*", "app.shortcut.*"]',
    ),
  });
  check("a glob that covers them passes", r.code === 0);
}
{
  // No declaration is not a defect: the app keeps its tier's exemption and the
  // bus never consults a list. Demanding one would make this a style opinion.
  const r = run({
    "apps/clock/src-tauri/Cargo.toml": CARGO,
    [`${PROFILES}/dev.arlen.clock.toml`]: profile(null),
  });
  check("an app that declares nothing is left alone", r.code === 0);
}
{
  // A component that does not link the plugin has no such subscriptions to lose,
  // so an empty list is right for it and must not be reported.
  const r = run({
    "apps/terminal/src-tauri/Cargo.toml": CARGO,
    [`${PROFILES}/arlen-compositor.toml`]: profile("[]"),
    [`${PROFILES}/dev.arlen.terminal.toml`]: profile(
      '["app.toolbar.action_invoked", "app.shortcut.action_invoked"]',
    ),
  });
  check("a non-plugin component's empty list is left alone", r.code === 0);
}
{
  const r = run({ "README.md": "no apps, no profiles\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
