// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The inventory this gate keeps is prose, and prose rots silently. These cases
// are the two ways it rots, made to happen on purpose.
//
// Note what is NOT tested: whether a switch is in the right state. That is a
// judgement the gate deliberately refuses to make, so asserting it here would be
// asserting it in the one place nobody would look for it.
//
// Run: node dev/scripts/test-check-inert-switches.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-inert-switches.py");

const failures = [];

// A tree the gate can read: a git repo (it shells out to `git grep`), one Rust
// file naming the envs, and an image tree whose unit sets one of them.
function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-inert-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  for (const args of [["init", "-q"], ["add", "-A"]]) {
    spawnSync("git", args, { cwd: dir, encoding: "utf8" });
  }
  return dir;
}

const UNIT = (env) => `[Service]\n${env}\n`;

// Every switch the real inventory names, so a case fails for the reason it is
// about and not because some other entry went missing in the fixture.
const READS = `
fn _reads() {
    std::env::var("ARLEN_OWNER_USER").ok();
    std::env::var("ARLEN_CONFIG_BROKER_IDENTITY_UID").ok();
    std::env::var("ARLEN_EVENT_BUS_ENFORCE").ok();
    std::env::var_os("ARLEN_CAPSULE_REQUIRE_FENCE");
    std::env::var("ARLEN_STAMPED_IDENTITY").ok();
}
`;

const U = "dev/mkosi/mkosi.extra/usr/lib/systemd/system";
const BASE = {
  "src/lib.rs": READS,
  // ARLEN_OWNER_USER is recorded UNSET since 15 Aug (the graph daemon's socket
  // went per-user, so there is no shared socket for it to guard) - so the base
  // fixture must NOT set it, or every case fails on that mismatch instead of on
  // what it is testing. It used to be set here, back when the switch was.
  // Recorded as `set`, and only two real units carry it - the check sees the
  // tree, not the rollout, so one unit here is the whole of that state.
  [`${U}/arlen-auditd.service`]: UNIT("Environment=ARLEN_STAMPED_IDENTITY=enforce"),
  // Recorded as `set` since the 12 Aug flip: the bus scopes are enforced in the
  // release image, not shadowed. This fixture moved WITH the switch, which is the
  // rule the gate exists for - a flip and its written justification land together.
  [`${U}/arlen-event-bus.service.d/10-enforce.conf`]: UNIT(
    "Environment=ARLEN_EVENT_BUS_ENFORCE=1",
  ),
  // Recorded as `set` since 13 Aug, and NOT by an `Environment=` line. sysusers
  // allocates the broker's uid, so no file can carry the value; it is derived at
  // boot from the owner of the broker's socket. The fixture carries the generator
  // that derives it, because what this gate reads has to be the shape the image
  // really has - a fixture that only ever models `Environment=` would pass a tree
  // where the switch is set by something else, or fail one where it is.
  "dev/mkosi/mkosi.extra/usr/lib/systemd/user-environment-generators/50-arlen-identity-broker":
    'uid=$(stat -c %u "$sock") || exit 0\necho "ARLEN_CONFIG_BROKER_IDENTITY_UID=$uid"\n',
};

function check(name, files, expect) {
  const dir = tree(files);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  rmSync(dir, { recursive: true, force: true });
}

console.log("check-inert-switches:");

// The last of the sixteen. This gate exists because a protection recorded as on
// and actually off is invisible, and it had that shape itself: pointed at a tree
// with no image it printed "no dev/mkosi/mkosi.extra; nothing to check" and
// exited 0. The image tree is committed, so that is a wrong root, not an image
// nobody has built.
check(
  "a tree with no image is refused rather than skipped",
  tree({ "README.md": "no image here\n" }),
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check("a tree matching every recorded state passes", BASE, (code) => code === 0);

// Rot one: the env is renamed or dropped, and the reason keeps arguing about a
// switch that is not there. An inventory of absent switches reads reassuring.
check(
  "an inventoried env that no source reads any more is caught",
  { ...BASE, "src/lib.rs": READS.replace('std::env::var("ARLEN_EVENT_BUS_ENFORCE").ok();', "") },
  (code, out) => code === 1 && out.includes("ARLEN_EVENT_BUS_ENFORCE"),
);

// Rot two: someone flips a switch and leaves the justification describing the
// old state. The gate has no opinion on the flip, only on the stale prose.
check(
  "switching an off switch on without updating its reason is caught",
  {
    ...BASE,
    [`${U}/arlen-capsuled.service`]: UNIT("Environment=ARLEN_CAPSULE_REQUIRE_FENCE=1"),
  },
  (code, out) => code === 1 && out.includes("recorded as unset"),
);

check(
  "losing a switch that is recorded as on is caught",
  // Overrides the unit that CARRIES the switch. It used to override the graph
  // unit, which carried ARLEN_OWNER_USER until that went per-user; pointing the
  // override at one file and the assertion at a switch in another would pass or
  // fail for reasons unrelated to losing anything.
  { ...BASE, [`${U}/arlen-auditd.service`]: UNIT("Environment=RUST_LOG=info") },
  (code, out) => code === 1 && out.includes("ARLEN_STAMPED_IDENTITY"),
);

// A mention is not a setting: the daemon's own source names every one of these
// envs, and a check that counted mentions would call all four set and never
// fail again.
check(
  "an env only MENTIONED in the image tree does not count as set",
  { ...BASE, [`${U}/README`]: "we could set ARLEN_CAPSULE_REQUIRE_FENCE here one day\n" },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all inert-switch cases passed");
