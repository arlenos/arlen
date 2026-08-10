// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the packaged-unit gate must catch, and what it must leave alone.
//
// The gate exists because a daemon can be built into the image with no unit
// packaged: the binary installs, nothing ever starts it, and that reads as a
// runtime bug rather than a packaging one. It is worth testing for the same
// reason the fixture checker is - on 9 August it reported four missing units of
// which three were impossible, because it read the CRATE off a build line
// instead of the BINARY off the unit it was judging. A gate that cries three
// times too often gets skimmed, and then it stops catching the fourth.
//
// Each case below is a tree the gate is run against, so both directions are
// proven: the shapes it must fail on, and the neighbouring shapes it must pass.
//
// Run: node dev/scripts/test-check-units.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-packaged-units.sh");

const failures = [];

/** Write `files` into a throwaway tree and return its path. */
function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-unit-gate-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

/** Run the gate against `dir`; returns {code, out}. */
function run(dir) {
  // Both streams on every path: `execFileSync`'s return value is stdout alone, so a
  // case asserting on something written to stderr while the gate still exits 0
  // would compare against an empty string and pass for the wrong reason.
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

// A unit whose ExecStart binary the image installs, packaged: nothing to report.
const CANONICAL = `[Unit]
Description=Example
[Service]
ExecStart=/usr/lib/arlen/libexec/arlen-example
[Install]
WantedBy=default.target
`;
const PHASE = `#!/bin/sh
cargo build --release --manifest-path daemons/example/Cargo.toml
install -Dm755 "$CARGO_TARGET_DIR/release/arlen-example" \\
    "$DESTDIR/usr/lib/arlen/libexec/arlen-example"
`;

check(
  "a packaged unit for an installed binary passes",
  tree({
    "daemons/example/dist/example.service": CANONICAL,
    "dev/mkosi/mkosi.build.d/01-example.sh.chroot": PHASE,
    "dev/mkosi/mkosi.extra/usr/lib/systemd/system/example.service": CANONICAL,
  }),
  (code) => code === 0,
);

check(
  "an installed binary with NO packaged unit fails and names it",
  tree({
    "daemons/example/dist/example.service": CANONICAL,
    "dev/mkosi/mkosi.build.d/01-example.sh.chroot": PHASE,
  }),
  (code, out) => code !== 0 && out.includes("UNPACKAGED UNIT: example.service"),
);

// The regression that made the gate untrustworthy: a phase builds one package
// out of a workspace, and the crate's OTHER daemons - whose binaries this image
// has never contained - were demanded too.
check(
  "a unit whose binary is never installed is out of scope, not a failure",
  tree({
    "daemons/example/dist/example.service": CANONICAL,
    "daemons/example/dist/other.service": CANONICAL.replace(
      "arlen-example",
      "arlen-other",
    ),
    "dev/mkosi/mkosi.build.d/01-example.sh.chroot": PHASE,
    "dev/mkosi/mkosi.extra/usr/lib/systemd/system/example.service": CANONICAL,
  }),
  (code, out) => code === 0 && !out.includes("other.service"),
);

// The enablement symlink is named after the unit and would satisfy a naive
// existence check on its own, leaving systemd trying to start a unit that is not
// there - worse than no gate.
check(
  "a dangling .wants symlink does not count as the packaged unit",
  tree({
    "daemons/example/dist/example.service": CANONICAL,
    "dev/mkosi/mkosi.build.d/01-example.sh.chroot": PHASE,
    "dev/mkosi/mkosi.extra/usr/lib/systemd/system/default.target.wants/example.service":
      CANONICAL,
  }),
  (code, out) => code !== 0 && out.includes("UNPACKAGED UNIT: example.service"),
);

console.log(failures.length ? "\nsome cases regressed" : "\nboth directions hold");
process.exit(failures.length ? 1 : 0);
