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

import { writeFileSync, readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

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
// The header carries a disk-space precheck that exits 1 when the filesystem
// holding /var/tmp is fuller than the build needs, and this harness runs the
// header verbatim. On 22 August that turned every case red while an image build
// was in flight on the same machine: the precheck refused, nothing after it ran,
// and four cases reported `survived=true exit=1` about a trap they never reached.
// A control that fails because the disk is full is telling the truth about the
// disk and a lie about its subject.
//
// So the precheck is neutralised here - `NEED_GB=0` makes the comparison pass
// without touching the script that ships, and the trap below it is what gets
// tested either way.
const header = lines
  .slice(0, trapAt + 1)
  .join("\n")
  .replace(/^NEED_GB=\d+$/m, "NEED_GB=0");

const failures = [];

function runWith(tail) {
  const dir = mint("arlen-trap-");
  // `here` resolves to the script's own directory, so the script under test has
  // to live where the fake image is.
  writeFileSync(join(dir, "arlen.raw"), COMPLETE);
  writeFileSync(join(dir, "build-image.sh"), `${header}\n${tail}\n`, { mode: 0o755 });
  const r = spawnSync("sh", [join(dir, "build-image.sh")], { encoding: "utf8" });
  const survived = existsSync(join(dir, "arlen.raw"));
  // The content matters as much as the existence: restoring the half-written
  // file instead of the kept one would leave a file present and useless, which
  // is the failure this whole trap exists to prevent.
  const content = survived ? readFileSync(join(dir, "arlen.raw"), "utf8") : null;
  const strayPrev = existsSync(join(dir, "arlen.raw.prev"));
  cleanup(dir);
  return { survived, content, strayPrev, code: r.status ?? 1 };
}

const COMPLETE = "a complete image from an earlier run";

// The recovery cases need a DIFFERENT starting state - `.prev` present, and
// `arlen.raw` there or not - so they set the directory up themselves rather than
// inheriting `runWith`'s "one complete image, no .prev".
function runFrom(files, tail) {
  const dir = mint("arlen-recover-");
  for (const [name, body] of Object.entries(files)) writeFileSync(join(dir, name), body);
  writeFileSync(join(dir, "build-image.sh"), `${header}\n${tail}\n`, { mode: 0o755 });
  const r = spawnSync("sh", [join(dir, "build-image.sh")], { encoding: "utf8" });
  const raw = existsSync(join(dir, "arlen.raw"))
    ? readFileSync(join(dir, "arlen.raw"), "utf8")
    : null;
  const prev = existsSync(join(dir, "arlen.raw.prev"))
    ? readFileSync(join(dir, "arlen.raw.prev"), "utf8")
    : null;
  cleanup(dir);
  return { raw, prev, err: r.stderr ?? "", code: r.status ?? 1 };
}

function check2(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...detail });
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

// The case that motivated the rename. `mkosi --force` deletes the output as it
// starts and only then runs ten minutes of cargo and npm inside the image, so a
// compile error there used to leave no image at all - the very failure the two
// cases above were written to survive, just on the other side of the mkosi call.
// The tail reproduces the real sequence: move the good one aside, write a partial
// over the name, then fail.
check(
  "a failure after the image was moved aside restores the previous one",
  'mv "$here/arlen.raw" "$here/arlen.raw.prev"\nprev_image=1\nwriting_image=1\n' +
    'printf half > "$here/arlen.raw"\nfalse',
  (r) => r.survived && r.content === COMPLETE && r.code !== 0,
);

// Order matters inside the trap: it removes the partial and THEN moves the kept
// copy back. Reversed, the restore would be overwritten by the removal and the
// case above would pass on existence while leaving nothing usable - which is why
// the content is asserted rather than just the file.
check(
  "the restored image is the kept copy, not the partial one",
  'mv "$here/arlen.raw" "$here/arlen.raw.prev"\nprev_image=1\nwriting_image=1\n' +
    'printf half > "$here/arlen.raw"\nfalse',
  // `survived` first: without it a run that restored NOTHING reports content
  // `null`, and `null !== "half"` is true, so the case would pass for a build
  // that lost the image entirely. Checked against the pre-rename trap, where it
  // did exactly that.
  (r) => r.survived && r.content !== "half",
);

// And nothing is left lying around under the .prev name, which would be read as
// a spare image by anyone who found it later.
check(
  "a successful run leaves no stray copy behind",
  'mv "$here/arlen.raw" "$here/arlen.raw.prev"\nprev_image=1\nwriting_image=1\n' +
    'printf new > "$here/arlen.raw"\n[ -z "$prev_image" ] || rm -f "$here/arlen.raw.prev"',
  (r) => r.survived && !r.strayPrev && r.code === 0,
);

// The status has to survive the trap, or a failed build reports success to
// whoever called it and the next step runs on an image that is not there.
// ── recovery from a build that was KILLED, where no trap ran ────────────────
//
// An EXIT trap does not run on SIGKILL, so an out-of-memory kill leaves the last
// good image parked under `.prev` for good. That is not hypothetical: it is the
// state this repository was in on 5 September, and it reads as "no image".
{
  const r = runFrom({ "arlen.raw.prev": COMPLETE }, "true");
  check2("an orphaned .prev with no image is moved back", r.raw === COMPLETE && r.prev === null, r);
  check2("and the run says it did so", r.err.includes("recovered"), r);
}
{
  // Ambiguous: the previous run wrote an output before it died, and whether that
  // output is whole cannot be told from here. Both are left alone.
  const r = runFrom({ "arlen.raw.prev": COMPLETE, "arlen.raw": "possibly partial" }, "true");
  check2(
    "an orphaned .prev beside an image touches neither",
    r.raw === "possibly partial" && r.prev === COMPLETE,
    r,
  );
  check2("and says it cannot tell which is good", r.err.includes("cannot"), r);
}
{
  // The ordinary case must not have grown a message.
  const r = runFrom({ "arlen.raw": COMPLETE }, "true");
  check2("a normal run with no .prev says nothing about recovery", !r.err.includes("recovered"), r);
}

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
