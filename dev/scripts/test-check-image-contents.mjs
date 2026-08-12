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

import { mkdtempSync, rmSync } from "node:fs";
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
  if (r.status !== 0) throw new Error(`fixture image failed: ${r.stderr}`);
  return img;
}

function check(name, args, expect) {
  const r = spawnSync("bash", [GATE, ...args], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
}

console.log("check-image-contents:");

const img = shelllessImage("noshell.raw", "[Service]\nExecStart=/usr/bin/arlen-ghost\n");

check(
  "an image the inspection could not read is refused, not reported clean",
  [img],
  (code, out) => code === 2 && out.includes("the inspection did not run"),
);

check(
  "naming an image that is not there is an error",
  [join(dir, "no-such.raw")],
  (code) => code !== 0,
);

// Deliberate, and the reason the check can run from `just check-executor` on a
// tree that has never built an image: being asked about a named file that is
// missing is an error, having nothing to ask about is not.
check(
  "naming no image at all on a tree with none is not a failure",
  [],
  (code) => code === 0,
);

rmSync(dir, { recursive: true, force: true });

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all image-contents cases passed (happy path needs a real image)");
