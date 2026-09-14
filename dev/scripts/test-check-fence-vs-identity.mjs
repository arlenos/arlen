// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for check-fence-vs-identity.py: a daemon doing both must be
// caught, doing either alone must not be, and a carried entry that stopped
// doing both must be reported as stale rather than silently kept.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-fence-vs-identity.py");

let failures = 0;
function bad(message) {
  console.error(`FAIL ${message}`);
  failures += 1;
}

function tree(daemons) {
  const root = mint("fence-id-");
  for (const [name, body] of Object.entries(daemons)) {
    mkdirSync(join(root, `daemons/${name}/src`), { recursive: true });
    writeFileSync(join(root, `daemons/${name}/src/main.rs`), body);
  }
  return root;
}

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const FENCE = "    fence_writes(&[dir]).ok();\n";
const RESOLVE = "    let _ = ConnectionAuth::extract_from(&stream, uid);\n";

// 1. The fault: one daemon does both.
{
  const root = tree({ both: FENCE + RESOLVE, plain: "fn main() {}\n" });
  const { code, out } = run(root);
  if (code !== 1) bad(`a daemon doing both must fail, got ${code}: ${out}`);
  if (!out.includes("daemons/both")) bad(`the report must name it: ${out}`);
  cleanup(root);
}

// 2. Either alone is fine - the pairing is the finding, not each half.
{
  const root = tree({ fenced: FENCE, resolving: RESOLVE });
  const { code, out } = run(root);
  if (code !== 0) bad(`fencing or resolving alone must pass, got ${code}: ${out}`);
  cleanup(root);
}

// 3. A carried entry that stopped doing both is stale and must be reported, so
//    the list shrinks instead of outliving what it allowed.
{
  const root = tree({ capsuled: FENCE, other: "fn main() {}\n" });
  mkdirSync(join(root, "daemons/capsuled/src"), { recursive: true });
  const { code, out } = run(root);
  if (code !== 1) bad(`a stale carried entry must fail, got ${code}: ${out}`);
  if (!out.includes("no longer do both")) bad(`and say why: ${out}`);
  cleanup(root);
}

// 4. Reading nothing is an error, never a pass.
{
  const root = mint("fence-id-empty-");
  mkdirSync(join(root, "daemons"), { recursive: true });
  const { code } = run(root);
  if (code !== 2) bad(`a tree with no daemon sources must be an error, got ${code}`);
  cleanup(root);
}

if (failures === 0) console.log("check-fence-vs-identity: control ok");
process.exit(failures ? 1 : 0);
