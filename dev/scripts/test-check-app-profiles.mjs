// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The second authorisation gate proved rather than assumed. A missing permission
// profile is a REFUSED launch - `arlen-run` exits 65 and never falls back to
// unconfined - so an app the image installs without one is an app that stops
// working the day the confinement flag goes true. If this check were blind, that
// would be discovered by the rollout instead of by CI.
//
// Run: node dev/scripts/test-check-app-profiles.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-app-profiles.py");

const STEPS = "dev/mkosi/mkosi.build.d";
const PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000";

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-appprof-"));
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

const STEP = 'install -Dm755 "$OUT" "$DESTDIR/usr/lib/arlen/apps/dev.arlen.probe/bin/arlen-probe"\n';
const PROFILE = '[info]\napp_id = "dev.arlen.probe"\ntier = "first-party"\n\n[graph]\nread = []\n';

console.log("check-app-profiles:");

check(
  "an installed app with no profile is caught",
  { [`${STEPS}/09-probe.sh.chroot`]: STEP },
  (code, out) => code === 1 && out.includes("dev.arlen.probe"),
);

check(
  "the same app with a profile passes",
  {
    [`${STEPS}/09-probe.sh.chroot`]: STEP,
    [`${PROFILES}/dev.arlen.probe.toml`]: PROFILE,
  },
  (code) => code === 0,
);

// The drift the file manager's own header warned about: a profile that parses
// and ships, but names a different app. The loader resolves by id, so it is not
// this app's profile at all - and the launch refuses at run time, where the
// message is about a missing profile rather than about the wrong name inside a
// present one.
check(
  "a profile naming a different app is not that app's profile",
  {
    [`${STEPS}/09-probe.sh.chroot`]: STEP,
    [`${PROFILES}/dev.arlen.probe.toml`]:
      '[info]\napp_id = "dev.arlen.somethingelse"\ntier = "first-party"\n',
  },
  (code, out) => code === 1 && out.includes("dev.arlen.probe"),
);

// A profile that does not parse ships as a file and refuses as a launch, which
// is the worst of both: present in the tree, absent in effect.
check(
  "a profile that does not parse is caught",
  {
    [`${STEPS}/09-probe.sh.chroot`]: STEP,
    [`${PROFILES}/dev.arlen.probe.toml`]: "[info\napp_id = broken\n",
  },
  (code, out) => code === 1 && out.includes("dev.arlen.probe"),
);

// And the fail-closed shape the gate states for itself: no install steps at all
// means the layout moved, not that every app is fine.
check(
  "an empty tree is a moved layout, not a pass",
  { "README.md": "nothing here\n" },
  (code, out) => code === 1 && out.includes("layout moved"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("a missing, misnamed or unparsable profile is caught; a correct one passes");
