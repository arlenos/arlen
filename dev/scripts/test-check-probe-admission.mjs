// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for check-probe-admission. Each of the three refusals is
// planted, because a gate whose failing case has never been seen is a gate nobody
// has reason to trust - and this one is the half that keeps a per-variant policy
// from quietly becoming a permanent widening.

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "../..");
const GATE = join(HERE, "check-probe-admission.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// A tree with the two things the gate reads: the identity constants and the
// mkosi build phases.
function tree({ surfaces = ['"dev.arlen.harness"'], phases = {}, extraTree = null }) {
  const dir = mkdtempSync(join(tmpdir(), "probe-admission-"));
  const perms = join(dir, "sdk/permissions/src");
  mkdirSync(perms, { recursive: true });
  writeFileSync(
    join(perms, "identity.rs"),
    `const USER_SURFACES: &[&str] = &[${surfaces.join(", ")}];\n`
  );
  const ph = join(dir, "dev/mkosi/mkosi.build.d");
  mkdirSync(ph, { recursive: true });
  for (const [name, body] of Object.entries(phases)) {
    writeFileSync(join(ph, name), body);
  }
  if (extraTree) {
    const p = join(dir, "dev/mkosi/mkosi.extra", extraTree);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, "dogfood\n");
  }
  return dir;
}

function run(dir) {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const STAGES = 'cat > "$DESTDIR/var/lib/arlen/permissions/user-surfaces.extra"\n';

// The repository as it stands: the probe is admitted by the verify phase only.
check("the repository as it stands passes", run(ROOT).code === 0);

// (1) The widening the whole arrangement exists to avoid.
{
  const d = tree({
    surfaces: ['"dev.arlen.harness"', '"dogfood"'],
    phases: { "09-verify-probes.sh.chroot": STAGES },
  });
  const r = run(d);
  check("a probe compiled into the shipped surfaces is caught", r.code === 1);
  check("and the message names it", r.out.includes("dogfood"));
  rmSync(d, { recursive: true, force: true });
}

// (2) mkosi.extra ships in every image, so the file cannot live there. This is
// the mistake the kg-probe profile's own comment warns about, one file over.
{
  const d = tree({
    phases: { "09-verify-probes.sh.chroot": STAGES },
    extraTree: "var/lib/arlen/permissions/user-surfaces.extra",
  });
  const r = run(d);
  check("the extras file committed under mkosi.extra is caught", r.code === 1);
  check("and the message says it ships in every image", r.out.includes("EVERY image"));
  rmSync(d, { recursive: true, force: true });
}

// (3) A release phase staging it puts the admission on the shipped image.
{
  const d = tree({ phases: { "05-ai.sh.chroot": STAGES } });
  const r = run(d);
  check("a release phase staging the extras file is caught", r.code === 1);
  check("and the message names the phase", r.out.includes("05-ai.sh.chroot"));
  rmSync(d, { recursive: true, force: true });
}

// A verify phase staging it is the intended arrangement and must stay quiet, or
// the gate would forbid the thing it exists to permit.
{
  const d = tree({ phases: { "09-verify-probes.sh.chroot": STAGES } });
  check("the verify phase staging it is not a finding", run(d).code === 0);
  rmSync(d, { recursive: true, force: true });
}

// A tree whose constants cannot be found has not been checked, and saying so
// beats a cheerful pass - the failure this project has hit twice.
{
  const dir = mkdtempSync(join(tmpdir(), "probe-admission-empty-"));
  mkdirSync(join(dir, "sdk/permissions/src"), { recursive: true });
  writeFileSync(join(dir, "sdk/permissions/src/identity.rs"), "// no constants here\n");
  mkdirSync(join(dir, "dev/mkosi/mkosi.build.d"), { recursive: true });
  const r = run(dir);
  check("a tree with no user-surface constants is an error, not a pass", r.code === 2);
  rmSync(dir, { recursive: true, force: true });
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery shape holds");
process.exit(failures ? 1 : 0);
