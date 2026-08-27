#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-profile-keys.py. The fault is staged as it actually arrived -
// `read = [...]` under `[filesystem]`, which is not a field, in a profile that
// otherwise reads like a considered grant - rather than as a contrived string.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-profile-keys.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

// Enough of the real schema for the extractor to work on, in its real shape.
const SCHEMA = `
pub struct PermissionProfile {
    pub info: ProfileInfo,
    #[serde(default)]
    pub filesystem: FilesystemPermissions,
    #[serde(default)]
    pub network: NetworkPermissions,
}

pub struct ProfileInfo {
    pub app_id: String,
    #[serde(default = "default_tier")]
    pub tier: AppTier,
}

pub struct FilesystemPermissions {
    #[serde(default)]
    pub home: bool,
    #[serde(default)]
    pub custom: Vec<PathBuf>,
    #[serde(default)]
    pub read_only: Vec<PathBuf>,
}

pub struct NetworkPermissions {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}
`;

function tree({ profile, schema = SCHEMA, catalogue = null, named = null } = {}) {
  const root = mkdtempSync(join(tmpdir(), "profilekeys-"));
  mkdirSync(join(root, "sdk/permissions/src"), { recursive: true });
  mkdirSync(join(root, "sdk/permissions/profiles"), { recursive: true });
  mkdirSync(join(root, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"), { recursive: true });
  if (schema !== null) writeFileSync(join(root, "sdk/permissions/src/lib.rs"), schema);
  if (profile !== null) {
    writeFileSync(
      join(root, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000/dev.arlen.thing.toml"),
      profile,
    );
  }
  // The authored corpus, checked since 27 August: it is the larger body and the
  // one edited in bulk, and it was outside this gate entirely.
  if (catalogue !== null) {
    writeFileSync(join(root, `sdk/permissions/profiles/${named ?? "some-app"}.toml`), catalogue);
  }
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf-8", stdio: "pipe" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

const GOOD = `[info]
app_id = "dev.arlen.thing"
tier = "first-party"

[filesystem]
read_only = [
    "/home/$USER",
]
`;

{
  const root = tree({ profile: GOOD });
  const rc = run(root);
  rc === 0 ? ok("a profile whose keys all exist passes") : bad("a profile whose keys all exist passes", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The defect as made: a plausible-looking key the parser silently drops.
  const root = tree({ profile: GOOD.replace("read_only = [", "read = [") });
  const rc = run(root);
  rc === 1 ? ok("a key the schema does not have is caught") : bad("a key the schema does not have is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = tree({ profile: GOOD + "\n[filesytem]\nhome = true\n" });
  const rc = run(root);
  rc === 1 ? ok("a misspelled SECTION is caught too") : bad("a misspelled SECTION is caught too", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // A sub-table's fields live in a type this does not resolve, so it must not
  // guess: reporting confident nonsense about a shape it half-knows is worse
  // than saying nothing about it.
  const root = tree({
    profile: GOOD + '\n[[graph.relations]]\nfrom = "a"\nto = "b"\ntype = "C"\n',
  });
  const rc = run(root);
  rc === 1 ? ok("an unknown section is still caught when it holds a sub-table") : bad("an unknown section is still caught when it holds a sub-table", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // A gate that reads nothing must not look like a gate that found nothing wrong.
  const root = tree({ profile: GOOD, schema: null });
  const rc = run(root);
  rc === 2 ? ok("a missing schema is an error, not a pass") : bad("a missing schema is an error, not a pass", `expected 2, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const root = tree({ profile: null });
  const rc = run(root);
  rc === 2 ? ok("finding no profiles at all is not a pass") : bad("finding no profiles at all is not a pass", `expected 2, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The same defect one directory over: a key the schema does not have parses as
  // an empty section, so a grant somebody wrote is silently not there.
  const root = tree({ profile: GOOD, catalogue: '[info]\napp_id = "some-app"\ntier = "third-party"\n\n[filesystem]\nread = ["/home"]\n' });
  const rc = run(root);
  rc === 1
    ? ok("an unknown key in the authored corpus is caught")
    : bad("an unknown key in the authored corpus is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  // The file is found by NAME and the id inside is what a peer is compared
  // against, so a disagreement means one of the two reaches nobody.
  const root = tree({
    profile: GOOD,
    named: "some-app",
    catalogue: '[info]\napp_id = "other-app"\ntier = "third-party"\n',
  });
  const rc = run(root);
  rc === 1
    ? ok("a file whose name and app_id disagree is caught")
    : bad("a file whose name and app_id disagree is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

{
  const repo = join(here, "..", "..");
  const rc = run(repo);
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `expected 0, got ${rc}`);
}

console.log("every key in a shipped permission profile is one the schema accepts");
process.exit(failures === 0 ? 0 : 1);
