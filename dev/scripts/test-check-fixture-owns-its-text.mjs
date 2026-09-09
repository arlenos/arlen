// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-fixture-owns-its-text.py`: a fixture that presses a word
// the catalogue owns has to be caught, and one that presses a word it typed
// itself has to pass. Both directions, because a check that only ever says yes is
// the thing this whole family exists to stop.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const gate = path.join(here, "check-fixture-owns-its-text.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failed += 1;
    console.log(`  FAIL ${name}`);
    if (detail) console.log(`       ${detail}`);
  }
}

/// A tree with one catalogue sentence and one fixture body.
function gateOver(fixture, { catalogWord = "Entfernen" } = {}) {
  const dir = mint("arlen-fixture-text-");
  try {
    const i18n = path.join(dir, "apps", "demo", "src", "lib", "i18n");
    const hosts = path.join(dir, "dev", "screenshot", "hosts");
    mkdirSync(i18n, { recursive: true });
    mkdirSync(hosts, { recursive: true });
    writeFileSync(
      path.join(i18n, "messages.ts"),
      `export const messages = {\n  de: {\n    "d.remove": "${catalogWord}",\n  },\n};\n`,
      "utf8",
    );
    writeFileSync(path.join(hosts, "demo-fixture.js"), fixture, "utf8");
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("fixture-owns-its-text:");

{
  const r = gateOver(`var b = byText("Entfernen");\n`);
  check("a fixture pressing catalogue copy is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names the word", r.out.includes("`Entfernen`"));
}

{
  // The same word, typed by the fixture first. It owns both ends, so it passes.
  const r = gateOver(
    `setter.call(input, "Entfernen");\nif (el.textContent.trim() === "Entfernen") el.click();\n`,
  );
  check("a word the fixture typed itself passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  // A word that is nobody's copy.
  const r = gateOver(`var b = byText("nosuchcommand");\n`);
  check("a word no catalogue carries passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  // The repair for this defect is a comment naming the old word, and the first
  // cut of the gate read that as the fault.
  const r = gateOver(`// NOT byText("Entfernen"): the label moved.\nvar b = last(dialog);\n`);
  check("a comment naming the old word is not a finding", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  const r = (() => {
    try {
      return { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  })();
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("a fixture presses what it owns, not what the copy says");
