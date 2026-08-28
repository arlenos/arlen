// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the unit-identity drift check: plant each way the
// hand-kept table can fall out of step with the shipped units, and watch it refuse.
//
// The check exists because the table is hand-kept ON PURPOSE - the cgroup route is
// sound precisely because the kernel guarantees the unit name while we choose the
// app_id - and a hand-kept table drifts quietly in the worst direction: a new
// daemon ships, nothing maps it, and it authenticates as nobody, which from
// outside looks exactly like a daemon refused for a good reason.

import { writeFileSync, mkdirSync, cpSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-unit-identity.py");
const UNITS = "dev/mkosi/mkosi.extra/usr/lib/systemd/system";
const USER_UNITS = "dev/mkosi/mkosi.extra/usr/lib/systemd/user";
const RESOLVER = "sdk/permissions/src/unit_identity.rs";
const PATH_RESOLVER = "sdk/permissions/src/identity.rs";

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

// Run the check against a COPY of the tree, so a planted defect never touches the
// real one. The check resolves its paths from its own location, so it travels.
function withTree(mutate) {
  const dir = mint("unit-identity-");
  mkdirSync(join(dir, UNITS), { recursive: true });
  mkdirSync(join(dir, USER_UNITS), { recursive: true });
  mkdirSync(join(dir, dirname(RESOLVER)), { recursive: true });
  mkdirSync(join(dir, "dev/scripts"), { recursive: true });
  cpSync(join(ROOT, UNITS), join(dir, UNITS), { recursive: true });
  cpSync(join(ROOT, USER_UNITS), join(dir, USER_UNITS), { recursive: true });
  cpSync(join(ROOT, RESOLVER), join(dir, RESOLVER));
  // The PATH resolver too, since the check now also asks whether each unit's
  // binary is one `path_to_app_id` can name. Without it that half skips, and the
  // case planting an unnameable binary would pass on the OTHER check's message.
  cpSync(join(ROOT, PATH_RESOLVER), join(dir, PATH_RESOLVER));
  cpSync(CHECK, join(dir, "dev/scripts/check-unit-identity.py"));
  // The verify build phase, because the check now reads it: a table entry for a
  // unit that only exists on verify images is excused only if that phase really
  // writes the unit. Without the file in the copy, every run here would report
  // the exception as stale and the whole control would go red for the wrong
  // reason.
  // The whole phase directory, not one named file: the check allows a table entry
  // for any unit a build phase writes, and there are two such units now (the
  // verify probe and the eBPF sensor). Copying them by name meant adding a line
  // here every time, and forgetting turned the control red for a reason that had
  // nothing to do with what it was testing.
  cpSync(
    join(ROOT, "dev/mkosi/mkosi.build.d"),
    join(dir, "dev/mkosi/mkosi.build.d"),
    { recursive: true }
  );
  mutate(dir);
  const r = spawnSync("python3", [join(dir, "dev/scripts/check-unit-identity.py")], {
    encoding: "utf8",
  });
  cleanup(dir);
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

// The tree as it stands passes, so a failure below is the planted defect and not
// a pre-existing one.
{
  const r = withTree(() => {});
  check("the tree as it stands passes", r.code === 0);
  check("and it says how many units are named", r.out.includes("named by the cgroup resolver"));
}
{
  // The verify-only exception must EXPIRE. If the phase stops writing the unit,
  // the entry is stale in exactly the way this check exists to catch, and being
  // marked verify-only must not make it permanent.
  const r = withTree((dir) => {
    writeFileSync(
      join(dir, "dev/mkosi/mkosi.build.d/09-verify-probes.sh.chroot"),
      "#!/bin/sh\necho the phase no longer writes any probe unit\n"
    );
  });
  check("a verify-only entry whose phase stopped writing it is caught", r.code === 1);
  check("and the message says the phase does not write it", r.out.includes("does not write it"));
}

// A new system daemon ships and nobody maps it. This is the quiet one.
{
  const r = withTree((dir) => {
    writeFileSync(
      join(dir, UNITS, "arlen-newthing.service"),
      "[Service]\nExecStart=/usr/bin/arlen-newthing\n",
    );
  });
  check("a shipped unit with no entry is refused", r.code === 1);
  check("and the message names the unit", r.out.includes("arlen-newthing.service"));
}

// Both fixtures below inject into the SYSTEM table, so they anchor on a unit
// that is unambiguously in it. They used to anchor on `arlen-timeline.service`,
// which moved to the per-user table on 15 Aug - the replace still matched, the
// injected entry landed in the wrong table, and both cases failed on the message
// rather than on the behaviour. Anchoring on the event bus, which is a system
// unit by nature, is what keeps that from happening again.
// A line in the SYSTEM table to hang the mutations on. It has been re-anchored
// twice now - first off `arlen-timeline`, then off `arlen-event-bus` - because
// both moved to the user table when their daemons moved per-user, and a control
// anchored on a line that moves goes red while nothing is wrong. Anchor on the
// entry least likely to move, and expect to move this again if the config broker
// ever follows the others.
const SYSTEM_ANCHOR = '("arlen-config-broker.service", "config-broker"),';

// An entry for a unit that no longer ships: coverage that cannot fire.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace(
        SYSTEM_ANCHOR,
        `${SYSTEM_ANCHOR}\n    ("arlen-gone.service", "gone"),`,
      ),
    );
  });
  check("an entry for a unit that does not ship is refused", r.code === 1);
  check("and the message says it cannot fire", r.out.includes("cannot fire"));
}

// Both excused and mapped - the state the check caught on its very first run,
// when this file's author had done exactly that to arlen-llama.service.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace(
        SYSTEM_ANCHOR,
        `${SYSTEM_ANCHOR}\n    ("arlen-llama.service", "llama"),`,
      ),
    );
  });
  check("a unit both excused and mapped is refused", r.code === 1);
  check("and the message says the excuse outlived its reason", r.out.includes("outlived"));
}

// The per-user table, whose extra property is that it must AGREE with the binary
// route. A table that disagrees names one daemon two ways, and the lookup under
// the wrong one answers "no grants" - which reads as security working.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace('("arlen-notifyd.service", "notifyd")', '("arlen-notifyd.service", "notify-daemon")'),
    );
  });
  check("a per-user id that disagrees with its binary is refused", r.code === 1);
  check("and the message says how the miss would present", r.out.includes("silently misses"));
}

// The one deviation is recorded WITH its reason, so reverting it to the derived
// name is what fails - not the deviation itself.
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace('("arlen-ai-engine-daemon.service", "ai-agent")', '("arlen-ai-engine-daemon.service", "ai-engine-daemon")'),
    );
  });
  check("deriving the engine daemon's id from its unit name is refused", r.code === 1);
  check("and the recorded reason is quoted back", r.out.includes("nothing else in the system uses that id"));
}

// A new per-user daemon nobody named: the launcher would register nothing and its
// peers would be refused with no way to tell that from a policy decision.
{
  const r = withTree((dir) => {
    writeFileSync(
      join(dir, USER_UNITS, "arlen-newd.service"),
      "[Service]\nExecStart=/usr/lib/arlen/libexec/arlen-newd\n",
    );
  });
  check("a shipped per-user unit with no entry is refused", r.code === 1);
  check("and the message says its peers would be refused", r.out.includes("peers would"));
}

// A gap that is NAMED must be closed properly: an entry left beside the
// unnameable listing is caught rather than silently preferred. (The undo signer
// used to be this case and now resolves, which is why the check's own
// excuse-outlived-its-reason rule matters - the reason went false before the
// listing did.)
{
  const r = withTree((dir) => {
    const p = join(dir, RESOLVER);
    writeFileSync(
      p,
      readFileSync(p, "utf8").replace('("arlen-undod.service", "undod")', '("arlen-undod.service", "undod"),\n    ("arlen-store-backend.service", "store-backend")'),
    );
  });
  check("naming a unit listed as unnameable is refused", r.code === 1);
  check("and the message says the entry was left behind", r.out.includes("left behind"));
}

{
  // A unit whose binary matches no identity rule. The table can still name it -
  // the supervisor stamps the cgroup - so the checks above are happy; this is the
  // OTHER route, the one a socket peer is identified by.
  const r = withTree((dir) => {
    const unit = join(dir, USER_UNITS, "arlen-clockd.service");
    writeFileSync(
      unit,
      readFileSync(unit, "utf8").replace(
        /^ExecStart=.*$/m,
        "ExecStart=/usr/lib/arlen/libexec/nothing-names-this"
      )
    );
  });
  check("a unit whose binary no identity rule names is refused", r.code === 1);
  check("and the message says it resolves as UnknownBinary", r.out.includes("UnknownBinary"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nevery drift is caught");
process.exit(failures ? 1 : 0);
