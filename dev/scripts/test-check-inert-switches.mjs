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
  [`${U}/arlen-graph.service`]: UNIT("Environment=ARLEN_OWNER_USER=arlen"),
  // Recorded as `set`, and only two real units carry it - the check sees the
  // tree, not the rollout, so one unit here is the whole of that state.
  [`${U}/arlen-auditd.service`]: UNIT("Environment=ARLEN_STAMPED_IDENTITY=enforce"),
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
  { ...BASE, [`${U}/arlen-event-bus.service`]: UNIT("Environment=ARLEN_EVENT_BUS_ENFORCE=1") },
  (code, out) => code === 1 && out.includes("recorded as unset"),
);

check(
  "losing a switch that is recorded as on is caught",
  { ...BASE, [`${U}/arlen-graph.service`]: UNIT("Environment=ARLEN_DB_PATH=/var/lib/arlen/events.db") },
  (code, out) => code === 1 && out.includes("ARLEN_OWNER_USER"),
);

// A mention is not a setting: the daemon's own source names every one of these
// envs, and a check that counted mentions would call all four set and never
// fail again.
check(
  "an env only MENTIONED in the image tree does not count as set",
  { ...BASE, [`${U}/README`]: "we could set ARLEN_EVENT_BUS_ENFORCE here one day\n" },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all inert-switch cases passed");
