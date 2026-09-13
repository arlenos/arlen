// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for check-integration-builds.py: it must catch a component the
// suite needs and the nightly does not build, and must not complain about the
// reverse. Both halves against a fixture tree, so the real one is untouched.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-integration-builds.py");

function tree({ suite, recipe }) {
  const root = mint("itbuilds-");
  mkdirSync(join(root, "dev/integration/tests"), { recursive: true });
  mkdirSync(join(root, "dev/scripts"), { recursive: true });
  writeFileSync(join(root, "dev/integration/tests/integration_backend_smoke.rs"), suite);
  writeFileSync(join(root, "dev/justfile"), recipe);
  return root;
}

let failures = 0;
function bad(message) {
  console.error(`FAIL ${message}`);
  failures += 1;
}

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const recipeWith = (list) => `
integration-nightly:
    #!/usr/bin/env bash
    for c in ${list}; do
        echo build $c
    done
    cargo test

next-recipe:
    echo unrelated
`;

const suite = `
    stack.spawn("daemons/event-bus", "event-bus", &[]).expect("spawn");
    if !arlen_integration::binary_built("daemons/consent-broker", "arlen-consent-broker") {
        return;
    }
    stack.spawn("store-backend", "arlen-store-backend", &[]).expect("spawn");
`;

// 1. The fault: the recipe misses two of the three the suite names.
{
  const root = tree({ suite, recipe: recipeWith("daemons/event-bus") });
  const { code, out } = run(root);
  if (code !== 1) bad(`a missing component must fail, got ${code}: ${out}`);
  for (const want of ["daemons/consent-broker", "store-backend"]) {
    if (!out.includes(want)) bad(`the report must name ${want}: ${out}`);
  }
  // The hyphenated one is the reason this control exists: the first cut of the
  // parser dropped `store-backend` for its hyphen and reported it missing from a
  // list it was in.
  cleanup(root);
}

// 2. The fix: all three built, and a spare in the recipe is not a complaint.
{
  const root = tree({
    suite,
    recipe: recipeWith("daemons/event-bus daemons/consent-broker store-backend daemons/spare"),
  });
  const { code, out } = run(root);
  if (code !== 0) bad(`a covered suite must pass, got ${code}: ${out}`);
  cleanup(root);
}

// 3. Reading nothing is an error, never a pass: an unparsable recipe must not
//    report success by finding no components to compare.
{
  const root = tree({ suite, recipe: "integration-nightly:\n    echo no loop here\n" });
  const { code } = run(root);
  if (code !== 2) bad(`an unreadable build loop must be an error, got ${code}`);
  cleanup(root);
}

if (failures === 0) console.log("check-integration-builds: control ok");
process.exit(failures ? 1 : 0);
