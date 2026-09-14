// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-theme-fallbacks-agree.
//
// The case it was written from: sixteen of eighteen app.css copies held
// `--color-border: #262626` where the theme says `#27272a`, a different green
// for success, and no `--color-info` at all. Every one of them says in its own
// comment that it mirrors the theme.
//
// Run: node dev/scripts/test-check-theme-fallbacks-agree.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-theme-fallbacks-agree.py");

const THEME = "sdk/theme/themes/dark.toml";
const APP = "apps/demo/src/app.css";

const themeFile = [
  "[color.bg]",
  'shell   = "#0a0a0a"',
  'app     = "#0f0f0f"',
  'card    = "#171717"',
  'overlay = "#00000080"',
  'input   = "#1a1a1a"',
  "",
  "[color.fg]",
  'primary   = "#fafafa"',
  'secondary = "#a1a1aa"',
  'disabled  = "#52525b"',
  'inverse   = "#0a0a0a"',
  "",
  "[color.border]",
  'default = "#27272a"',
  'strong  = "#63636b"',
  "",
  "[color.semantic]",
  'error   = "#ef4444"',
  'warning = "#eab308"',
  'success = "#22c55e"',
  'info    = "#3b82f6"',
  "",
].join("\n");

/// An app copy that agrees, with optional per-token surgery.
function appCss(edits = {}) {
  const tokens = {
    "--color-bg-shell": "#0a0a0a",
    "--color-bg-app": "#0f0f0f",
    "--color-bg-card": "#171717",
    "--color-bg-overlay": "#00000080",
    "--color-bg-input": "#1a1a1a",
    "--color-fg-primary": "#fafafa",
    "--color-fg-secondary": "#a1a1aa",
    "--color-fg-disabled": "#52525b",
    "--color-fg-inverse": "#0a0a0a",
    "--color-border": "#27272a",
    "--color-border-strong": "#63636b",
    "--color-error": "#ef4444",
    "--color-warning": "#eab308",
    "--color-success": "#22c55e",
    "--color-info": "#3b82f6",
    ...edits,
  };
  const body = Object.entries(tokens)
    .filter(([, v]) => v !== null)
    .map(([k, v]) => `  ${k}: ${v};`)
    .join("\n");
  return `:root {\n${body}\n}\n`;
}

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-fallbacks-");
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

console.log("check-theme-fallbacks-agree:");

check("a copy that mirrors the theme passes", { [THEME]: themeFile, [APP]: appCss() }, (c) => c === 0);

check(
  "a drifted literal is a finding",
  { [THEME]: themeFile, [APP]: appCss({ "--color-border": "#262626" }) },
  (c, out) => c === 1 && out.includes("--color-border") && out.includes("#27272a"),
);

check(
  "a token the copy never names is a finding",
  { [THEME]: themeFile, [APP]: appCss({ "--color-info": null }) },
  (c, out) => c === 1 && out.includes("--color-info"),
);

// The monochrome accent is written as a var in several real copies; a relation
// is a statement, not a stale number, and the gate must not read it as one.
check(
  "a token written as a var rather than a literal is not a finding",
  {
    [THEME]: themeFile,
    [APP]: appCss({ "--color-fg-secondary": "var(--color-fg-primary)" }),
  },
  (c) => c === 0,
);

// A light block below the dark one legitimately redefines these.
check(
  "a later light block does not count as a drift",
  {
    [THEME]: themeFile,
    [APP]: appCss() + ".light {\n  --color-bg-app: #ffffff;\n  --color-warning: #a16207;\n}\n",
  },
  (c) => c === 0,
);

check(
  "a border written through its own default token is still compared",
  {
    [THEME]: themeFile,
    [APP]: appCss({ "--color-border": "var(--color-border-default)" })
      .replace(":root {", ":root {\n  --color-border-default: #262626;"),
  },
  (c, out) => c === 1 && out.includes("#27272a"),
);

check(
  "a tree with no theme file refuses rather than passing",
  { [APP]: appCss() },
  (c, out) => c === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a tree with no app.css refuses rather than passing",
  { [THEME]: themeFile },
  (c, out) => c === 2 && out.includes("NOTHING WAS READ"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) {
    console.log(`--- ${f.name} (exit ${f.code})`);
    console.log(f.out.trim());
  }
  process.exit(1);
}
console.log("the theme is the source, and a fallback that drifts from it is a finding");
