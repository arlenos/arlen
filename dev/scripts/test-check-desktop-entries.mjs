#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-desktop-entries.py. The case that matters is the HINT one: three
// entries written on 15 August were each "valid" and each named two main categories,
// which puts an app in the menu twice. A check that only failed on hard errors would
// have passed all three, so the control proves it fails on a hint.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-desktop-entries.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

let haveValidator = true;
try {
  execFileSync("desktop-file-validate", ["--version"], { stdio: "ignore" });
} catch {
  haveValidator = false;
}

function tree(entries, identifier = "dev.arlen.good", installedIcons = null) {
  const root = mint("desktop-entries-");
  // The image side. `Icon=` is a theme NAME, so the only thing that can answer
  // whether it resolves is a build phase installing an icon by that name.
  mkdirSync(join(root, "dev", "mkosi", "mkosi.build.d"), { recursive: true });
  const icons = installedIcons ?? Object.keys(entries).map((n) => `arlen-${n}`);
  writeFileSync(
    join(root, "dev", "mkosi", "mkosi.build.d", "04-apps.sh.chroot"),
    icons
      .map((i) => `install -Dm644 "$SRCDIR/x.png" "$DESTDIR/usr/share/icons/hicolor/128x128/apps/${i}.png"\n`)
      .join(""),
  );
  for (const [name, body] of Object.entries(entries)) {
    mkdirSync(join(root, "apps", name, "dist"), { recursive: true });
    writeFileSync(join(root, "apps", name, "dist", `arlen-${name}.desktop`), body);
    // The identifier the app installs under, which is what the entry's app id
    // has to agree with.
    mkdirSync(join(root, "apps", name, "src-tauri"), { recursive: true });
    writeFileSync(
      join(root, "apps", name, "src-tauri", "tauri.conf.json"),
      `{\n  "identifier": "${identifier}"\n}\n`,
    );
  }
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

const good = `[Desktop Entry]
Type=Application
Name=Good
Exec=arlen-good %f
Icon=arlen-good
Terminal=false
Categories=Utility;TextEditor;
X-Arlen-AppId=dev.arlen.good
`;

// Valid to the letter, and it names Utility AND Graphics - two main categories.
const twoMain = good.replace("Categories=Utility;TextEditor;", "Categories=Utility;Graphics;");

// A hard error: Type is required.
const broken = good.replace("Type=Application\n", "");

// Valid to the letter, and unnameable: no app id, so nothing states which app
// this is and every daemon that admits by app id has nothing to key on.
const noAppId = good.replace("X-Arlen-AppId=dev.arlen.good\n", "");

if (!haveValidator) {
  console.log("  skip desktop-file-validate is not installed; the check self-skips too");
  console.log("install desktop-file-utils to exercise this control");
  process.exit(0);
}

{
  const root = tree({ good });
  const rc = run(root);
  rc === 0 ? ok("a clean entry passes") : bad("a clean entry passes", `expected 0, got ${rc}`);
}

{
  // The same entry against an app that installs under a different identifier:
  // valid, nameable, and naming an app nothing can be.
  const root = tree({ good }, "dev.arlen.somethingelse");
  const rc = run(root);
  rc === 1
    ? ok("an app id that disagrees with the install identifier is caught")
    : bad("an app id that disagrees with the install identifier is caught", `expected 1, got ${rc}`);
}

{
  const root = tree({ noAppId });
  const rc = run(root);
  rc === 1
    ? ok("an entry that never says which app it is is caught")
    : bad("an entry that never says which app it is is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree({ good, twomain: twoMain });
  const rc = run(root);
  rc === 1
    ? ok("two main categories is caught, though the entry is valid")
    : bad("two main categories is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree({ broken });
  const rc = run(root);
  rc === 1
    ? ok("a hard error is caught")
    : bad("a hard error is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = mint("desktop-entries-empty-");
  mkdirSync(join(root, "apps"), { recursive: true });
  const rc = run(root);
  rc === 1
    ? ok("finding no entry at all is not a pass")
    : bad("finding no entry at all is not a pass", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // The planted defect: an entry naming an icon no phase installs. It validates
  // perfectly - `Icon=` is a name, not a path - and the launcher draws a
  // placeholder.
  const root = tree(
    {
      good: `[Desktop Entry]\nType=Application\nName=Good\nExec=arlen-good %f\nIcon=arlen-nothing-installs-this\nCategories=Utility;\nX-Arlen-AppId=dev.arlen.good\n`,
    },
    "dev.arlen.good",
    ["arlen-good"],
  );
  const rc = run(root);
  rc === 1
    ? ok("an icon no phase installs is a finding")
    : bad("an icon no phase installs is a finding", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // And the near-miss: the same entry with the install line present passes, so
  // the rung is about the agreement and not about having an Icon key at all.
  const root = tree({
    good: `[Desktop Entry]\nType=Application\nName=Good\nExec=arlen-good %f\nIcon=arlen-good\nCategories=Utility;\nX-Arlen-AppId=dev.arlen.good\n`,
  });
  const rc = run(root);
  rc === 0
    ? ok("an icon a phase installs passes")
    : bad("an icon a phase installs passes", `expected 0, got ${rc}`);
  cleanup(root);
}

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
console.log("a hint fails the check, and an icon nothing installs does too");
