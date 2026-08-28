#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-openable-apps-can-read.py. The fault is staged as it actually
// arrived - the calendar's real profile, one custom directory and nothing else,
// under an entry that claims a MimeType and takes %f - rather than as a contrived
// string.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-openable-apps-can-read.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree({ entry, profile } = {}) {
  const root = mint("openable-");
  mkdirSync(join(root, "apps/thing/dist"), { recursive: true });
  mkdirSync(join(root, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"), { recursive: true });
  writeFileSync(join(root, "apps/thing/dist/arlen-thing.desktop"), entry);
  if (profile !== null) {
    writeFileSync(
      join(root, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000/dev.arlen.thing.toml"),
      profile,
    );
  }
  return root;
}

const OPENS_FILES = `[Desktop Entry]
Type=Application
Name=Thing
Exec=arlen-thing %f
MimeType=text/calendar;
X-Arlen-AppId=dev.arlen.thing
`;

function run(root) {
  try { execFileSync("python3", [check, root], { encoding: "utf8" }); return 0; }
  catch (e) { return e.status ?? 1; }
}

{
  // THE case: exactly the calendar's shape on 20 August.
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\ncustom = [\n  "$HOME/.local/share/arlen/thing",\n]\n`,
  });
  const rc = run(root);
  rc === 1
    ? ok("an app that opens files with only a custom directory is caught")
    : bad("an app that opens files with only a custom directory is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\nhome = true\n`,
  });
  const rc = run(root);
  rc === 0 ? ok("a home grant passes") : bad("a home grant passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A reader that never writes says so with `read_only`, and that reaches the file
  // just as well. Before 25 Aug this failed, so the only way past the gate was the
  // read-write grant the app does not want.
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\nread_only = [\n    "/home/$USER",\n]\n`,
  });
  const rc = run(root);
  rc === 0 ? ok("a read-only grant on a user directory passes") : bad("a read-only grant on a user directory passes", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A read-only path that reaches nowhere a person keeps files is still no answer.
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\nread_only = [\n    "/sys/class/power_supply",\n]\n`,
  });
  const rc = run(root);
  rc === 1 ? ok("a read-only path outside the user's files is not a read grant") : bad("a read-only path outside the user's files is not a read grant", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // A narrower user dir is a real answer, not a workaround.
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\ndocuments = true\n`,
  });
  const rc = run(root);
  rc === 0 ? ok("a narrower user directory passes too") : bad("a narrower user directory passes too", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // A user-dir flag under the WRONG section must not satisfy it: that would be a
  // different bug reading as this one's fix.
  const root = tree({
    entry: OPENS_FILES,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\ncustom = []\n\n[network]\nhome = true\n`,
  });
  const rc = run(root);
  rc === 1
    ? ok("a home flag in another section does not count")
    : bad("a home flag in another section does not count", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // An app that opens nothing is none of this check's business.
  const root = tree({
    entry: `[Desktop Entry]\nType=Application\nName=Thing\nExec=arlen-thing\nX-Arlen-AppId=dev.arlen.thing\n`,
    profile: `[info]\napp_id = "dev.arlen.thing"\n\n[filesystem]\ncustom = []\n`,
  });
  const rc = run(root);
  rc === 0 ? ok("an app that opens nothing is not asked to read") : bad("an app that opens nothing is not asked to read", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // Reading nothing must not read as a pass.
  const root = mint("openable-empty-");
  const rc = run(root);
  rc === 2 ? ok("finding no entries at all is not a pass") : bad("finding no entries at all is not a pass", `expected 2, got ${rc}`);
  cleanup(root);
}

{
  const rc = run(join(here, "..", ".."));
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `got ${rc}`);
}

console.log(
  failures === 0
    ? "a promise to open a file is checked against permission to read one"
    : `\n${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
