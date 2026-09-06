#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-headless-render.py`. The case that matters is the third one:
// the gate was written the morning `sweep-axe.sh` was found calling the renderer
// bare, so the control holds that exact shape - a sweep with no Xvfb anywhere in
// it - and the check must go red on it. The two passing cases are the two files
// in the tree that legitimately build their own recipe, and a commented mention,
// which must never be a finding: one real script carries a sentence telling you
// to use headless.sh, and a scanner that read comments would report it.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-headless-render.py");
const REPO = join(HERE, "..", "..");

function tree(files) {
  const root = mint("headless-render-");
  const dir = join(root, "dev", "screenshot");
  mkdirSync(dir, { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, name), body);
  }
  return root;
}

function gateOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const BARE = `#!/usr/bin/env bash
timeout 300 python3 dev/screenshot/render-wide.py --url "$1" --out shot.png
`;
const WRAPPED = `#!/usr/bin/env bash
xvfb-run -a --server-args="-screen 0 1600x1200x24" bash -c '
  openbox &
  python3 dev/screenshot/render-wide.py --url "$1" --out shot.png
'
`;
const THROUGH_OWNER = `#!/usr/bin/env bash
timeout 300 dev/screenshot/headless.sh --url "$1" --out shot.png
`;
const ONLY_MENTIONED = `#!/usr/bin/env bash
# Use headless.sh, NOT render-wide.py directly: it is what supplies the Xvfb.
echo "see the comment above"
`;

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "a sweep that calls the renderer bare is caught",
    () => tree({ "sweep-bare.sh": BARE }),
    (code, out) => code === 1 && out.includes("sweep-bare.sh"),
    true,
  ],
  [
    "a script that builds its own Xvfb passes",
    () => tree({ "shoot-own-recipe.sh": WRAPPED }),
    (code) => code === 0,
    true,
  ],
  [
    "a script that goes through headless.sh passes",
    () => tree({ "sweep-good.sh": THROUGH_OWNER }),
    (code) => code === 0,
    true,
  ],
  [
    "a comment naming the renderer is not a finding",
    () => tree({ "advice.sh": ONLY_MENTIONED }),
    (code) => code === 0,
    true,
  ],
  [
    "headless.sh itself names it and is not a finding",
    () => tree({ "headless.sh": BARE }),
    (code) => code === 0,
    true,
  ],
];

let failed = 0;
for (const [name, build, ok, temp] of cases) {
  const root = build();
  const { code, out } = gateOn(root);
  if (temp) cleanup(root);
  if (ok(code, out)) {
    console.log(`  ok   ${name}`);
  } else {
    console.log(`  FAIL ${name}`);
    console.log(`       exit ${code}: ${out.trim()}`);
    failed = 1;
  }
}
process.exit(failed);
