// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// This one is a PARTIAL control and says so. The check asks a built image what is
// in it, and the happy path - a unit naming a binary the image does not ship, and
// the check naming it - needs an image with a shell in it, because the inspection
// runs inside the guest. Building one is the image work that is waiting on disk.
//
// What IS covered is the failure direction, which is where the defect was. A
// fixture image with no `/bin/sh` made every command inside the guest produce
// nothing while guestfish's own error kept the output non-empty, so the emptiness
// guard passed and the check reported "no unit names a missing arlen binary"
// about a filesystem it had never read. Found on 12 Aug by pointing this file's
// first fixture at it.
//
// Needs guestfish; skips cleanly without it, the same way the check does.
//
// Run: node dev/scripts/test-check-image-contents.mjs

import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-image-contents.sh");

if (spawnSync("command", ["-v", "guestfish"], { shell: true }).status !== 0) {
  console.log("check-image-contents: guestfish absent, cases not run");
  process.exit(0);
}

const failures = [];
const dir = mkdtempSync(join(tmpdir(), "arlen-imgcheck-"));

/// A minimal partitioned raw image with an ext4 root on sda2, which is what the
/// check mounts. No shell in it: that is the point of this fixture.
function shelllessImage(name, unit) {
  const img = join(dir, name);
  const r = spawnSync(
    "guestfish",
    [
      "--", "disk-create", img, "raw", "64M",
      ":", "add", img,
      ":", "run",
      ":", "part-init", "/dev/sda", "mbr",
      ":", "part-add", "/dev/sda", "p", "2048", "20479",
      ":", "part-add", "/dev/sda", "p", "20480", "-1024",
      ":", "mkfs", "ext4", "/dev/sda2",
      ":", "mount", "/dev/sda2", "/",
      ":", "mkdir-p", "/usr/lib/systemd/system",
      ":", "write", "/usr/lib/systemd/system/probe.service", unit,
    ],
    { encoding: "utf8" },
  );
  // A build failure here is almost never about this check. libguestfs boots a
  // real qemu, and on a machine already running one - an image build, a verify
  // boot - it dies on `io_uring: Cannot allocate memory` against an 8 MB
  // memlock limit. Measured on 14 Aug: red three times in an afternoon and
  // green minutes later on the identical tree.
  //
  // THROWING WAS THE WRONG ANSWER. This runs in the pre-commit hook, whose own
  // message offers `--no-verify`, so an environmental red teaches the reflex of
  // skipping every structural gate at once - and the day it is a real finding,
  // it gets skipped too. So the case is reported as NOT RUN, loudly, and the
  // rest of the file still runs. Not silently: a check that cannot say whether
  // it ran is the failure this whole directory exists to prevent.
  if (r.status !== 0) {
    console.log(`  SKIP the fixture image could not be built here: ${firstLine(r.stderr)}`);
    return null;
  }
  return img;
}

/** The first useful line of a tool's stderr, so a skip reason stays one line. */
function firstLine(text) {
  return (text || "").split("\n").find((l) => l.trim()) ?? "no output";
}

function check(name, args, expect) {
  const r = spawnSync("bash", [GATE, ...args], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
}

// NOT covered here, and said plainly rather than left as a gap you find later:
// the "every shipped arlen unit is enabled" section runs INSIDE the guest, so
// proving it needs a fixture with a working shell, `find` and `basename` - the
// fixtures below deliberately have none, which is what they test. Copying a shell
// and its libraries into a 64M image to get there would make these cases depend
// on the host's dynamic linker, which is worse than the gap.
//
// So it was shown failing the direct way instead, on 13 Aug: a qcow2 overlay over
// the real `arlen.raw` (instant, copy-on-write, no 4.8G copy), one unit's two
// enable symlinks deleted inside it, then the check run against the overlay. It
// named `arlen-powerd.service` and exited 1; against the unmodified image it
// exits 0. Reproduce with:
//
//   qemu-img create -f qcow2 -b "$PWD/dev/mkosi/arlen.raw" -F raw /tmp/ovl.qcow2
//   guestfish -a /tmp/ovl.qcow2 run : mount /dev/sda2 / : \
//     rm /etc/systemd/user/default.target.wants/arlen-powerd.service
//   dev/scripts/check-image-contents.sh /tmp/ovl.qcow2
//
// The desktop-entry section was shown failing the same way: delete
// `/usr/bin/arlen-clock` in an overlay and the check names
// `arlen-clock <- arlen-clock.desktop` and exits 1.
//
// That is a weaker guarantee than a committed case - it does not re-run - and it
// is recorded here so the difference is visible rather than assumed.

console.log("check-image-contents:");

const img = shelllessImage("noshell.raw", "[Service]\nExecStart=/usr/bin/arlen-ghost\n");

if (img) {
  check(
    "an image the inspection could not read is refused, not reported clean",
    [img],
    (code, out) => code === 2 && out.includes("the inspection did not run"),
  );
}

check(
  "naming an image that is not there is an error",
  [join(dir, "no-such.raw")],
  (code) => code !== 0,
);

// Deliberate, and the reason the check can run from `just check-executor` on a
// tree that has never built an image: being asked about a named file that is
// missing is an error, having nothing to ask about is not.
//
// Run against a COPY of the gate placed where its default path resolves to
// nothing, rather than against the gate in the tree. Two reasons, and the second
// is why this was changed on 18 August:
//
//   - This tree usually HAS an image, so `[]` was not exercising "a tree with
//     none" at all. It was running a full inspection of the real one and passing
//     because that happened to succeed, which is a different assertion than the
//     name makes.
//   - Which also meant it needed an appliance, and the comment above `shellless
//     Image` says exactly what happens then: on a machine already running qemu it
//     dies on `io_uring: Cannot allocate memory`. The fixture path skips for that;
//     this case went red for it, twice in one commit.
//
// A case about the branch that never opens an image must not need libguestfs to
// prove it.
{
  const isolated = join(dir, "scripts");
  mkdirSync(isolated, { recursive: true });
  const copy = join(isolated, "check-image-contents.sh");
  copyFileSync(GATE, copy);
  chmodSync(copy, 0o755);
  const r = spawnSync(copy, [], { encoding: "utf8" });
  const ok = r.status === 0;
  console.log(`  ${ok ? "ok  " : "FAIL"} naming no image at all on a tree with none is not a failure`);
  if (!ok) failures.push({ name: "no image", code: r.status, out: (r.stdout || "") + (r.stderr || "") });
}

// A SOURCE-level case, and labelled as one because the image-level version needs
// a built image with a shell in it. It pins the thing that made this check report
// a false orphan: it read only `mkosi.extra` for what the image stages, so a unit
// a BUILD PHASE installs - which is what a conditional unit must do - looked
// staged nowhere. The kernel sensor was called an orphan on the day it stopped
// being one. If someone drops the build.d source again, the false finding comes
// back and nothing else here would notice.
{
  const src = readFileSync(GATE, "utf8");
  const staged = src.slice(src.indexOf("staged=$("), src.indexOf("orphans=\"\""));
  const ok = staged.includes("mkosi.extra") && staged.includes("mkosi.build.d");
  console.log(`  ${ok ? "ok  " : "FAIL"} both ways a unit reaches the image count as staged`);
  if (!ok) failures.push({ name: "staged sources", code: 1, out: staged });
}

rmSync(dir, { recursive: true, force: true });

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all image-contents cases passed (happy path needs a real image)");
