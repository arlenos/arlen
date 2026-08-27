// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The gate is only worth having if it can still speak. These cases pin the three
// things it has to get right: it FINDS a spawn of a binary no build step installs,
// it does not fire on the ways a binary legitimately gets into the image (installed
// directly, symlinked onto PATH, or staged verbatim under mkosi.extra), and it
// reports its own KNOWN entries once they stop being true.
//
// Run: node dev/scripts/test-check-spawned-binaries.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-spawned-binaries.py");

const failures = [];

// The gate's KNOWN entries are claims about THIS tree, and the staleness guard
// added on 12 Aug reports an entry nothing spawns any more. A fixture tree has
// none of the real call sites, so every fixture has to carry a stub spawn or the
// guard correctly fires for entries that are perfectly healthy - which is what
// happened the first time this ran, and it is the same shape `test-check-wired`
// documents about its own exemption stubs.
//
// This couples the test to a list in the file it tests, deliberately, and the
// coupling runs the cheap way: add a KNOWN entry without a stub here and THIS
// test goes red immediately, rather than the gate quietly losing the ability to
// be pointed at a fixture.
const CARRIED = {
  // One stub line per KNOWN entry in the gate. The list is EMPTY as of 27 August
  // - `arlen-settings` and `arlen-harness` both gained build steps that day - so
  // this is empty too. A stub for a name the gate no longer carries makes every
  // fixture here report a real missing binary, which is the coupling working
  // rather than failing; a missing stub for a name it does carry is the same in
  // reverse. Add one line here with the entry, not after it.
};

function check(name, files, expect, { known } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-spawn-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  // KNOWN is hardcoded in the gate, so the staleness half cannot be reached from
  // a fixture tree - the entries it would need are the real ones, which are true.
  // Rather than leave that half unproven, the test runs a COPY of the gate with
  // an entry planted at the top of the real dict. Everything else is the same
  // file, so the logic under test is the logic that ships.
  let gate = GATE;
  if (known) {
    const src = readFileSync(GATE, "utf8").replace(
      "KNOWN = {",
      `KNOWN = {\n    ${JSON.stringify(known)}: "planted",`,
    );
    gate = join(dir, "gate.py");
    writeFileSync(gate, src);
  }
  const r = spawnSync("python3", [gate, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-spawned-binaries:");

check(
  "a spawn of a binary no build step installs is caught",
  { ...CARRIED, "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n' },
  (code, out) => code === 1 && out.includes("arlen-ghost"),
);

check(
  "a binary a build step installs passes",
  {
    ...CARRIED,
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
    ...CARRIED,
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
    ...CARRIED,
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
  { ...CARRIED, "apps/probe/src/lib.rs": 'std::process::Command::new("pactl").spawn();\n' },
  (code) => code === 0,
);

// The staleness half, added 12 Aug. `arlen-run` left this list by gaining a build
// step, and nothing here would have said so: the main loop skips an installed
// binary before it consults KNOWN, and it walks spawn SITES, so an entry whose
// last caller is gone is never examined either. Each entry is a sentence about the
// tree, and a sentence that has stopped being true reads as work somebody owes.
check(
  "an entry for a binary a build step now installs is caught",
  {
    ...CARRIED,
    "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n',
    "dev/mkosi/mkosi.build.d/09-ghost.sh.chroot":
      'install -Dm755 "$OUT" "$DESTDIR/usr/lib/arlen/libexec/arlen-ghost"\n',
  },
  (code, out) => code === 1 && out.includes("build step now installs it"),
  { known: "arlen-ghost" },
);

check(
  "an entry for a binary nothing spawns any more is caught",
  { ...CARRIED, "apps/probe/src/lib.rs": "// the spawn this entry was written for is gone\n" },
  (code, out) => code === 1 && out.includes("nothing spawns it"),
  { known: "arlen-ghost" },
);

// And the case it must stay quiet on, or every entry doing its job is a finding.
check(
  "a live entry stays quiet",
  { ...CARRIED, "apps/probe/src/lib.rs": 'std::process::Command::new("arlen-ghost").spawn();\n' },
  (code, out) => code === 0 && out.includes("planted"),
  { known: "arlen-ghost" },
);

// A tree with no Rust in it is a walk that reached nothing, and this used to
// answer "OK: 0 first-party program(s) spawned by name" over it.
check(
  "a tree with no Rust source refuses rather than passing",
  { "README.md": "no rust here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("a missing binary is caught, a legitimate install is not, and a stale entry is caught too");
