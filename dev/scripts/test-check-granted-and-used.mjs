// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A capability file is a grant and a ui-kit call is the use of it, and nothing
// joins them: an app can declare `arlen-shell:allow-theme-get` and never call
// `initArlenTheme`, which is a permission held for nothing AND a feature that
// silently is not happening. Three apps were in exactly that state on 9 August,
// sitting in the default palette while the other six followed the system theme.
//
// The check was shown to fail by hand when it was written - deleting a call from
// a layout - and that manual attempt is what found the import-versus-call bug.
// This file makes that permanent, because the bug it found is one line away from
// coming back: `calls()` skips lines starting with `import`, and a check looking
// for the bare name would go green on an app that kept the import and lost the
// call.
//
// Run: node dev/scripts/test-check-granted-and-used.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-granted-and-used.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-grantuse-"));
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

const grant = (perm) =>
  JSON.stringify({ identifier: "default", permissions: [perm] }, null, 2);

console.log("check-granted-and-used:");

check(
  "an app that grants the permission and calls it passes",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("arlen-shell:allow-theme-get"),
    "apps/probe/src/routes/+layout.svelte":
      'import { initArlenTheme } from "@arlen/ui-kit";\nonMount(() => initArlenTheme());\n',
  },
  (code) => code === 0,
);

check(
  "an app that grants it and never calls it is named",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("arlen-shell:allow-theme-get"),
    "apps/probe/src/routes/+layout.svelte": "<h1>hello</h1>\n",
  },
  (code, out) => code === 1 && out.includes("probe") && out.includes("initArlenTheme"),
);

// The bug the manual attempt found, made permanent. Keeping the import while
// losing the call is what a refactor does, and it is the shape that reads as
// wired from every angle except the running app.
check(
  "an import without a call is not a use",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("arlen-shell:allow-theme-get"),
    "apps/probe/src/routes/+layout.svelte":
      'import { initArlenTheme } from "@arlen/ui-kit";\n// nothing calls it\n',
  },
  (code, out) => code === 1 && out.includes("probe"),
);

check(
  "a commented-out call is not a call either",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("arlen-shell:allow-theme-get"),
    "apps/probe/src/routes/+layout.svelte": "// initArlenTheme();\n",
  },
  (code, out) => code === 1 && out.includes("probe"),
);

// The other direction is deliberately NOT a finding: an app that neither grants
// nor calls has simply not adopted the feature, and reporting it would turn this
// into a nag about what every app could be doing rather than a check that a
// declared grant is real.
//
// A SECOND app that does pair is in the fixture on purpose: without it the tree
// pairs nothing at all, which the zero-guard below now calls a broken scan - and
// rightly, since that is indistinguishable from the layout having moved. Writing
// the guard is what forced this fixture to be realistic, which is a fair trade.
check(
  "an app that grants nothing is not asked to call anything",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("core:window:allow-close"),
    "apps/probe/src/routes/+layout.svelte": "<h1>hello</h1>\n",
    "apps/other/src-tauri/capabilities/default.json": grant("arlen-shell:allow-theme-get"),
    "apps/other/src/routes/+layout.svelte": "onMount(() => initArlenTheme());\n",
  },
  (code, out) => code === 0 && !out.includes("probe"),
);

check(
  "the second pair is checked too, not just the first",
  {
    "apps/probe/src-tauri/capabilities/default.json": grant("arlen-shell:allow-locale-get"),
    "apps/probe/src/routes/+layout.svelte": "<h1>hello</h1>\n",
  },
  (code, out) => code === 1 && out.includes("initArlenLocale"),
);

// The defect this control found. Every app grants `arlen-shell:allow-theme-get`,
// so a scan that pairs nothing has stopped finding things rather than found
// nothing to say - and it used to print "0 checked" and exit 0, which is the
// silent-cap shape this directory exists to remove.
check(
  "a tree where nothing pairs is a broken scan, not a pass",
  { "apps/probe/src-tauri/capabilities/default.json": grant("core:window:allow-close") },
  (code, out) => code === 2 && out.includes("not a check that passed"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all granted-and-used cases passed");
