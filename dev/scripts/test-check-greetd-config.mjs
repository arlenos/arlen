#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for `check-greetd-config.py`: build a small image tree, break one thing
// at a time, and watch the check fail on each. A check nobody has seen fail is a
// check nobody has seen.
//
// The delete helper here is the one from `test-check-plugin-command-grants.mjs`,
// and it is not decoration. On 27 August a control in this directory passed the
// REPOSITORY ROOT to a cleanup that ended in `rmSync(dir, { recursive: true })`
// and deleted the tree, `.git` included. Every fixture is minted through `mint`,
// which records the path; `cleanup` refuses anything it did not mint.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHECK = join(HERE, "check-greetd-config.py");
const REPO = join(HERE, "..", "..");


function write(root, rel, contents) {
  const path = join(root, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, contents);
}

// A tree the check considers well-formed: one session, a staged binary, a created
// user, a PAM file. Each case below removes exactly one of those.
function wellFormed({ config, step, postinst, pam, packages } = {}) {
  const root = mint("greetd-config-");
  write(
    root,
    "dev/mkosi/mkosi.extra/etc/greetd/config.toml",
    config ??
      '[terminal]\nvt = 1\n[default_session]\ncommand = "/usr/bin/cage -s -- /usr/bin/arlen-greeter"\nuser = "_greetd"\n',
  );
  write(
    root,
    "dev/mkosi/mkosi.build.d/04r-greeter.sh.chroot",
    step ?? 'install -Dm755 "$out" "$DESTDIR/usr/bin/arlen-greeter"\n',
  );
  write(
    root,
    "dev/mkosi/mkosi.postinst",
    postinst ?? "useradd --create-home --groups video arlen\n",
  );
  write(root, "dev/mkosi/mkosi.conf", packages ?? "Packages=\n        greetd\n        cage\n\n[Other]\n");
  for (const f of pam ?? ["greetd-greeter"]) {
    write(root, `dev/mkosi/mkosi.extra/etc/pam.d/${f}`, "auth required pam_permit.so\n");
  }
  return root;
}

function gateOn(root) {
  try {
    const out = execFileSync("python3", [CHECK, root], { encoding: "utf-8" });
    return { code: 0, out };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

const cases = [
  [
    "the repository as it stands passes",
    () => REPO,
    (code) => code === 0,
    false,
  ],
  [
    "a well-formed fixture passes",
    () => wellFormed(),
    (code) => code === 0,
    true,
  ],
  [
    "a command no step installs and no package provides is caught",
    () =>
      wellFormed({
        config:
          '[terminal]\nvt = 1\n[default_session]\ncommand = "/usr/bin/regreet"\nuser = "_greetd"\n',
      }),
    (code, out) => code === 1 && out.includes("/usr/bin/regreet"),
    true,
  ],
  [
    "a user nothing creates is caught",
    () =>
      wellFormed({
        config:
          '[terminal]\nvt = 1\n[default_session]\ncommand = "/usr/bin/cage -s -- /usr/bin/arlen-greeter"\nuser = "lightdm"\n',
      }),
    (code, out) => code === 1 && out.includes("lightdm"),
    true,
  ],
  [
    "a PAM service with no file is caught, including the one greetd defaults to",
    () => wellFormed({ pam: ["greetd"] }),
    (code, out) => code === 1 && out.includes("greetd-greeter"),
    true,
  ],
  [
    "an explicit service naming a missing file is caught",
    () =>
      wellFormed({
        config:
          '[terminal]\nvt = 1\n[default_session]\ncommand = "/usr/bin/cage -s -- /usr/bin/arlen-greeter"\nuser = "_greetd"\nservice = "greetd-autologin"\n',
      }),
    (code, out) => code === 1 && out.includes("greetd-autologin"),
    true,
  ],
  [
    "a package that is only a comment does not count as installed",
    () =>
      wellFormed({
        packages: "Packages=\n        greetd\n        # cage\n\n[Other]\n",
      }),
    (code, out) => code === 1 && out.includes("/usr/bin/cage"),
    true,
  ],
  [
    "a missing config is a failure rather than a pass with nothing read",
    () => {
      const root = mint("greetd-config-empty-");
      mkdirSync(join(root, "dev/mkosi"), { recursive: true });
      return root;
    },
    (code, out) => code === 1 && out.includes("missing"),
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
    console.log(`     exit ${code}\n     ${out.trim().split("\n").join("\n     ")}`);
  }
}

if (failed) {
  console.log(`\n${failed} case(s) failed`);
  process.exit(1);
}
console.log(`\nall ${cases.length} cases behaved`);
