// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-default-handlers.
//
// The gate exists because a `MimeType=` line looks like an association and is
// not one: the launch service reads only `[Default Applications]`. So the cases
// that matter are the three ways the two sides drift apart, plus the refusal
// when there is nothing to compare.
//
// Run: node dev/scripts/test-check-default-handlers.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, cpSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = "dev/scripts/check-default-handlers.py";
const LIST = "dev/mkosi/mkosi.extra/usr/share/applications/mimeapps.list";
const failures = [];

/// A tree the gate can read: its own script, one entry, one list.
function tree(entry, list) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-handlers-"));
  mkdirSync(join(dir, dirname(GATE)), { recursive: true });
  cpSync(join(ROOT, GATE), join(dir, GATE));
  if (entry !== null) {
    mkdirSync(join(dir, "apps/reader/dist"), { recursive: true });
    writeFileSync(join(dir, "apps/reader/dist/arlen-reader.desktop"), entry);
  }
  if (list !== null) {
    mkdirSync(join(dir, dirname(LIST)), { recursive: true });
    writeFileSync(join(dir, LIST), list);
  }
  return dir;
}

function run(name, entry, list, expect) {
  const dir = tree(entry, list);
  const r = spawnSync("python3", [join(dir, GATE)], { encoding: "utf8" });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  const ok = expect(r.status ?? 1, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code: r.status, out });
  rmSync(dir, { recursive: true, force: true });
}

const ENTRY = "[Desktop Entry]\nName=Reader\nMimeType=application/pdf;\n";
const GOOD = "[Default Applications]\napplication/pdf=arlen-reader.desktop\n";

console.log("default handlers:");

run("the tree as it stands passes", null, null, () => {
  const r = spawnSync("python3", [join(ROOT, GATE)], { encoding: "utf8" });
  const ok = r.status === 0;
  if (!ok) console.log(`${r.stdout ?? ""}${r.stderr ?? ""}`);
  return ok;
});

run(
  "a claimed type with no default is caught",
  ENTRY,
  "[Default Applications]\ntext/plain=arlen-reader.desktop\n",
  (code, out) => code === 1 && out.includes("nothing defaults to it"),
);

run(
  "a default naming another app is caught",
  ENTRY,
  "[Default Applications]\napplication/pdf=someone-else.desktop\n",
  (code, out) => code === 1 && out.includes("is the entry that claims it"),
);

run(
  "a default for a type nobody claims is caught",
  ENTRY,
  `${GOOD}image/png=arlen-reader.desktop\n`,
  (code, out) => code === 1 && out.includes("no shipped entry claims that type"),
);

run("a matched pair passes", ENTRY, GOOD, (code) => code === 0);

run(
  "a tree with no list refuses rather than passing",
  ENTRY,
  null,
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

run(
  "a tree with no entries refuses rather than passing",
  null,
  GOOD,
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed`);
  for (const f of failures) console.log(`\n--- ${f.name}\n${f.out}`);
  process.exit(1);
}
console.log("a claim without a default, a default for another app and one for nobody are all caught");
