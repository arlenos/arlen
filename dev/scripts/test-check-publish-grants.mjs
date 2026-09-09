#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-publish-grants.py. The real case is from 9 September: the
// terminal was granted `arlen-shell:allow-ambient-set` and its shipped profile
// did not carry `app.ambient.set`, so at the enforce cutover the pulse would
// have gone quiet with nothing in the app to say why. The check passes against
// the repo now because both halves were fixed, which is exactly when a check
// needs proving on the state it was written for.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-publish-grants.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

/// A tree with one app: its capability grants, and its shipped profile.
function tree(grants, publish) {
  const dir = mint("arlen-pubgrants-");
  const cap = join(dir, "apps/demo/src-tauri/capabilities");
  mkdirSync(cap, { recursive: true });
  writeFileSync(
    join(cap, "default.json"),
    JSON.stringify({ permissions: grants.map((g) => `arlen-shell:allow-${g}`) }, null, 2),
  );
  writeFileSync(
    join(dir, "apps/demo/src-tauri/tauri.conf.json"),
    JSON.stringify({ identifier: "dev.arlen.demo" }, null, 2),
  );
  const profiles = join(dir, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000");
  mkdirSync(profiles, { recursive: true });
  writeFileSync(
    join(profiles, "dev.arlen.demo.toml"),
    `[info]\napp_id = "dev.arlen.demo"\ntier = "first-party"\n\n[event_bus]\npublish = [\n${publish
      .map((t) => `    "${t}",`)
      .join("\n")}\n]\nsubscribe = []\n`,
  );
  return dir;
}

function run(dir) {
  try {
    execFileSync("python3", [check, dir], { encoding: "utf8" });
    return { code: 0, out: "" };
  } catch (e) {
    return { code: e.status, out: String(e.stdout ?? "") };
  }
}

// THE REAL CASE.
let dir = tree(["ambient-set", "ambient-clear"], []);
let r = run(dir);
r.code === 1 && r.out.includes("app.ambient.set")
  ? ok("a granted surface with no publish scope fails")
  : bad("a granted surface with no publish scope fails", `code ${r.code}: ${r.out}`);
cleanup(dir);

dir = tree(["ambient-set", "ambient-clear"], ["app.ambient.set", "app.ambient.cleared"]);
r = run(dir);
r.code === 0
  ? ok("and passes once the profile carries it")
  : bad("and passes once the profile carries it", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A PREFIX GRANT IS A REAL GRANT. Several shipped profiles write `app.menu.*`
// rather than both halves, and the bus matches it by prefix - reading it as a
// literal would report an app that is correctly granted.
dir = tree(["menu-register", "menu-unregister"], ["app.menu.*"]);
r = run(dir);
r.code === 0
  ? ok("a trailing-star scope covers the topics under it")
  : bad("a trailing-star scope covers the topics under it", `code ${r.code}: ${r.out}`);
cleanup(dir);

// A comment inside the list ended the first version's regex early and made four
// correctly-granted apps look dark. The parse is TOML now; this holds it there.
dir = mint("arlen-pubgrants-");
{
  const cap = join(dir, "apps/demo/src-tauri/capabilities");
  mkdirSync(cap, { recursive: true });
  writeFileSync(
    join(cap, "default.json"),
    JSON.stringify({ permissions: ["arlen-shell:allow-menu-register"] }),
  );
  writeFileSync(
    join(dir, "apps/demo/src-tauri/tauri.conf.json"),
    JSON.stringify({ identifier: "dev.arlen.demo" }),
  );
  const profiles = join(dir, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000");
  mkdirSync(profiles, { recursive: true });
  writeFileSync(
    join(profiles, "dev.arlen.demo.toml"),
    '[info]\napp_id = "dev.arlen.demo"\ntier = "first-party"\n\n' +
      "[event_bus]\npublish = [\n    # why this one is here\n    \"app.menu.registered\",\n]\nsubscribe = []\n",
  );
}
r = run(dir);
r.code === 0
  ? ok("a comment inside the list does not hide the scope under it")
  : bad("a comment inside the list does not hide the scope under it", `code ${r.code}: ${r.out}`);
cleanup(dir);

// An app with no shell grants at all is not this check's business.
dir = tree([], []);
r = run(dir);
r.code === 0
  ? ok("an app that calls no shell surface passes")
  : bad("an app that calls no shell surface passes", `code ${r.code}: ${r.out}`);
cleanup(dir);

// And the repo itself.
try {
  execFileSync("python3", [check], { encoding: "utf8" });
  ok("and the repo itself passes");
} catch (e) {
  bad("and the repo itself passes", String(e.stdout ?? e));
}

console.log(
  failures === 0
    ? "a surface an app may call is one it may publish, and the check says so"
    : `\n${failures} failure(s)`,
);
process.exit(failures === 0 ? 0 : 1);
