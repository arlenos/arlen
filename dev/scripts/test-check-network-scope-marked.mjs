// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-network-scope-marked.
//
// Both directions matter. A new profile that takes the whole network without
// saying why is the case the check exists for - it would drop out of the grep
// that becomes the work list. A marker left behind on a profile that no longer
// grants the network is the opposite failure: a note explaining a grant that is
// not there, which is the same lie in reverse and would pad the work list.
//
// Run: node dev/scripts/test-check-network-scope-marked.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-network-scope-marked.py");
const failures = [];

const NOTE = "# NETWORK-SCOPE-PENDING: the scoped form is not enforceable yet.\n";

function tree(files, imageFiles = {}) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-netscope-"));
  mkdirSync(join(dir, "sdk/permissions/profiles"), { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, "sdk/permissions/profiles", `${name}.toml`), body);
  }
  // The image's own set, checked since 27 August: it is the one that actually
  // runs, and a wide grant added there used to carry no marker and go unrecorded.
  const imageDir = join(dir, "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000");
  mkdirSync(imageDir, { recursive: true });
  for (const [name, body] of Object.entries(imageFiles)) {
    writeFileSync(join(imageDir, `${name}.toml`), body);
  }
  return dir;
}

const profile = (id, { net = "none", note = false } = {}) =>
  (note ? NOTE : "") +
  `[info]\napp_id = "${id}"\ntier = "third-party"\n` +
  (net === "all"
    ? `\n[network]\nallow_all = true\n`
    : net === "scoped"
      ? `\n[network]\nallowed_domains = ["api.example.com"]\n`
      : "");

const run = (dir) => {
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

function check(name, ok, detail) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, detail });
}

console.log("check-network-scope-marked:");

// The planted defect: whole network, no note.
let d = tree({ silent: profile("silent", { net: "all" }) });
let r = run(d);
check(
  "a wide network grant with no note is reported",
  r.code === 1 && /silent/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// The same grant, explained.
d = tree({ explained: profile("explained", { net: "all", note: true }) });
r = run(d);
check("a wide grant that says why passes", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// A scoped grant is already the narrow thing and needs no note.
d = tree({ scoped: profile("scoped", { net: "scoped" }) });
r = run(d);
check("a scoped grant needs no note", r.code === 0, `exit=${r.code} out=${r.out}`);
rmSync(d, { recursive: true, force: true });

// A note outliving its grant pads the work list.
d = tree({ leftover: profile("leftover", { net: "none", note: true }) });
r = run(d);
check(
  "a note left on a profile with no network grant is reported",
  r.code === 1 && /leftover/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

// Reading nothing is not passing.
d = mkdtempSync(join(tmpdir(), "arlen-netscope-empty-"));
r = run(d);
check(
  "a tree with no profiles refuses rather than passing",
  r.code === 2 && /NOTHING WAS READ/.test(r.out),
  `exit=${r.code} out=${r.out}`,
);
rmSync(d, { recursive: true, force: true });

for (const f of failures) console.error(`\n--- ${f.name}\n${f.detail}`);
{
  // The same unexplained grant, one directory over.
  const dir = tree({ "ok-app": profile("ok-app") }, { "dev.arlen.thing": profile("dev.arlen.thing", { net: "all" }) });
  const r = run(dir);
  check(
    "a wide grant in an image profile is caught too",
    r.code === 1 && /dev\.arlen\.thing/.test(r.out),
    `exit ${r.code}: ${r.out}`,
  );
  rmSync(dir, { recursive: true, force: true });
}

if (failures.length) process.exit(1);
console.log("an unexplained wide grant is caught, a stale note too, and an empty read refuses");
