#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-apps-on-image.py. The check exists because an app that is
// finished and absent from the image looks fine from both sides; a check for that
// which cannot be watched failing is worth exactly as much as the silence it
// replaces.
//
// Each case builds a small tree and asserts the exit code, so the four things the
// check claims to do are demonstrated rather than described: it catches an app with
// no step, it accepts one with a stated reason, it rejects an excuse that has gone
// stale, and it refuses to be fooled by a step that only mentions an app in passing.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-apps-on-image.py");

let failures = 0;

function ok(name) {
  console.log(`  ok   ${name}`);
}

function bad(name, detail) {
  console.log(`  FAIL ${name}: ${detail}`);
  failures += 1;
}

/// Build a tree: `apps` is a list of app directory names (each gets a src-tauri so
/// it counts as an app), `steps` is a map of step filename to its contents.
/// The excused apps, asked of the check itself rather than copied here: a copy would
/// go stale the first time one of them is staged, and the control would then fail for
/// a reason that has nothing to do with what it tests.
const excused = execFileSync("python3", [check, "--excused"], { encoding: "utf8" })
  .split("\n")
  .filter(Boolean);

function tree(apps, steps) {
  const root = mkdtempSync(join(tmpdir(), "apps-on-image-"));
  for (const a of apps) mkdirSync(join(root, "apps", a, "src-tauri"), { recursive: true });
  // A tree missing the excused apps is a tree whose excuse list is stale, and the
  // check says so - correctly. Create them so each case tests the one thing it means
  // to.
  for (const a of excused) mkdirSync(join(root, "apps", a), { recursive: true });
  const stepDir = join(root, "dev/mkosi/mkosi.build.d");
  mkdirSync(stepDir, { recursive: true });
  for (const [name, body] of Object.entries(steps)) writeFileSync(join(stepDir, name), body);
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

// A real step: names the app's source directory AND installs into the apps tree.
const realStep = (app) => `#!/bin/sh
cd "$SRCDIR/arlen/apps/${app}"
install -Dm755 "$out" "$DESTDIR/usr/lib/arlen/apps/dev.arlen.${app}/bin/arlen-${app}"
`;

// 1. The defect the check was written for: an app with no step at all.
{
  const root = tree(["files", "viewers"], { "04c-files.sh.chroot": realStep("files") });
  const rc = run(root);
  rc === 1
    ? ok("an app with no build step is caught")
    : bad("an app with no build step is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 2. Both staged: the ordinary green.
{
  const root = tree(["files", "viewers"], {
    "04c-files.sh.chroot": realStep("files"),
    "04h-viewers.sh.chroot": realStep("viewers"),
  });
  const rc = run(root);
  rc === 0
    ? ok("every app staged passes")
    : bad("every app staged passes", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 3. A step that only MENTIONS another app - in a comment, say - must not vouch for
//    it. Without this, one careless sentence turns the check green for an app nobody
//    ships.
{
  const root = tree(["files", "viewers"], {
    "04c-files.sh.chroot": `${realStep("files")}\n# see also apps/viewers, which is similar\n`,
  });
  const rc = run(root);
  rc === 1
    ? ok("a passing mention does not count as staging")
    : bad("a passing mention does not count as staging", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 4. A step that names an app but installs nothing into the apps tree is not a
//    staging step either.
{
  const root = tree(["files"], {
    "04c-files.sh.chroot": `#!/bin/sh\ncd "$SRCDIR/arlen/apps/files"\ncargo build --release\n`,
  });
  const rc = run(root);
  rc === 1
    ? ok("naming an app without installing it does not count")
    : bad("naming an app without installing it does not count", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 5. The layout moving must not read as a pass.
{
  const root = mkdtempSync(join(tmpdir(), "apps-on-image-empty-"));
  const rc = run(root);
  rc === 1
    ? ok("a moved layout is not a pass")
    : bad("a moved layout is not a pass", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 6. The real tree passes, which is what the pre-commit hook runs.
{
  const rc = run(join(here, "..", ".."));
  rc === 0
    ? ok("the repository itself passes")
    : bad("the repository itself passes", `expected 0, got ${rc}`);
}

if (failures) {
  console.log(`\n${failures} case(s) failed`);
  process.exit(1);
}
console.log(
  "an unstaged app is caught, a stated reason passes, and neither a passing mention " +
    "nor a build-only step counts as putting an app on the image",
);
