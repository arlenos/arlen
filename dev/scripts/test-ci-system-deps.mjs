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

import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  readdirSync,
  chmodSync,
  rmSync,
  existsSync,
} from "node:fs";
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
function stage({ installFails = 0, archivesPrefilled = [], cached = [], fourOhFour = false, uris = [] } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ci-deps-"));
  const bin = join(dir, "bin");
  const archives = join(dir, "archives");
  const cache = join(dir, "cache");
  mkdirSync(bin);
  mkdirSync(archives, { recursive: true });
  mkdirSync(cache, { recursive: true });
  for (const p of archivesPrefilled) writeFileSync(join(archives, `${p}.deb`), "body");
  for (const p of cached) writeFileSync(join(cache, `${p}.deb`), "body");
  if (uris.length) writeFileSync(join(dir, "uris"), uris.map((u) => `'http://m/${u}.deb' ${u}.deb 1 SHA\n`).join(""));

  writeFileSync(join(bin, "sudo"), '#!/bin/sh\nexec "$@"\n');
  // A counter file, so "fail the first N attempts" is expressible.
  writeFileSync(join(dir, "attempts"), "0");
  writeFileSync(
    join(bin, "apt-get"),
    `#!/bin/sh
mode="$1"; shift
if [ "$mode" = update ]; then echo called >> "${dir}/update-calls"; exit 0; fi
# A resolution is not an installation: --print-uris prints what it would
# fetch and installs nothing, so it must not consume an attempt here either.
case " $* " in *" --print-uris "*) cat "${dir}/uris" 2>/dev/null; exit 0;; esac
n=$(cat "${dir}/attempts"); n=$((n+1)); echo "$n" > "${dir}/attempts"
echo "$@" >> "${dir}/install-argv"
if [ "$n" -le ${installFails} ]; then
  ${fourOhFour
    ? 'echo "E: Failed to fetch mirror+file:/pool/universe/libh/libheif/libheif-dev_1.17.6-1ubuntu4.6_amd64.deb  404  Not Found" >&2'
    : 'echo "apt-get: mirror said no" >&2'}
  exit 100
fi
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

function run(s, extraEnv = {}) {
  const r = spawnSync("bash", [SCRIPT], {
    encoding: "utf8",
    env: {
      ...process.env,
      // The retry COUNT is what these cases are about; the pause between
      // attempts is not, and paying it three times per failing case cost this
      // control most of its runtime. The script defaults to five when unset, so
      // CI and a real run are unaffected.
      APT_RETRY_SLEEP: "0",
      PATH: `${s.bin}:${process.env.PATH}`,
      APT_CACHE_DIR: s.cache,
      APT_ARCHIVE_DIR: s.archives,
      ...extraEnv,
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

// The key is a cache of the package LIST, not of the script that installs them.
// Keyed on the script, the 19 August retry-logic fix invalidated the cache for
// all 118 jobs at once and sent every one of them back to the mirror on the run
// that most needed a warm start.
{
  const wf = readFileSync(join(ROOT, ".github/workflows/ci.yml"), "utf8");
  check(
    "the cache key hashes the package list rather than the install script",
    wf.includes("hashFiles('dev/scripts/ci-system-packages.txt')") &&
      !wf.includes("hashFiles('dev/scripts/ci-system-deps.sh')"),
    "the key still hashes the script, so editing the retry logic costs a cold run",
  );
}

// The ratchet the 19 August runs died of: the step is killed from outside, so
// the archive is never written, so the next run restores nothing and hits the
// same mirror. The script's own budget is what keeps the ending in its hands.
{
  const s = stage({ installFails: 99 });
  const r = run(s, { APT_BUDGET_SECS: "70" });
  check(
    "a run that gives up still leaves its packages behind for the next one",
    /package cache holds/.test(r.out),
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

// THE CASE THE WARM-FIRST PATH EXISTS FOR. On 19 August a job died in
// `apt-get update` with every package already restored from the cache. A warm
// run must not ask the mirror anything it does not need.
{
  const s = stage({ cached: ["libgtk-3-dev"] });
  const r = run(s);
  check(
    "a warm run installs without asking the mirror for an index",
    r.code === 0 && !existsSync(join(s.dir, "update-calls")),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// And the fallback the staleness argument rests on: an install the image's
// index cannot satisfy has to reach `update`, not give up warm.
{
  const s = stage({ installFails: 1, cached: ["libgtk-3-dev"] });
  const r = run(s);
  check(
    "a warm install that fails falls back to updating the index",
    r.code === 0 && existsSync(join(s.dir, "update-calls")),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// A cold run has nothing to try, so it must go the long way round.
{
  const s = stage({});
  const r = run(s);
  check(
    "a cold run still fetches the index first",
    r.code === 0 && existsSync(join(s.dir, "update-calls")),
    r.out,
  );
  rmSync(s.dir, { recursive: true, force: true });
}

// A warm cache must not hide a failure either: having the bodies locally does
// not mean the install worked.
{
  const s = stage({ installFails: 9, cached: ["libgtk-3-dev"] });
  const r = run(s);
  {
    // A 404 is the runner image's index naming a version the archive has
    // superseded. Retrying against the same index cannot fix it, and the cached
    // copy of the dead version would otherwise ride along for every future run,
    // since the key rolls only when the package list or the image changes.
    const s = stage({
      installFails: 1,
      fourOhFour: true,
      cached: ["libheif-dev_1.17.6-1ubuntu4.6_amd64", "libssl-dev_3.0.2-0ubuntu1_amd64"],
      uris: ["libssl-dev_3.0.2-0ubuntu1_amd64"],
    });
    const r = run(s);
    check(
      "a 404 is named as a stale index rather than a slow mirror",
      r.out.includes("stale, not slow"),
      r.out,
    );
    const left = debs(s.cache);
    check(
      "and the superseded package is dropped from the cache",
      !left.some((f) => f.startsWith("libheif-dev_1.17.6")),
      left.join(" "),
    );
    check(
      "while what the archive still offers is kept",
      left.some((f) => f.startsWith("libssl-dev_3.0.2")),
      left.join(" "),
    );
  }

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

// The runner derives its list from the workflow, which is the right shape and
// is exactly how this script - not a check at all - ended up being run as one,
// asking a developer for a fingerprint and failing three apt attempts. The
// declaration is what stops that, so both halves are held: the marker is
// present here, and the runner acts on it.
{
  const script = readFileSync(SCRIPT, "utf8");
  check("the script declares it is not a local gate", /^# not-a-local-gate: \S/m.test(script));

  const runner = readFileSync(join(ROOT, "dev/scripts/run-ci-gates.sh"), "utf8");
  check(
    "the runner reads that declaration rather than a list of names",
    /not-a-local-gate/.test(runner) && !/ci-system-deps/.test(runner.split("\n").filter((l) => !l.trim().startsWith("#")).join("\n")),
    "run-ci-gates.sh should skip by declaration, not by naming this script",
  );

  // And prove it end to end: a marked script in a throwaway workflow is skipped
  // while an unmarked one beside it runs.
  const d = mkdtempSync(join(tmpdir(), "gate-runner-"));
  mkdirSync(join(d, "dev/scripts"), { recursive: true });
  mkdirSync(join(d, ".github/workflows"), { recursive: true });
  writeFileSync(join(d, "dev/scripts/check-marked.sh"), "# not-a-local-gate: would touch the machine\nexit 1\n");
  writeFileSync(join(d, "dev/scripts/check-plain.sh"), "exit 0\n");
  writeFileSync(
    join(d, ".github/workflows/ci.yml"),
    "run: |\n  bash dev/scripts/check-marked.sh\n  bash dev/scripts/check-plain.sh\n",
  );
  writeFileSync(join(d, "dev/scripts/run-ci-gates.sh"), readFileSync(join(ROOT, "dev/scripts/run-ci-gates.sh"), "utf8"));
  const r = spawnSync("bash", [join(d, "dev/scripts/run-ci-gates.sh")], { encoding: "utf8" });
  const out = (r.stdout || "") + (r.stderr || "");
  check(
    "a marked script is skipped, and skipping is said out loud",
    r.status === 0 && /not run here: would touch the machine/.test(out),
    out,
  );
  check("an unmarked script beside it still runs", /check-plain\.sh\s+ok/.test(out), out);
  rmSync(d, { recursive: true, force: true });
}

// The cache key had an EMPTY segment on its first real run - `apt-Linux--<hash>`,
// because `ImageOS` is set on the runner process and not in the workflow's `env`
// map, so the expression resolved to nothing. It cost only re-downloads rather
// than a wrong install, which is exactly why it would have sat there: a key that
// silently stops distinguishing what it names still looks like a key.
{
  const wf = readFileSync(join(ROOT, ".github/workflows/ci.yml"), "utf8");
  const keys = [...wf.matchAll(/^\s+key: (apt-.*)$/gm)].map((m) => m[1]);
  check("every job caches packages under a key", keys.length === 3, `found ${keys.length}`);
  // NB this one reads the source text, so it catches a literally empty segment
  // and NOT the bug above, which was empty only at runtime. The check below is
  // the one that would have caught it; this is here because both shapes exist.
  check(
    "no cache key has a literally empty segment",
    keys.every((k) => !/--/.test(k)),
    keys.join("\n       "),
  );
  check(
    "the key varies with the runner image, not just the package list",
    keys.every((k) => /runner-image/.test(k)),
    keys.join("\n       "),
  );
}

console.log(failures ? `\n${failures} case(s) failed` : "\nthe cache is used, the failures are still failures");
process.exit(failures ? 1 : 0);
