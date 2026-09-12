// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-role-keeps-its-tab-stop.
//
// The case it was written from: a second action tucked inside a control, wearing
// `role="button"` so a screen reader announces it and `tabindex="-1"` so no
// keyboard can reach it. The quick-settings tile and the places sidebar both had
// one.
//
// The rest are the lines that must not be crossed. `tabindex="-1"` alone is a
// legitimate pattern (a container focused programmatically so Escape lands), and
// a roving-tabindex widget deliberately holds its ITEMS at -1 under one container
// tab stop - but only when that container role is actually there.
//
// Run: node dev/scripts/test-check-role-keeps-its-tab-stop.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-role-keeps-its-tab-stop.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-tabstop-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-role-keeps-its-tab-stop:");

check(
  "a role=button held at tabindex -1 is a finding",
  {
    "sdk/ui-kit/src/lib/Row.svelte":
      '<button class="row">\n' +
      '  <span class="label">Home</span>\n' +
      '  <span role="button" tabindex="-1" aria-label="Unpin Home">x</span>\n' +
      "</button>\n",
  },
  (code, out) => code === 1 && out.includes("Row.svelte"),
);

check(
  "the same row passes once the second action is a control of its own",
  {
    "sdk/ui-kit/src/lib/Row.svelte":
      '<li class="item">\n' +
      '  <button class="row"><span class="label">Home</span></button>\n' +
      '  <button type="button" aria-label="Unpin Home">x</button>\n' +
      "</li>\n",
  },
  (code) => code === 0,
);

// A container focused so Escape reaches its handler is the pattern this tree
// already uses in four places, and it carries no role.
check(
  "tabindex -1 with no role is not a finding",
  {
    "apps/demo/src/lib/Overlay.svelte":
      '<div class="overlay" tabindex="-1">\n  <button>ok</button>\n</div>\n',
  },
  (code) => code === 0,
);

// A roving-tabindex widget: the container owns the arrow keys and is the tab
// stop, so its options legitimately sit at -1.
check(
  "an option under a listbox is the roving pattern, not the defect",
  {
    "apps/demo/src/lib/Picker.svelte":
      '<div role="listbox" tabindex="0">\n' +
      '  <div role="option" tabindex="-1">One</div>\n' +
      '  <div role="option" tabindex="-1">Two</div>\n' +
      "</div>\n",
  },
  (code) => code === 0,
);

// And the exemption does not travel: an option with no container role above it
// is the same unreachable control in different clothes.
check(
  "an option with no container role is still a finding",
  {
    "apps/demo/src/lib/Stray.svelte":
      '<div class="list">\n  <div role="option" tabindex="-1">One</div>\n</div>\n',
  },
  (code, out) => code === 1 && out.includes("Stray.svelte"),
);

check(
  "an empty tree refuses rather than passing",
  { "README.md": "no components here\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("a control a keyboard cannot reach is a finding, and the two patterns that look like one are not");
