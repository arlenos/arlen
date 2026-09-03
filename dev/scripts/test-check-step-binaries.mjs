#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-step-binaries.py`: the search name in a build phase has to
// be a binary some crate writes. Planted from the real case - a crate called
// `arlen-pdf-app` and a phase looking for `arlen-pdf`.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-step-binaries.py");
const REPO = join(HERE, "..", "..");

// A phase that enters the shared build cache, with or without creating it. The
// third rule's fixture: `05b-pi` did exactly this and killed a whole build.
function cacheTree({ makes }) {
  const root = mint("step-cache-");
  // The gate refuses a tree it could read nothing from, so the fixture carries a
  // crate even though this rule does not look at one.
  const c = join(root, "apps/demo/src-tauri/Cargo.toml");
  mkdirSync(dirname(c), { recursive: true });
  writeFileSync(c, `[package]\nname = "arlen-demo-app"\nversion = "0.1.0"\n`);
  const p = join(root, "dev/mkosi/mkosi.build.d/05z-cache.sh.chroot");
  mkdirSync(dirname(p), { recursive: true });
  writeFileSync(
    p,
    `#!/bin/sh\nset -eu\ncache="\${BUILDDIR:-/var/tmp/arlen-build}"\n` +
      (makes ? `mkdir -p "$cache"\n` : "") +
      `cd "$cache"\ncurl -fsSLO https://example.invalid/x.tar.xz\n`,
  );
  return root;
}

function tree({ crate, search, install }) {
  const root = mint("step-binaries-");
  const write = (rel, body) => {
    const p = join(root, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  };
  write("apps/demo/src-tauri/Cargo.toml", `[package]\nname = "${crate}"\nversion = "0.1.0"\n`);
  const line = install
    ? `install -Dm644 "$SRCDIR/arlen/${install}" "$DESTDIR/usr/share/x"\n`
    : "";
  write(
    "dev/mkosi/mkosi.build.d/04z-demo.sh.chroot",
    `#!/bin/sh\nout=$(find "$CARGO_TARGET_DIR" -type f -path '*/release/${search}' | head -1)\n${line}`,
  );
  return root;
}

function gateOn(root) {
  try {
    return { code: 0, out: execFileSync("python3", [CHECK, root], { encoding: "utf-8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const cases = [
  ["the repository as it stands passes", () => REPO, (code) => code === 0, false],
  [
    "a phase looking for the name its crate writes passes",
    () => tree({ crate: "arlen-demo-app", search: "arlen-demo-app" }),
    (code) => code === 0,
    true,
  ],
  [
    "a phase entering the build cache without creating it is caught",
    () => cacheTree({ makes: false }),
    (code, out) => code === 1 && out.includes("build cache"),
    true,
  ],
  [
    "the same phase creating it first passes",
    () => cacheTree({ makes: true }),
    (code) => code === 0,
    true,
  ],
  [
    "the real case is caught: crate arlen-pdf-app, phase looking for arlen-pdf",
    () => tree({ crate: "arlen-pdf-app", search: "arlen-pdf" }),
    (code, out) => code === 1 && out.includes("arlen-pdf"),
    true,
  ],
  [
    "a file the phase installs from the checkout must be there",
    () =>
      tree({
        crate: "arlen-demo-app",
        search: "arlen-demo-app",
        install: "apps/demo/dist/moved-away.desktop",
      }),
    (code, out) => code === 1 && out.includes("moved-away.desktop"),
    true,
  ],
  [
    "and one that is there passes",
    () =>
      tree({
        crate: "arlen-demo-app",
        search: "arlen-demo-app",
        install: "apps/demo/src-tauri/Cargo.toml",
      }),
    (code) => code === 0,
    true,
  ],
  [
    "no crate at all refuses rather than passing with nothing read",
    () => {
      const root = mint("step-binaries-empty-");
      mkdirSync(join(root, "dev/mkosi/mkosi.build.d"), { recursive: true });
      return root;
    },
    (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
    true,
  ],
];

let failed = 0;
for (const [name, build, expect, disposable] of cases) {
  const root = build();
  const { code, out } = gateOn(root);
  if (disposable) cleanup(root);
  const ok = expect(code, out);
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failed += 1;
    console.log(`     exit ${code}\n     ${out.trim().split("\n").slice(0, 2).join("\n     ")}`);
  }
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log(`\nall ${cases.length} cases behaved`);
