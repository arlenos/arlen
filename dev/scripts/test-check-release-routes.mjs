// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the release-route check.
//
// Both halves need pinning for different reasons. The static half is the one
// that runs everywhere, so it has to catch an app that forgot the exclusion and
// stay quiet about one that did not. The `--built` half is the one that reads
// what was actually emitted, and it must NOT run unasked - an `apps/x/build` on
// a working tree is almost always a dev build, which contains every harness
// route and should, so a check that inspected it by default would go red on a
// healthy tree and be switched off within a day.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-release-routes.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

const WIRED = `
import { routesDir } from "../../dev/build/release-routes.js";
const config = { kit: { files: { routes: routesDir(import.meta.dirname) } } };
export default config;
`;
const UNWIRED = `const config = { kit: {} };\nexport default config;\n`;

function run(files, args = []) {
  const dir = mint("release-routes-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir, ...args], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("release routes:");

{
  const r = run({
    "apps/demo/svelte.config.js": UNWIRED,
    "apps/demo/src/routes/_rendertest/+page.svelte": "<p>harness</p>\n",
    "apps/demo/src/routes/+page.svelte": "<p>real</p>\n",
  });
  check(
    "an app with harness routes and no exclusion is caught",
    r.code === 1 && r.out.includes("_rendertest"),
  );
}
{
  const r = run({
    "apps/demo/svelte.config.js": WIRED,
    "apps/demo/src/routes/_rendertest/+page.svelte": "<p>harness</p>\n",
    "apps/demo/src/routes/+page.svelte": "<p>real</p>\n",
  });
  check("the wired case passes", r.code === 0);
}
{
  // An app with no harness routes needs no exclusion, and demanding one would
  // make the gate an opinion about config style rather than about shipping.
  const r = run({
    "apps/demo/svelte.config.js": UNWIRED,
    "apps/demo/src/routes/+page.svelte": "<p>real</p>\n",
  });
  check("an app with no harness routes is left alone", r.code === 0);
}
{
  // A build present but not asked about: silent. This is the default on every
  // working tree.
  const r = run({
    "apps/demo/svelte.config.js": WIRED,
    "apps/demo/src/routes/_rendertest/+page.svelte": "<p>harness</p>\n",
    "apps/demo/build/_app/entry/app.js": 'const routes = ["_rendertest"];\n',
  });
  check("a build is not inspected unless asked", r.code === 0);
}
{
  // ...and when asked, the same build is a finding.
  const r = run(
    {
      "apps/demo/svelte.config.js": WIRED,
      "apps/demo/src/routes/_rendertest/+page.svelte": "<p>harness</p>\n",
      "apps/demo/build/_app/entry/app.js": 'const routes = ["_rendertest"];\n',
    },
    ["--built"],
  );
  check(
    "--built catches a harness route in the emitted files",
    r.code === 1 && r.out.includes("_rendertest"),
  );
}
{
  const r = run(
    {
      "apps/demo/svelte.config.js": WIRED,
      "apps/demo/src/routes/_rendertest/+page.svelte": "<p>harness</p>\n",
      "apps/demo/build/_app/entry/app.js": 'const routes = ["/","/settings"];\n',
    },
    ["--built"],
  );
  check("--built passes a build that names none", r.code === 0);
}
{
  const r = run({ "README.md": "no apps here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth halves hold");
process.exit(failures ? 1 : 0);
