// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-terminal-palette-agrees.
//
// The case it was written from: the Settings swatch floor held `#2563eb` for
// blue while the theme file authored `#7d9cc4`, so the editor offered a palette
// the terminal never printed. An axe sweep found it sideways, by measuring the
// contrast of a colour that does not ship.
//
// Run: node dev/scripts/test-check-terminal-palette-agrees.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-terminal-palette-agrees.py");

const SLOTS = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "bright_black", "bright_red", "bright_green", "bright_yellow",
  "bright_blue", "bright_magenta", "bright_cyan", "bright_white",
];
// Sixteen distinguishable values, so a mix-up between two slots is visible
// rather than accidentally equal.
const HEX = SLOTS.map((_, i) => `#0000${(0x10 + i).toString(16)}`);

const camel = (s) => {
  const [head, tail] = s.split("_");
  return tail ? head + tail[0].toUpperCase() + tail.slice(1) : head;
};

function themeFile(values = HEX) {
  return (
    "[color.bg]\napp = \"#0f0f0f\"\n\n[terminal.ansi]\n" +
    SLOTS.map((s, i) => `${s.padEnd(14)} = "${values[i]}"`).join("\n") +
    "\n"
  );
}

function terminalApp(values = HEX) {
  return (
    "export const arlenTerminalTheme = {\n  background: \"#0f0f0f\",\n" +
    SLOTS.map((s, i) => `  ${camel(s)}: "${values[i]}",`).join("\n") +
    "\n};\n"
  );
}

function settingsStore(values = HEX) {
  return (
    "export const SYS_DEFAULTS = {\n  cursorSize: 24,\n" +
    values.map((v, i) => `  ansi${i}: "${v}",`).join("\n") +
    "\n  termBg: \"#0f0f0f\",\n};\n"
  );
}

const THEME = "sdk/theme/themes/dark.toml";
const APP = "apps/terminal/src/lib/terminal-theme.ts";
const SETTINGS = "apps/settings/src/lib/stores/themeSystem.ts";

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-termpalette-");
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

console.log("check-terminal-palette-agrees:");

check(
  "three agreeing copies pass",
  { [THEME]: themeFile(), [APP]: terminalApp(), [SETTINGS]: settingsStore() },
  (code) => code === 0,
);

// The real case, in both copies, one at a time.
const drifted = [...HEX];
drifted[4] = "#2563eb";

check(
  "a drifted slot in the Settings floor is a finding",
  { [THEME]: themeFile(), [APP]: terminalApp(), [SETTINGS]: settingsStore(drifted) },
  (code, out) => code === 1 && out.includes("themeSystem.ts") && out.includes("blue"),
);

check(
  "a drifted slot in the xterm grid is a finding",
  { [THEME]: themeFile(), [APP]: terminalApp(drifted), [SETTINGS]: settingsStore() },
  (code, out) => code === 1 && out.includes("terminal-theme.ts") && out.includes("blue"),
);

check(
  "a slot a copy does not name at all is a finding",
  {
    [THEME]: themeFile(),
    [APP]: terminalApp(),
    [SETTINGS]: settingsStore().replace(/  ansi12: "[^"]+",\n/, ""),
  },
  (code, out) => code === 1 && out.includes("bright_blue"),
);

// Case is presentation, not meaning: the theme writes lowercase hex and a copy
// that shouts the same colour is the same colour.
check(
  "the same colour in a different case is not a finding",
  {
    [THEME]: themeFile(),
    [APP]: terminalApp(),
    [SETTINGS]: settingsStore(HEX.map((h) => h.toUpperCase())),
  },
  (code) => code === 0,
);

check(
  "a theme that authors no slots refuses rather than passing",
  {
    [THEME]: "[color.bg]\napp = \"#0f0f0f\"\n",
    [APP]: terminalApp(),
    [SETTINGS]: settingsStore(),
  },
  (code, out) => code === 2 && out.includes("NOTHING TO COMPARE AGAINST"),
);

check(
  "a missing copy refuses rather than passing",
  { [THEME]: themeFile(), [APP]: terminalApp() },
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
console.log("the theme is the source, and a copy that drifts from it is a finding");
