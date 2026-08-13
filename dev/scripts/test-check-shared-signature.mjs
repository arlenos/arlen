// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the shared-signature check: show that it does work only
// when it should, and that its selection is right.
//
// Deliberately NOT compiling anything. The check's expensive half is `cargo check`
// over the dependents, which the real planted-defect run exercised (a two-argument
// `lookup` in `sdk/permissions` named five broken crates, including the changed
// crate's own tests and `daemons/config-broker`). What a fast control can pin, and
// what would silently rot, is the part around it: which commits do work at all,
// and which crates get selected. A check that quietly stopped selecting anything
// would still exit 0 forever.

import { spawnSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-shared-signature.sh");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(...paths) {
  const r = spawnSync("bash", [CHECK, ...paths], { encoding: "utf8", cwd: ROOT });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// A commit that cannot break anything elsewhere must cost nothing. This is the
// property that keeps the hook fast enough not to be bypassed, which matters more
// than the check itself - a bypassed hook protects nothing.
{
  const started = Date.now();
  const r = run("daemons/knowledge/src/daemon.rs", "apps/files/core/src/lib.rs");
  check("a commit touching no shared crate does no work", r.code === 0 && r.out.trim() === "");
  check("and returns immediately", Date.now() - started < 3000);
}

// A shared change selects the crate itself AND its dependents. `--all-targets` on
// the changed crate is what catches a break in its own tests, which is exactly the
// miss that prompted this.
{
  const r = spawnSync("bash", ["-c", `bash ${CHECK} sdk/permissions/src/lib.rs | head -1`], {
    encoding: "utf8",
    cwd: ROOT,
  });
  const line = (r.stdout || "").trim();
  check("a shared change announces the crate it came from", line.includes("sdk/permissions"));
  const count = Number((line.match(/checking (\d+) affected/) || [])[1] || 0);
  check(`and selects the dependents too (${count} crates)`, count > 5);
}

// The build cache holds a VENDORED checkout of this repo whose manifests match the
// same search. Checking it would compile a stale copy of the tree against new
// source and report failures about neither.
{
  const r = spawnSync("bash", ["-c", `bash ${CHECK} contracts/audit-proto/src/lib.rs | head -1`], {
    encoding: "utf8",
    cwd: ROOT,
  });
  check(
    "the vendored copy in the build cache is never selected",
    !(r.stdout || "").includes("mkosi.builddir"),
  );
}

console.log(failures ? `\n${failures} failure(s)` : "\nthe selection holds");
process.exit(failures ? 1 : 0);
