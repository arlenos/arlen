#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for `installs_unit` in check-shipped-units.py - the decision "does
// this build-step text PLACE a unit, or merely mention it".
//
// The case that forced the split is the first one. A build phase said in prose
// that it deliberately does NOT ship `arlen-trash-cleanup.service`, and the gate,
// which substring-matched the whole script, found the name in that sentence and
// reported the unit as deployed. Documenting a deliberate omission is behaviour
// this tree asks for everywhere else, so the checker had to stop punishing it.
//
// The second case matters more: the new rule is NARROWER, and a narrow rule can
// under-report. A unit a phase really installs must still be seen, or a shipped
// unit sits on the image while the list calls it deferred.
//
// The predicate is exercised directly rather than through the gate, because the
// gate reads the real tree and carries hand-kept lists that a synthetic fixture
// cannot satisfy - running it on a temp tree buries the answer in unrelated
// findings, which is exactly what the first version of this file did.
//
// Run: node dev/scripts/test-check-shipped-units.mjs

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// Load the gate by path (its filename is not an importable module name) and ask
/// the predicate one question.
function installs(script, unit) {
  const py = `
import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location("g", ${JSON.stringify(join(ROOT, "dev/scripts/check-shipped-units.py"))})
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
print("YES" if m.installs_unit(sys.stdin.read(), ${JSON.stringify(unit)}) else "NO")
`;
  const r = spawnSync("python3", ["-c", py], { input: script, encoding: "utf8" });
  return (r.stdout || "").trim() === "YES";
}

console.log("check-shipped-units (installs_unit):");

check(
  "a unit named only in a comment is not installed",
  !installs(
    "#!/bin/sh\n# Deliberately not shipped: thing.service deletes user data on a timer.\necho hi\n",
    "thing.service",
  ),
);

check(
  "a unit an install line places IS installed",
  installs(
    '#!/bin/sh\ninstall -Dm644 "$src/dist/thing.service" "$DESTDIR/usr/lib/systemd/system/thing.service"\n',
    "thing.service",
  ),
);

check(
  "a unit placed with cp is installed too",
  installs('#!/bin/sh\ncp "$src/dist/thing.service" "$DESTDIR/usr/lib/systemd/system/"\n', "thing.service"),
);

check(
  "a commented-out install line does not count, however much it looks like one",
  !installs(
    '#!/bin/sh\n# install -Dm644 "$src/dist/thing.service" "$DESTDIR/usr/lib/systemd/system/thing.service"\n',
    "thing.service",
  ),
);

check(
  "an install line for a DIFFERENT unit does not count",
  !installs(
    '#!/bin/sh\ninstall -Dm644 "$src/dist/other.service" "$DESTDIR/usr/lib/systemd/system/other.service"\n',
    "thing.service",
  ),
);

// The real phase, as written, so the regression is pinned against the actual file
// rather than a paraphrase of it.
{
  const real = spawnSync("cat", [join(ROOT, "dev/mkosi/mkosi.build.d/08v-installd.sh.chroot")], {
    encoding: "utf8",
  }).stdout;
  check(
    "the real install phase ships installd.service and not the trash timer",
    installs(real, "installd.service") && !installs(real, "arlen-trash-cleanup.service"),
  );
}

console.log(
  failures ? `\n${failures} case(s) failed` : "\nprose cannot ship a unit, and an install line cannot hide one",
);
process.exit(failures ? 1 : 0);
