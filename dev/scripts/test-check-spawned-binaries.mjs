// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate is only worth having if it can still speak. These cases pin the two
// halves that matter: it FINDS a spawn of a binary no build step installs, and it
// does not fire on the ways a binary legitimately gets into the image (installed
// directly, symlinked onto PATH, or staged verbatim under mkosi.extra).
//
// Run: node dev/scripts/test-check-spawned-binaries.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-spawned-binaries.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-spawn-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-spawned-binaries:");

check(
  "a spawn of a binary no build step installs is caught",
  { "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n' },
  (code, out) => code === 1 && out.includes("arlen-ghost"),
);

check(
  "a binary a build step installs passes",
  {
    "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n',
    "dev/mkosi/mkosi.build.d/09-ghost.sh.chroot":
      'install -Dm755 "$OUT" "$DESTDIR/usr/lib/arlen/libexec/arlen-ghost"\n',
  },
  (code) => code === 0,
);

// The launcher's shape: the real binary in libexec, a PATH symlink beside it. The
// spawn names the symlink, so a rule that only read `install` destinations would
// report a binary that is plainly there.
check(
  "a PATH symlink counts as installed",
  {
    "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n',
    "dev/mkosi/mkosi.build.d/09-ghost.sh.chroot":
      'install -Dm755 "$OUT" "$DESTDIR/usr/lib/arlen/libexec/arlen-ghost"\n' +
      'ln -sf /usr/lib/arlen/libexec/arlen-ghost "$DESTDIR/usr/bin/arlen-ghost"\n',
  },
  (code) => code === 0,
);

check(
  "a binary staged verbatim under mkosi.extra counts as installed",
  {
    "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n',
    "dev/mkosi/mkosi.extra/usr/bin/arlen-ghost": "#!/bin/sh\n",
  },
  (code) => code === 0,
);

// Foreign tools are out of scope on purpose: whether the image installs `pactl`
// is a package decision, and reporting fifteen of those would bury the two that
// are ours.
check(
  "a foreign program is not this gate's business",
  { "apps/probe/src/lib.rs": 'std::process::Command::new("pactl").spawn();\n' },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all spawned-binary cases passed");
