#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for `covered` in check-writable-paths-exist.py - the decision "does
// anything create this ReadWritePaths entry before the unit's namespace is set
// up".
//
// This gate has now been wrong TWICE, both times permissively, and both times
// the same way: it accepted an ancestor as proof about a descendant. First a
// tmpfiles entry for `/var/lib` was read as covering `/var/lib/arlen/identity`,
// so it passed `permission-helper.service`, the very unit it was written for.
// Then `%h` sat in the guaranteed list as a PREFIX, so it passed
// `installd.service`, whose `%h/.local/share/applications` killed the unit at the
// NAMESPACE step on the 19 Aug image.
//
// Nobody caught either from reading the script, because both readings sound
// reasonable. What catches them is a case that states the answer independently.
// That is what this file is, and its absence is why the gate shipped wrong twice.
//
// Run: node dev/scripts/test-check-writable-paths-exist.mjs

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GATE = join(ROOT, "dev/scripts/check-writable-paths-exist.py");
let failures = 0;

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) {
    failures++;
    if (detail) console.log(`       ${detail}`);
  }
}

/// Ask the predicate one question, with an explicit creator set, so the answer
/// does not depend on the real tree.
function covered(path, creators) {
  const py = `
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("g", ${JSON.stringify(GATE)})
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
args = json.load(sys.stdin)
print("YES" if m.covered(args["path"], set(args["creators"])) else "NO")
`;
  const r = spawnSync("python3", ["-c", py], {
    input: JSON.stringify({ path, creators }),
    encoding: "utf8",
  });
  return (r.stdout || "").trim() === "YES";
}

console.log("check-writable-paths-exist (covered):");

check(
  "a path with its own creator is covered",
  covered("/var/lib/arlen/identity", ["/var/lib/arlen/identity"]),
);

check(
  "a creator for the PARENT does not cover the child",
  !covered("/var/lib/arlen/identity", ["/var/lib"]),
  "the first version accepted this and passed permission-helper.service",
);

check(
  "%h alone is guaranteed - the home directory exists",
  covered("%h", []),
);

check(
  "a path INSIDE %h is not guaranteed",
  !covered("%h/.local/share/applications", []),
  "the second version accepted this and passed installd.service",
);

check(
  "%h expands, so a tmpfiles entry spelling the home out counts",
  covered("%h/.local/share/arlen", ["/home/arlen/.local/share/arlen"]),
  "without expansion the specifier matches nothing and working units read as broken",
);

check(
  "%t and /run/user/%U are the same directory written two ways",
  covered("%t/arlen", ["/run/user/%U/arlen"]),
);

check(
  "a trailing slash does not change the answer",
  covered("/var/lib/arlen/identity/", ["/var/lib/arlen/identity"]),
);

// End to end. The gate must be green on the tree, and it must go red if the one
// path the 19 Aug boot died on loses its creator - the regression that matters.
{
  const r = spawnSync("python3", [GATE], { encoding: "utf8" });
  check("the gate passes on the tree as it stands", r.status === 0, (r.stderr || "").trim());

  const conf = join(ROOT, "dev/mkosi/mkosi.extra/etc/tmpfiles.d/arlen-home.conf");
  const original = spawnSync("cat", [conf], { encoding: "utf8" }).stdout;
  const withoutIt = original
    .split("\n")
    .filter((l) => !l.startsWith("d /home/arlen/.local/share/applications"))
    .join("\n");
  spawnSync("cp", [conf, `${conf}.bak`]);
  try {
    spawnSync("tee", [conf], { input: withoutIt, encoding: "utf8", stdio: ["pipe", "ignore", "ignore"] });
    const red = spawnSync("python3", [GATE], { encoding: "utf8" });
    check(
      "removing the applications dir turns the gate red again",
      red.status === 1 && (red.stderr || "").includes("applications"),
      "the boot failure this fixes would not be caught",
    );
  } finally {
    spawnSync("mv", [`${conf}.bak`, conf]);
  }
}

console.log(
  failures
    ? `\n${failures} case(s) failed`
    : "\nan ancestor is not a creator, and a specifier is compared as the path it means",
);
process.exit(failures ? 1 : 0);
