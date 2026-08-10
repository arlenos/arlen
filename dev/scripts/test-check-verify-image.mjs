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
import { spawnSync } from "node:child_process";

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

// Both streams, joined, on every path. The try/catch version read `execFileSync`'s
// return value on success - which is stdout alone - so a case asserting on a
// message the gate writes to stderr while still exiting 0 would have seen an empty
// string and been wrong quietly. It also let the sync call echo the child's stderr
// into this run's own output, so an EXPECTED failure printed a wall of red above
// the word "ok". Same shape, same fix, as the pre-commit hook's test.
function run(dir) {
  const r = spawnSync("bash", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
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
  "a probe smuggled into the unconditional copy fails",
  // mkosi.extra is staged into every image, so a probe there ships to users while
  // checks 1-3 still report that the verify image only adds. The bypass, not an
  // observed mistake.
  tree({
    [`${D}/01-thing.sh.chroot`]: RELEASE,
    [`${D}/09-verify.sh.chroot`]: VERIFY_OK,
    "dev/mkosi/mkosi.extra/usr/bin/arlen-probe": "#!/bin/sh\n",
  }),
  (code, out) => code !== 0 && out.includes("PROBE IN THE UNCONDITIONAL COPY"),
);

check(
  "a directory both sides merely create is not a collision",
  // Found by the gate firing on its own author: staging a unit needs
  // `mkdir -p $DESTDIR/usr/lib/systemd/system`, and mkosi.extra contains that same
  // directory, so the probe-in-the-unconditional-copy check reported a probe
  // shipping to users. A directory is not an installed artefact; two phases both
  // needing it to exist is ordinary.
  tree({
    [`${D}/01-thing.sh.chroot`]: RELEASE,
    [`${D}/09-verify.sh.chroot`]:
      VERIFY_OK + 'mkdir -p "$DESTDIR/usr/lib/systemd/system"\n',
    "dev/mkosi/mkosi.extra/usr/lib/systemd/system/arlen-thing.service": "[Unit]\n",
  }),
  (code, out) => code === 0 && out.includes("adds 1 path"),
);

check(
  "a tree with no verify phase at all is quiet",
  // The state before this work existed, and the state of any checkout that never
  // grows a verify variant. Not a finding.
  tree({ [`${D}/01-thing.sh.chroot`]: RELEASE }),
  (code) => code === 0,
);

check(
  "an image build whose phases are gone is a failure, not a pass",
  // The shape this gate could not report on itself: with no phases directory it
  // printed "nothing to check" and exited 0, so moving or deleting the phases
  // would have turned it permanently green while watching nothing. Present here
  // as a case rather than a comment, because that is the only version that stays
  // true.
  tree({ "dev/mkosi/mkosi.conf": "[Distribution]\n" }),
  (code, out) => code !== 0 && out.includes("mkosi.build.d"),
);

check(
  "a tree that builds no image really has nothing to check",
  // The other half, and the reason the first is a distinction rather than a
  // blanket rule: a checkout with no image build at all is out of scope, and
  // failing it would make the gate wrong everywhere it is not needed.
  tree({ "README.md": "no image here\n" }),
  (code, out) => code === 0 && out.includes("no image build"),
);

console.log(failures.length ? "\nsome cases regressed" : "\nevery shape holds");
process.exit(failures.length ? 1 : 0);
