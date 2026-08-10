// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The image-cleanup trap must remove a half-written image and must NOT remove a
// finished one.
//
// It exists because a build that died during `systemd-repart` on 10 Aug left a
// 4.4G arlen.raw behind that looked real, and verifying against it reports system
// defects that do not exist. The first version deleted on ANY failure, which is
// worse than the problem: nearly everything above the mkosi call is cross-building
// daemons, so a Rust compile error - this script's most common failure by far -
// would have taken out the previous complete image and turned a typo into a
// 40-minute rebuild.
//
// Both directions are tested against the REAL script text rather than a copy of
// the trap, because a trap that has drifted from the one that ships proves
// nothing. The header up to and including the trap is taken verbatim, and a
// failing command is appended at the point being tested.
//
// Run: node dev/scripts/test-build-image-trap.mjs

import { mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const SCRIPT = join(ROOT, "dev/mkosi/build-image.sh");

const text = readFileSync(SCRIPT, "utf8");
const lines = text.split("\n");
const trapAt = lines.findIndex((l) => l.startsWith("trap "));
if (trapAt < 0) {
  console.log("FAIL could not find the trap line in build-image.sh");
  process.exit(1);
}
// Everything up to and including the trap, which carries `set -eu`, `here=` and
// the `writing_image=""` the trap reads.
const header = lines.slice(0, trapAt + 1).join("\n");

const failures = [];

function runWith(tail) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-trap-"));
  // `here` resolves to the script's own directory, so the script under test has
  // to live where the fake image is.
  writeFileSync(join(dir, "arlen.raw"), "a complete image from an earlier run");
  writeFileSync(join(dir, "build-image.sh"), `${header}\n${tail}\n`, { mode: 0o755 });
  const r = spawnSync("sh", [join(dir, "build-image.sh")], { encoding: "utf8" });
  const survived = existsSync(join(dir, "arlen.raw"));
  rmSync(dir, { recursive: true, force: true });
  return { survived, code: r.status ?? 1 };
}

function check(name, tail, expect) {
  const r = runWith(tail);
  const ok = expect(r);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...r });
}

console.log("build-image trap:");

check(
  "a failure before the image write keeps the previous image",
  "false",
  (r) => r.survived && r.code !== 0,
);

check(
  "a failure during the image write removes the partial image",
  "writing_image=1\nfalse",
  (r) => !r.survived && r.code !== 0,
);

check(
  "a successful run keeps the image it just wrote",
  "writing_image=1\ntrue",
  (r) => r.survived && r.code === 0,
);

// The status has to survive the trap, or a failed build reports success to
// whoever called it and the next step runs on an image that is not there.
check(
  "the build's exit status is passed through unchanged",
  "writing_image=1\nexit 3",
  (r) => r.code === 3,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}: survived=${f.survived} exit=${f.code}`);
  process.exit(1);
}
console.log("all build-image trap cases passed");
