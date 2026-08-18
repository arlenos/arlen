#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for ci-system-deps.sh.
//
// A CI script is the one kind of change that cannot be tried where it is
// written: it runs on a Debian runner as a user with passwordless sudo, and
// this machine is Arch with no apt at all. The last two attempts at this step
// were "obviously right" edits that reached CI before anyone could tell - one
// of them fell through to `break` whether or not the install succeeded. So apt
// and sudo are shimmed here and the behaviour is pressed on directly.
//
// Run: node dev/scripts/test-ci-system-deps.mjs

import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, chmodSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const SCRIPT = join(ROOT, "dev/scripts/ci-system-deps.sh");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// A shimmed runner: `sudo` runs its arguments, `apt-get` behaves as told.
//
// The install shim writes a `.deb` per named package into apt's archive
// directory, which is what the real one does and what the save step reads.
function stage({ installFails = 0, archivesPrefilled = [], cached = [] } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ci-deps-"));
  const bin = join(dir, "bin");
  const archives = join(dir, "archives");
  const cache = join(dir, "cache");
  mkdirSync(bin);
  mkdirSync(archives, { recursive: true });
  mkdirSync(cache, { recursive: true });
  for (const p of archivesPrefilled) writeFileSync(join(archives, `${p}.deb`), "body");
  for (const p of cached) writeFileSync(join(cache, `${p}.deb`), "body");

  writeFileSync(join(bin, "sudo"), '#!/bin/sh\nexec "$@"\n');
  // A counter file, so "fail the first N attempts" is expressible.
  writeFileSync(join(dir, "attempts"), "0");
  writeFileSync(
    join(bin, "apt-get"),
    `#!/bin/sh
mode="$1"; shift
if [ "$mode" = update ]; then exit 0; fi
n=$(cat "${dir}/attempts"); n=$((n+1)); echo "$n" > "${dir}/attempts"
echo "$@" >> "${dir}/install-argv"
if [ "$n" -le ${installFails} ]; then echo "apt-get: mirror said no" >&2; exit 100; fi
for a in "$@"; do
  case "$a" in
    -*|*=*) ;;
    *) : > "${archives}/$a.deb" ;;
  esac
done
exit 0
`,
  );
  chmodSync(join(bin, "sudo"), 0o755);
  chmodSync(join(bin, "apt-get"), 0o755);
  return { dir, bin, archives, cache };
}

function run(s) {
  const r = spawnSync("bash", [SCRIPT], {
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${s.bin}:${process.env.PATH}`,
      APT_CACHE_DIR: s.cache,
      APT_ARCHIVE_DIR: s.archives,
    },
  });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const debs = (d) => readdirSync(d).filter((f) => f.endsWith(".deb")).sort();

console.log("ci-system-deps:");

{
  const r = spawnSync("bash", ["-n", SCRIPT], { encoding: "utf8" });
  check("the script parses", r.status === 0, r.stderr);
}

// Cold: nothing cached, so the run pays for the download and leaves the cache
// populated for the next one. If this does not save, the whole change is
// decoration.
{
  const s = stage();
  const r = run(s);
  const saved = debs(s.cache);
  check("a cold run installs and fills the cache", r.code === 0 && saved.length > 10, `${r.out}\n${saved.join(" ")}`);
  check(
    "the cached set is the package list, not a sample of it",
    saved.includes("libwebkit2gtk-4.1-dev.deb") && saved.includes("libheif-plugin-libde265.deb"),
    saved.join(" "),
  );
  // The failure a shim cannot stage: apt removes the downloaded `.deb` after a
  // successful install unless told otherwise, so on a real runner the save step
  // would copy an empty directory and every run would stay cold while looking
  // like it cached. Assert the option, since the files cannot speak here.
  const argv = readFileSync(join(s.dir, "install-argv"), "utf8");
  check(
    "the install is told to keep the packages it downloaded",
    /APT::Keep-Downloaded-Packages=true/.test(argv),
    argv,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// Warm: the point of the exercise. The cached bodies must reach apt's archive
// directory BEFORE the install runs, because that is the only reason apt does
// not go to the mirror for them.
{
  const s = stage({ cached: ["libwebkit2gtk-4.1-dev", "libgtk-3-dev"] });
  const r = run(s);
  check(
    "a warm run puts the cached packages where apt looks first",
    r.code === 0 && /restored 2 cached package/.test(r.out),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// The bug that shipped last time: a loop that reports success no matter what.
{
  const s = stage({ installFails: 9 });
  const r = run(s);
  check(
    "an install that never succeeds is a failure, not a pass",
    r.code === 1 && /failed after three attempts/.test(r.out),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// And the case the retry exists for: one bad mirror, then a good one.
{
  const s = stage({ installFails: 1 });
  const r = run(s);
  check(
    "a single failed attempt is retried and the run recovers",
    r.code === 0 && /attempt 1 stalled or failed/.test(r.out),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// A warm cache must not hide a failure either: having the bodies locally does
// not mean the install worked.
{
  const s = stage({ installFails: 9, cached: ["libgtk-3-dev"] });
  const r = run(s);
  check("a warm cache does not turn a failed install green", r.code === 1, r.out);
  rmSync(s.dir, { recursive: true, force: true });
}

// The list in the script is the one the workflow must not keep a second copy
// of. Three copies is what it had, and the lint one had already drifted to a
// different set. Matching on `apt-get install` rather than a package name, so
// the prose that legitimately mentions webkit does not read as a fourth copy.
{
  const wf = join(ROOT, ".github/workflows/ci.yml");
  const text = spawnSync("cat", [wf], { encoding: "utf8" }).stdout || "";
  check(
    "the workflow installs nothing on its own",
    !/apt-get\s+install/.test(text),
    "ci.yml installs packages inline again; the script is meant to be the only copy",
  );
}

console.log(failures ? `\n${failures} case(s) failed` : "\nthe cache is used, the failures are still failures");
process.exit(failures ? 1 : 0);
