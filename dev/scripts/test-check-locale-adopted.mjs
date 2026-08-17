// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-locale-adopted.
//
// The case that matters is the first: an app with a full catalogue, `$t()` on
// every string, and nothing that ever sets the language. That is what the greeter
// was, and it looks completely healthy in the source - the defect is the absence.
//
// Run: node dev/scripts/test-check-locale-adopted.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-locale-adopted.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-locale-"));
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
  rmSync(dir, { recursive: true, force: true });
}

const CATALOG = 'export const messages = { en: { "d.hi": "Hello" }, de: { "d.hi": "Hallo" } };\n';

console.log("check-locale-adopted:");

check(
  "a tree with no catalogue anywhere is refused rather than reported clean",
  { "apps/demo/src/routes/+page.svelte": "<p>x</p>\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a catalogue with no startup language is caught",
  {
    "apps/demo/src/lib/i18n/messages.ts": CATALOG,
    "apps/demo/src/routes/+layout.svelte": '<script>\n  import "../app.css";\n</script>\n',
  },
  (code, out) => code === 1 && out.includes("apps/demo"),
);

check(
  "the kit helper counts",
  {
    "apps/demo/src/lib/i18n/messages.ts": CATALOG,
    "apps/demo/src/routes/+layout.svelte":
      '<script>\n  import { initArlenLocale } from "@arlen/ui-kit/i18n";\n' +
      "  onMount(() => void initArlenLocale());\n</script>\n",
  },
  (code) => code === 0,
);

// Settings resolves it itself, from the file it owns. That is not a workaround to
// be nagged about, so the second form has to count too.
check(
  "an app that resolves the language itself counts",
  {
    "apps/demo/src/lib/i18n/messages.ts": CATALOG,
    "apps/demo/src/routes/+layout.svelte":
      '<script>\n  invoke("config_get").then((ui) => locale.set(ui));\n</script>\n',
  },
  (code) => code === 0,
);

check(
  "an app with no catalogue is not this rule's business",
  {
    "apps/demo/src/lib/i18n/messages.ts": CATALOG,
    // Called, not merely imported: an import with no call is the "wired but never
    // run" shape, and this gate is right to keep counting that as absent.
    "apps/demo/src/routes/+layout.svelte":
      '<script>\n  import { initArlenLocale } from "@arlen/ui-kit/i18n";\n' +
      "  onMount(() => void initArlenLocale());\n</script>\n",
    "apps/plain/src/routes/+page.svelte": "<p>no catalogue here</p>\n",
  },
  (code, out) => code === 0 && !out.includes("apps/plain"),
);

for (const f of failures) {
  console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
}
if (failures.length) process.exit(1);
console.log("a catalogue must be reachable, and both ways of adopting a language pass");
