// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for `check-fixture-expect-still-said.py`. The fault it exists for is
// an EXPECT the copy has moved past, and the near-misses it must NOT report are a
// sentence the fixture supplies itself and a catalogue value written with a
// `\uXXXX` escape - the second one because comparing without decoding reported a
// sentence that is on screen in front of you as one nothing says.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const gate = path.join(here, "check-fixture-expect-still-said.py");

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

function gateOver(expect, { catalogValue = "Diese Suche wurde nicht gespeichert.", body = "" } = {}) {
  const dir = mint("arlen-expect-said-");
  try {
    const i18n = path.join(dir, "apps", "demo", "src", "lib", "i18n");
    const hosts = path.join(dir, "dev", "screenshot", "hosts");
    mkdirSync(i18n, { recursive: true });
    mkdirSync(hosts, { recursive: true });
    writeFileSync(
      path.join(i18n, "messages.ts"),
      `export const messages = {\n  de: {\n    "d.saved": "${catalogValue}",\n  },\n};\n`,
      "utf8",
    );
    writeFileSync(
      path.join(hosts, "demo-fixture.js"),
      `// EXPECT: ${expect}\n${body}\n`,
      "utf8",
    );
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("fixture-expect-still-said:");

{
  const r = gateOver("Diese Suche wurde nicht gespeichert");
  check("a sentence the catalogue carries passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  // The real defect: the same words, and the copy now says them mid-sentence.
  const r = gateOver("Nicht gespeichert");
  check("an EXPECT the copy moved past is caught", r.code === 1, r.out.trim().split("\n")[0]);
  check("and the finding names it", r.out.includes("`Nicht gespeichert`"));
}

{
  // A device name, a media title, a hostname: the fixture's own data.
  const r = gateOver("Vesktop", { body: 'var items = [{ title: "Vesktop" }];' });
  check("a string the fixture supplies passes", r.code === 0, r.out.trim().split("\n")[0]);
}

{
  // Catalogues write an umlaut as an escape and an EXPECT writes the letter.
  const r = gateOver("Drück eine Tastenkombination", {
    catalogValue: "Dr\\u00fcck eine Tastenkombination",
  });
  check("a catalogue escape is decoded before comparing", r.code === 0, r.out.trim().split("\n")[0]);
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
console.log("a fixture expects words something still says");
