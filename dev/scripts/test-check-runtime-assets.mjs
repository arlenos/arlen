#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-runtime-assets.py. The defect it was written for was silent -
// the terminal's shell integration was absent from the image and everything degraded
// politely - so a check for it that nobody has watched fail would be the same silence
// wearing a green tick.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-runtime-assets.py");

let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => {
  console.log(`  FAIL ${n}: ${d}`);
  failures += 1;
};

// Asked of the check rather than copied, so the two cannot drift apart.
const excused = execFileSync("python3", [check, "--unprovided"], { encoding: "utf8" })
  .split("\n")
  .filter(Boolean);

/// `reads` is Rust source keyed by file name under apps/; `steps` is build-step text.
function tree(reads, steps) {
  const root = mkdtempSync(join(tmpdir(), "runtime-assets-"));
  mkdirSync(join(root, "apps/x/src"), { recursive: true });
  for (const [name, body] of Object.entries(reads)) {
    writeFileSync(join(root, "apps/x/src", name), body);
  }
  const stepDir = join(root, "dev/mkosi/mkosi.build.d");
  mkdirSync(stepDir, { recursive: true });
  for (const [name, body] of Object.entries(steps)) writeFileSync(join(stepDir, name), body);
  // The excused roots must be read by something, or the check reports them as stale
  // entries - correctly. One file naming them keeps each case about its own subject.
  const refs = excused.map((e, i) => {
    const [tag, name] = e.split(":");
    const dir = tag === "etc" ? "/etc/arlen" : "/usr/share/arlen";
    return `const _E${i}: &str = "${dir}/${name}";`;
  });
  writeFileSync(join(root, "apps/x/src/excused.rs"), refs.join("\n"));
  return root;
}

function run(root) {
  try {
    execFileSync("python3", [check, root], { encoding: "utf8" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

// 1. The defect: code names a path, no step provides it.
{
  const root = tree(
    { "lib.rs": 'const D: &str = "/usr/share/arlen/terminal/zdotdir";' },
    { "04-x.sh.chroot": "#!/bin/sh\ninstall -Dm755 x $DESTDIR/usr/bin/x\n" },
  );
  const rc = run(root);
  rc === 1
    ? ok("a path nothing installs is caught")
    : bad("a path nothing installs is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 2. A step that installs it passes.
{
  const root = tree(
    { "lib.rs": 'const D: &str = "/usr/share/arlen/terminal/zdotdir";' },
    {
      "04-x.sh.chroot":
        '#!/bin/sh\ninstall -Dm644 f "$DESTDIR/usr/share/arlen/terminal/zdotdir/.zshrc"\n',
    },
  );
  const rc = run(root);
  rc === 0
    ? ok("a path the image installs passes")
    : bad("a path the image installs passes", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 3. The prefix trap that fooled a manual sweep: an installed `themes` must not vouch
//    for a read of `theme`. This is the case that makes the check worth more than a
//    grep, and it is the real shape - `wallpapers` (the catalogue) against `wallpaper`
//    (the default manifest). Written with a neutral pair because the real one carries an
//    entry in NOT_PROVIDED, which would excuse it and hide what this case tests.
{
  const root = tree(
    { "lib.rs": 'const D: &str = "/usr/share/arlen/theme/default.toml";' },
    { "08-w.sh.chroot": '#!/bin/sh\ninstall -d "$DESTDIR/usr/share/arlen/themes"\n' },
  );
  const rc = run(root);
  rc === 1
    ? ok("a longer sibling name does not vouch for a shorter one")
    : bad("a longer sibling name does not vouch for a shorter one", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 4. A path named only inside #[cfg(test)] is a test, not a component asking for a file.
{
  const root = tree(
    {
      "lib.rs":
        '#[cfg(test)]\nmod t {\n  const D: &str = "/usr/share/arlen/nonesuch/a.jpeg";\n}\n',
      "real.rs": 'const R: &str = "/usr/share/arlen/terminal/zdotdir";',
    },
    {
      "04-x.sh.chroot":
        '#!/bin/sh\ninstall -Dm644 f "$DESTDIR/usr/share/arlen/terminal/zdotdir/.zshrc"\n',
    },
  );
  const rc = run(root);
  rc === 0
    ? ok("a path only named in a test is not a runtime asset")
    : bad("a path only named in a test is not a runtime asset", `expected 0, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 5. The second tree is read the same way: a trust anchor nothing installs is caught.
{
  const root = tree(
    { "lib.rs": 'const K: &str = "/etc/arlen/nonesuch/k.pub";' },
    { "04-x.sh.chroot": "#!/bin/sh\ninstall -Dm755 x $DESTDIR/usr/bin/x\n" },
  );
  const rc = run(root);
  rc === 1
    ? ok("an /etc/arlen path nothing installs is caught")
    : bad("an /etc/arlen path nothing installs is caught", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 6. A moved layout is not a pass.
{
  const root = mkdtempSync(join(tmpdir(), "runtime-assets-empty-"));
  const rc = run(root);
  rc === 1
    ? ok("a moved layout is not a pass")
    : bad("a moved layout is not a pass", `expected 1, got ${rc}`);
  rmSync(root, { recursive: true, force: true });
}

// 7. The repository itself, which is what the hook runs.
{
  const rc = run(join(here, "..", ".."));
  rc === 0
    ? ok("the repository itself passes")
    : bad("the repository itself passes", `expected 0, got ${rc}`);
}

if (failures) {
  console.log(`\n${failures} case(s) failed`);
  process.exit(1);
}
console.log(
  "an uninstalled path is caught, an installed one passes, a longer sibling name does " +
    "not vouch for a shorter one, and a test-only path is not an asset",
);
