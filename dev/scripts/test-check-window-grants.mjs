// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The case that matters here is the third one. A naive version of this check -
// read `apps/*/src`, compare against the grants - reported thirteen apps
// over-granting close/minimize/show, uniformly, and every one of those was wrong:
// the calls live in the ui-kit's `WindowControls`, which the apps render. A
// uniform finding across an entire tree is almost always a shared caller the
// scanner cannot see, and this pins the credit that fixes it.
//
// Run: node dev/scripts/test-check-window-grants.mjs

import { mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-window-grants.py");

const failures = [];

function check(name, files, expect) {
  const dir = mint("arlen-wgrants-");
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

const caps = (perms) =>
  JSON.stringify({ identifier: "main", windows: ["main"], permissions: perms }, null, 2);

// Stands in for the real ui-kit control, calling what it calls.
const KIT = {
  "sdk/ui-kit/src/lib/components/ui/window-controls/WindowControls.svelte":
    "<script>\n  const w = getCurrentWindow();\n" +
    "  const a = () => w.close();\n  const b = () => w.minimize();\n</script>\n",
};

console.log("check-window-grants:");

check(
  "a grant with no call behind it is caught",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:window:allow-unmaximize"]),
    "apps/probe/src/lib/x.ts": "export const x = 1;\n",
  },
  (code, out) => code === 1 && out.includes("allow-unmaximize"),
);

check(
  "the same grant with the call passes",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:window:allow-unmaximize"]),
    "apps/probe/src/lib/x.ts": "await getCurrentWindow().unmaximize();\n",
  },
  (code) => code === 0,
);

// The one the naive version got wrong thirteen times over.
check(
  "an app that renders the shared control is credited with its calls",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps([
      "core:window:allow-close",
      "core:window:allow-minimize",
    ]),
    "apps/probe/src/routes/+page.svelte":
      '<script>\n  import { WindowControls } from "@arlen/ui-kit";\n</script>\n<WindowControls />\n',
  },
  (code) => code === 0,
);

// And the boundary: rendering the control does not credit a call it never makes.
check(
  "the credit covers only what the control actually calls",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:window:allow-show"]),
    "apps/probe/src/routes/+page.svelte":
      '<script>\n  import { WindowControls } from "@arlen/ui-kit";\n</script>\n<WindowControls />\n',
  },
  (code, out) => code === 1 && out.includes("allow-show"),
);

// The attribute form: `data-tauri-drag-region` invokes start-dragging with no
// method call, so a file carrying it is a call site. This was documented as an
// uncovered hazard for twenty minutes on the strength of a broken pathspec that
// said the attribute occurred once, in the kit; it occurs in nine app files.
check(
  "the drag-region attribute counts as calling start-dragging",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:window:allow-start-dragging"]),
    "apps/probe/src/routes/+page.svelte": '<div data-tauri-drag-region>title</div>\n',
  },
  (code) => code === 0,
);

check(
  "an app with neither the attribute nor the call is still caught",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:window:allow-start-dragging"]),
    "apps/probe/src/routes/+page.svelte": "<div>title</div>\n",
  },
  (code, out) => code === 1 && out.includes("allow-start-dragging"),
);

// A permission with no name-to-call rule is out of scope, not a finding.
check(
  "a permission outside the window family is not this check's business",
  {
    ...KIT,
    "apps/probe/src-tauri/capabilities/main.json": caps(["core:event:default"]),
    "apps/probe/src/lib/x.ts": "export const x = 1;\n",
  },
  (code) => code === 0,
);

check(
  "an empty tree is a moved layout, not a pass",
  { "README.md": "nothing here\n" },
  (code, out) => code === 1 && out.includes("layout moved"),
);

// The count-drop half. It audits its own tree only - a fixture has none of the
// real apps - so the copy goes inside dev/scripts under a dotted name and runs
// with no argument, the same shape `test-check-peer-identity-sandbox.mjs` needs.
{
  const name = "a carried count with nothing left behind it is reported";
  const copy = join(ROOT, `dev/scripts/.tmp-wgrants-${process.pid}.py`);
  let got;
  try {
    writeFileSync(
      copy,
      readFileSync(GATE, "utf8").replace(
        "KNOWN: dict[str, tuple[int, str]] = {",
        'KNOWN: dict[str, tuple[int, str]] = {\n    "no-such-app": (2, "planted"),',
      ),
    );
    const r = spawnSync("python3", [copy], { encoding: "utf8" });
    got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  } finally {
    rmSync(copy, { force: true });
  }
  const ok = got.code === 1 && got.out.includes("only 0 remain");
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
}

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("an unused grant is caught, a shared control is credited, and a stale count is reported");
