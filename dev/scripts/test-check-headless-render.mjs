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
//: The same recipe through the shared display helper, which is how every script
//: in this tree builds its own Xvfb since the loop moved into `own-display.sh`.
//: Without this case the check reads as covered while recognising only the older
//: spelling - which is exactly what happened on 11 September.
const OWNED = `#!/usr/bin/env bash
. "$(dirname "\${BASH_SOURCE[0]}")/lib/own-display.sh"
own_display "-screen 0 1600x1200x24" bash -c '
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

// A gate that explains itself names the renderer in its own docstring, and a
// docstring is as much prose as a `#` line. This reported one as a bare render
// on 12 September, which is how the `#`-only rule showed its shape.
const DOCSTRING_MENTION = `#!/usr/bin/env python3
"""Check a thing.

The renderer render-wide.py refuses a page that has one, which is the other
half of this check.
"""
import sys
print("ok")
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
    "a script that owns its display through the helper passes",
    () => tree({ "shoot-owned.sh": OWNED }),
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
    "a python docstring naming the renderer is not a finding",
    () => tree({ "check-thing.py": DOCSTRING_MENTION }),
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
