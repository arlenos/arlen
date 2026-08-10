// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the verify-image gate must catch, and what it must leave alone.
//
// The gate keeps one relationship true: the verify image is the release image
// PLUS probes, never minus anything. That relationship cannot be tested by
// building both images - it would cost an hour a run - so it is asserted
// structurally, and a structural assertion is only worth having if the shapes it
// claims to catch are shown to fail it.
//
// Run: node dev/scripts/test-check-verify-image.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-verify-image.sh");
const D = "dev/mkosi/mkosi.build.d";

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-verify-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  try {
    return { code: 0, out: execFileSync("bash", [GATE, dir], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

const RELEASE = 'install -Dm755 "$out" "$DESTDIR/usr/bin/arlen-thing"\n';
const VERIFY_OK =
  'if [ "${ARLEN_VERIFY_IMAGE:-0}" != "1" ]; then exit 0; fi\n' +
  'install -Dm755 "$p" "$DESTDIR/usr/bin/arlen-probe"\n';

check(
  "a verify phase that only adds passes",
  tree({ [`${D}/01-thing.sh.chroot`]: RELEASE, [`${D}/09-verify.sh.chroot`]: VERIFY_OK }),
  (code, out) => code === 0 && out.includes("adds 1 path"),
);

check(
  "a release phase reading the flag fails",
  // The shape that lets the verify image differ by omission: any phase that can
  // ask whether this is a verify build can also skip its own work.
  tree({
    [`${D}/01-thing.sh.chroot`]:
      'if [ "${ARLEN_VERIFY_IMAGE:-0}" = "1" ]; then exit 0; fi\n' + RELEASE,
    [`${D}/09-verify.sh.chroot`]: VERIFY_OK,
  }),
  (code, out) => code !== 0 && out.includes("VERIFY BRANCH IN A RELEASE PHASE"),
);

check(
  "a verify phase overwriting a release path fails",
  tree({
    [`${D}/01-thing.sh.chroot`]: RELEASE,
    [`${D}/09-verify.sh.chroot`]:
      'install -Dm755 "$p" "$DESTDIR/usr/bin/arlen-thing"\n',
  }),
  (code, out) => code !== 0 && out.includes("OVERWRITES A RELEASE PATH"),
);

check(
  "a verify phase that removes something fails",
  tree({
    [`${D}/01-thing.sh.chroot`]: RELEASE,
    [`${D}/09-verify.sh.chroot`]: VERIFY_OK + 'rm -f "$DESTDIR/usr/bin/arlen-thing"\n',
  }),
  (code, out) => code !== 0 && out.includes("REMOVES SOMETHING"),
);

check(
  "a tree with no verify phase at all is quiet",
  // The state before this work existed, and the state of any checkout that never
  // grows a verify variant. Not a finding.
  tree({ [`${D}/01-thing.sh.chroot`]: RELEASE }),
  (code) => code === 0,
);

console.log(failures.length ? "\nsome cases regressed" : "\nall four shapes hold");
process.exit(failures.length ? 1 : 0);
