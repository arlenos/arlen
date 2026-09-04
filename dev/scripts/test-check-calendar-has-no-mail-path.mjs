#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-calendar-has-no-mail-path.py: put the fault back and watch
// it fail. The fault is staged the way it would actually arrive - a mail crate
// added to the calendar's manifest by somebody implementing RSVP - rather than
// as a contrived string.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-calendar-has-no-mail-path.py");
let failures = 0;
const ok = (name) => console.log(`  ok   ${name}`);
const bad = (name, detail) => {
  console.log(`  FAIL ${name}`);
  console.log(`       ${detail}`);
  failures += 1;
};

function tree({ manifestExtra = "", source = "fn main() {}\n" } = {}) {
  const root = mint("cal-mail-");
  for (const part of ["apps/calendar/core/src", "daemons/calendar/src"]) {
    mkdirSync(join(root, part), { recursive: true });
  }
  writeFileSync(
    join(root, "daemons/calendar/Cargo.toml"),
    `[package]\nname = "arlen-calendar"\nversion = "0.1.0"\n\n[dependencies]\nchrono = "0.4"\n${manifestExtra}`,
  );
  writeFileSync(join(root, "daemons/calendar/src/main.rs"), source);
  writeFileSync(
    join(root, "apps/calendar/core/Cargo.toml"),
    `[package]\nname = "arlen-calendar-core"\nversion = "0.1.0"\n`,
  );
  writeFileSync(join(root, "apps/calendar/core/src/lib.rs"), "pub fn f() {}\n");
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

{
  const root = tree();
  const rc = run(root);
  rc === 0 ? ok("a calendar that sends no mail passes") : bad("a calendar that sends no mail passes", `got ${rc}`);
  cleanup(root);
}

{
  // The edit this exists for: RSVP needs to send one message, so the mail crate
  // goes in the calendar and the open question is answered by accident.
  const root = tree({ manifestExtra: 'lettre = "0.11"\n' });
  const rc = run(root);
  rc === 1
    ? ok("a mail crate in the calendar's manifest is caught")
    : bad("a mail crate in the calendar's manifest is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  const root = tree({ source: 'fn send() { stream.write_all(b"EHLO arlen\\r\\n"); }\n' });
  const rc = run(root);
  rc === 1
    ? ok("calendar code that speaks SMTP is caught")
    : bad("calendar code that speaks SMTP is caught", `expected 1, got ${rc}`);
  cleanup(root);
}

{
  // Reading an invitation is what the calendar already does, deliberately. A
  // check that failed on it would be arguing with the design.
  const root = tree({ source: 'fn read_invite(ics: &str) -> bool { ics.contains("METHOD:REQUEST") }\n' });
  const rc = run(root);
  rc === 0
    ? ok("reading an invitation is not a mail path")
    : bad("reading an invitation is not a mail path", `expected 0, got ${rc}`);
  cleanup(root);
}

{
  // Reading nothing must not read as a pass.
  const root = mint("cal-mail-empty-");
  const rc = run(root);
  rc === 2
    ? ok("finding no calendar at all is not a pass")
    : bad("finding no calendar at all is not a pass", `expected 2, got ${rc}`);
  cleanup(root);
}

{
  const rc = run(join(here, "..", ".."));
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `got ${rc}`);
}

console.log(
  failures === 0
    ? "the calendar still stops where sending begins"
    : `\n${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
