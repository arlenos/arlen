#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-sandbox-env.py. The repository passes it, which is exactly the
// state a check that cannot fail would also be in - so each case below is a tree
// built to be wrong, and the check is asserted to say so.
//
// The first case is the real one, copied in shape from `run_worker` as it stood on
// 16 August: a worker spawned with piped stdio and no cleared environment.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-sandbox-env.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// Returns true when the check PASSED the tree.
function passes(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return true;
  } catch {
    return false;
  }
}

function tree(files) {
  const root = mint("sandbox-env-");
  for (const [rel, body] of Object.entries(files)) {
    const full = join(root, rel);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, body);
  }
  return root;
}

const DIRTY = `fn run_worker(sandbox_bin: &Path, input: &[u8]) -> Result<Vec<u8>, E> {
    let mut child = Command::new(sandbox_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(vec![])
}
`;

const CLEAN = DIRTY.replace(
  ".stdin(Stdio::piped())",
  ".env_clear()\n        .stdin(Stdio::piped())",
);

// The shape as it actually shipped, which the check must refuse.
let root = tree({ "ai/ai-sandbox/src/lib.rs": DIRTY });
passes(root)
  ? bad("a worker spawned without env_clear is refused", "the check passed it")
  : ok("a worker spawned without env_clear is refused");
cleanup(root);

root = tree({ "ai/ai-sandbox/src/lib.rs": CLEAN });
passes(root)
  ? ok("the same spawn with env_clear passes")
  : bad("the same spawn with env_clear passes", "the check refused it");
cleanup(root);

// A literal path rather than a variable: the same defect, spelled differently.
root = tree({
  "apps/viewers/src-tauri/src/decode.rs": `fn go() {
    let c = Command::new("/usr/lib/arlen/arlen-thumbnail-sandbox").spawn();
}
`,
});
passes(root)
  ? bad("a literal worker path is refused too", "the check passed it")
  : ok("a literal worker path is refused too");
cleanup(root);

// Not every spawn is a worker. A check that demanded env_clear from nmcli would be
// wrong and would be turned off within the week.
root = tree({
  "apps/desktop-shell/src-tauri/src/network.rs": `fn go() {
    let c = Command::new("nmcli").arg("dev").spawn();
}
`,
});
passes(root)
  ? ok("an ordinary shell-out is left alone")
  : bad("an ordinary shell-out is left alone", "the check refused it");
cleanup(root);

// Tests drive the worker binaries directly and ship nothing.
root = tree({ "ai/ai-sandbox/tests/thumbnail_integration.rs": DIRTY });
passes(root)
  ? ok("a test that drives a worker directly is not held to it")
  : bad("a test that drives a worker directly is not held to it", "the check refused it");
cleanup(root);

// ... and neither is an in-file test module, which is where most of ours live.
root = tree({
  "ai/ai-sandbox/src/lib.rs": `fn shipped() {}

#[cfg(test)]
mod tests {
    ${DIRTY}
}
`,
});
passes(root)
  ? ok("a #[cfg(test)] module is not held to it")
  : bad("a #[cfg(test)] module is not held to it", "the check refused it");
cleanup(root);

// The one that keeps the exclusion honest: shipped code in a file that ALSO has a
// test module must still be checked, or the exclusion becomes a way to opt out.
root = tree({
  "ai/ai-sandbox/src/lib.rs": `${DIRTY}

#[cfg(test)]
mod tests {
    fn t() {}
}
`,
});
passes(root)
  ? bad("shipped code beside a test module is still checked", "the check passed it")
  : ok("shipped code beside a test module is still checked");
cleanup(root);

// A tree with no Rust in it is a walk that reached nothing, and this used to
// print "check-sandbox-env: ok" over it - the one sentence a check about
// spawning secrets must never say about a tree it did not read.
root = tree({ "README.md": "no rust here\n" });
{
  let code = 0;
  let out = "";
  try {
    out = execFileSync("python3", [check, root], { encoding: "utf8" });
  } catch (e) {
    code = e.status ?? 1;
    out = `${e.stdout ?? ""}${e.stderr ?? ""}`;
  }
  code === 2 && out.includes("NOTHING WAS READ")
    ? ok("a tree with no Rust source refuses rather than passing")
    : bad("a tree with no Rust source refuses rather than passing", `exit ${code}: ${out}`);
}
cleanup(root);

if (failures) {
  console.log(`\n${failures} control(s) failed`);
  process.exit(1);
}
console.log("\nall sandbox-env controls passed");
