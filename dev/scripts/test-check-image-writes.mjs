// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the image-write check.
//
// The two red cases are the two mistakes of 15 Aug, in the order they happened:
// a unit moved to `systemd/user` while its `mkdir` kept saying `system`, and the
// `chmod` under it kept saying `system` too. The first killed a fifteen-minute
// build at its last step; the second was waiting to kill the next one.
//
// The green cases matter as much, because this gate's FIRST run reported ten
// defects and every one was its own regex being wrong: `install -d` makes a
// directory rather than writing a file, and `ln -sf x y` creates y rather than
// requiring it. A gate whose findings are noise teaches people to scroll past
// it, so both spellings are pinned here.

import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-image-writes.py");
const STEP = "dev/mkosi/mkosi.build.d/50-thing.sh.chroot";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

function run(body) {
  const dir = mkdtempSync(join(tmpdir(), "imagewrites-"));
  const p = join(dir, STEP);
  mkdirSync(dirname(p), { recursive: true });
  writeFileSync(p, body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("image writes:");

{
  // The build that died: mkdir names system, the write names user.
  const r = run(`mkdir -p "$DESTDIR/usr/lib/systemd/system"
cat > "$DESTDIR/usr/lib/systemd/user/arlen-kg-probe.service" <<'UNIT'
[Unit]
UNIT
`);
  check(
    "a write into a directory the step never made is caught",
    r.code === 1 && r.out.includes("usr/lib/systemd/user"),
  );
}
{
  // The one waiting behind it: the chmod kept the old path.
  const r = run(`mkdir -p "$DESTDIR/usr/lib/systemd/user"
cat > "$DESTDIR/usr/lib/systemd/user/arlen-kg-probe.service" <<'UNIT'
[Unit]
UNIT
chmod 644 "$DESTDIR/usr/lib/systemd/system/arlen-kg-probe.service"
`);
  check(
    "a chmod on a path nothing wrote is caught",
    r.code === 1 && r.out.includes("chmod"),
  );
}
{
  const r = run(`mkdir -p "$DESTDIR/usr/lib/systemd/user"
cat > "$DESTDIR/usr/lib/systemd/user/arlen-kg-probe.service" <<'UNIT'
[Unit]
UNIT
chmod 644 "$DESTDIR/usr/lib/systemd/user/arlen-kg-probe.service"
`);
  check("the corrected pair passes", r.code === 0);
}
{
  // `mkdir -p a/b/c` makes a/b as well, so a write into a/b is fine.
  const r = run(`mkdir -p "$DESTDIR/var/lib/arlen/permissions/0"
cat > "$DESTDIR/var/lib/arlen/permissions/thing.extra" <<'X'
x
X
`);
  check("a deeper mkdir covers writes into the levels above it", r.code === 0);
}
{
  // The first run's false positives, both directions.
  const r = run(`install -d -m755 "$DESTDIR/usr/bin"
ln -sf /usr/lib/arlen/libexec/arlen-run "$DESTDIR/usr/bin/arlen-run"
`);
  check("install -d counts as making the directory", r.code === 0);
  check("and a symlink is a write, not a demand that the link exist", r.code === 0);
}
{
  const r = run(`install -Dm755 "$X/bin/thing" "$DESTDIR/usr/lib/arlen/libexec/thing"
chmod 755 "$DESTDIR/usr/lib/arlen/libexec/thing"
`);
  check("install -D makes its own parents", r.code === 0);
}
{
  const r = run(`echo hi\n`);
  check("a step with no DESTDIR write passes rather than refusing", r.code === 0);
}
{
  // THE SHAPE THAT GOT THROUGH. A command split over two physical lines with a
  // trailing backslash. Every pattern in the gate matches within one line, so
  // this was not merely unflagged - it was never examined, and the build died on
  // it at `No such file or directory`. Two symlinks in the real tree were written
  // this way and had never been checked.
  const r = run(
    'ln -sf ../a.service \\\n' +
      '       "$DESTDIR/usr/lib/systemd/user/default.target.wants/a.service"\n'
  );
  check("a write split across a line continuation is caught", r.code === 1);
}
{
  // And the same write, with its mkdir above it, still passes - the join must not
  // turn a correct step red.
  const r = run(
    'mkdir -p "$DESTDIR/usr/lib/systemd/user/default.target.wants"\n' +
      'ln -sf ../a.service \\\n' +
      '       "$DESTDIR/usr/lib/systemd/user/default.target.wants/a.service"\n'
  );
  check("a continued write with its mkdir above it passes", r.code === 0);
}
{
  // A write whose path is built from a shell variable cannot be checked. It must
  // still be COUNTED, or a looped write reads the same as no write at all.
  const r = run(
    'mkdir -p "$DESTDIR/usr/lib/systemd/user"\n' +
      'cat > "$DESTDIR/usr/lib/systemd/user/arlen-$variant.service" <<UNIT\nx\nUNIT\n'
  );
  check("a non-literal write path is reported, not silently dropped", r.out.includes("not a literal"));
  check("and it does not fail the step", r.code === 0);
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
