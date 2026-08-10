// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The pre-commit hook's own blind spot, and the warning that names it.
//
// The hook runs the gates against the working tree, so a file staged in one state
// and edited into another is judged on the version that is NOT being committed.
// That is how a throwaway line landed inside the very commit that added the hook.
// The gap is not closed - partial staging is legitimate - it is announced, and a
// warning nobody has proved fires is worth about as much as the comment it
// replaced. So case two builds exactly that situation and requires the warning.
//
// Each case runs in a throwaway repo with NO `dev/scripts/run-ci-gates.sh`, which
// is deliberate: the hook then takes its "gates not run" path, and whether the
// warning appears is decided by the divergence check alone rather than by fifteen
// seconds of unrelated gates.
//
// Run: node dev/scripts/test-pre-commit-hook.mjs

import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync, spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const HOOK = join(ROOT, ".githooks/pre-commit");

const failures = [];

// Setup git, kept quiet: the seed commit fires the hook too, and four repos'
// worth of "gates not run" printed between the results makes the results the
// hard part to find.
function git(dir, ...args) {
  return execFileSync("git", ["-C", dir, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  });
}

/** A repo with the hook installed and one committed file to build on. */
function repo() {
  const dir = mkdtempSync(join(tmpdir(), "arlen-hook-test-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "t@example.invalid");
  git(dir, "config", "user.name", "t");
  mkdirSync(join(dir, ".githooks"), { recursive: true });
  copyFileSync(HOOK, join(dir, ".githooks/pre-commit"));
  chmodSync(join(dir, ".githooks/pre-commit"), 0o755);
  git(dir, "config", "core.hooksPath", ".githooks");
  writeFileSync(join(dir, "seed"), "seed\n");
  git(dir, "add", "seed");
  git(dir, "commit", "-qm", "seed");
  return dir;
}

/** Commit, returning what the hook said and whether it let the commit through.
 *
 * Both streams, joined. The first version read `execFileSync`'s return value,
 * which is stdout alone - and the hook says everything on stderr, so the case
 * this file exists for reported a missing warning that was in fact printed. A
 * test that watches the wrong stream fails honestly-looking and is worse than
 * no test, since the fix goes into whatever it points at. */
function commit(dir) {
  const r = spawnSync("git", ["-C", dir, "commit", "-m", "x"], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, fn) {
  const dir = repo();
  let ok = false;
  try {
    ok = fn(dir);
  } catch (e) {
    ok = false;
    failures.push({ name, err: String(e) });
  }
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name });
  rmSync(dir, { recursive: true, force: true });
}

check("a whole staged file draws no warning", (dir) => {
  writeFileSync(join(dir, "a.txt"), "one\n");
  git(dir, "add", "a.txt");
  const { code, out } = commit(dir);
  return code === 0 && !out.includes("not the version being committed");
});

check("a file staged then edited is named", (dir) => {
  // The real shape: staged at one state, edited at another, gates saw the second.
  writeFileSync(join(dir, "b.txt"), "staged\n");
  git(dir, "add", "b.txt");
  writeFileSync(join(dir, "b.txt"), "edited after staging\n");
  const { code, out } = commit(dir);
  return code === 0 && out.includes("not the version being committed") && out.includes("b.txt");
});

check("an unstaged file alone is not a divergence", (dir) => {
  // Only files that are BOTH staged and further edited matter; a file left dirty
  // beside the commit is not being committed at all, and warning about it would
  // fire on nearly every commit and be tuned out within a day.
  writeFileSync(join(dir, "c.txt"), "staged\n");
  git(dir, "add", "c.txt");
  writeFileSync(join(dir, "loose.txt"), "never staged\n");
  const { code, out } = commit(dir);
  return code === 0 && !out.includes("not the version being committed");
});

check("the warning does not block the commit", (dir) => {
  // Deliberate: partial staging is legitimate, and a hook that refuses it gets
  // bypassed with --no-verify, after which it protects nothing at all.
  writeFileSync(join(dir, "d.txt"), "staged\n");
  git(dir, "add", "d.txt");
  writeFileSync(join(dir, "d.txt"), "edited\n");
  commit(dir);
  return git(dir, "log", "--oneline").split("\n").filter(Boolean).length === 2;
});

console.log(failures.length ? "\nsome cases regressed" : "\nevery shape holds");
process.exit(failures.length ? 1 : 0);
