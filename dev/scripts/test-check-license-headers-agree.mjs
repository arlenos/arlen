// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the licence-header gate catch a file that relicenses itself?
//
// The fault it exists for is invisible to `reuse lint`, so "CI is green" says
// nothing about whether this works. Each case below puts one header into a
// fixture tree with its own REUSE.toml and requires the gate to answer, and the
// quiet cases matter as much: a gate that fired on the lifted Wayland XMLs or on
// the thousands of files that carry no header at all would be turned off in a
// day.
//
// OVER A FIXTURE, never this tree. The hook says at its own top that the gates
// run concurrently, so a control that edits a tracked file is visible to every
// neighbour while it runs. The gate takes its root as `argv[1]` for this.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-license-headers-agree.py");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

const MAP = `version = 1

[[annotations]]
path = "**"
precedence = "closest"
SPDX-FileCopyrightText = "2026 Tim Kicker"
SPDX-License-Identifier = "AGPL-3.0-only"

[[annotations]]
path = "daemons/kernel-layer/**"
precedence = "closest"
SPDX-FileCopyrightText = "2026 Tim Kicker"
SPDX-License-Identifier = "GPL-2.0-only"

[[annotations]]
path = "sdk/*/plain.rs"
precedence = "closest"
SPDX-FileCopyrightText = "2026 Tim Kicker"
SPDX-License-Identifier = "Apache-2.0"
`;

/// Run the gate over a throwaway tree holding `files` ({ relative path: text }).
function gateOver(files) {
  const dir = mint("arlen-license-headers-");
  try {
    writeFileSync(path.join(dir, "REUSE.toml"), MAP, "utf8");
    for (const [rel, text] of Object.entries(files)) {
      const full = path.join(dir, rel);
      mkdirSync(path.dirname(full), { recursive: true });
      writeFileSync(full, text, "utf8");
    }
    try {
      return { code: 0, out: execFileSync("python3", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

// Assembled, never spelled out: `reuse lint` reads this file too, and a fixture
// header written literally is parsed as this file's own licence. That is how the
// licence job went red on the commit that added the gate, twice over - once here
// and once in the checker's pattern.
const TAG = "SPDX-License" + "-Identifier:";
const header = (id) => `// SPDX-FileCopyrightText: 2026 Tim Kicker\n//\n// ${TAG} ${id}\n\n//! A file.\n`;

console.log("licence headers agree with the map:");

// The real tree, read-only. Every case below is only meaningful if this is green.
{
  let r;
  try {
    r = { code: 0, out: execFileSync("python3", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands agrees with its own map", r.code === 0, r.out.trim().split("\n").pop());
  check("and the lifted carve-outs are recorded rather than reported",
        /recorded carve-outs/.test(r.out), r.out.trim().split("\n").pop());
}

{
  const r = gateOver({ "apps/settings/src/a.rs": header("AGPL-3.0-only") });
  check("a header that matches the map passes", r.code === 0, r.out.trim().split("\n")[0]);
  // Pinned: a run that compared nothing also exits 0, which is how a control
  // like this goes quietly vacuous.
  check("and the gate actually read the header", /1 headers agree/.test(r.out), r.out.trim().split("\n")[0]);
}

{
  const r = gateOver({ "apps/settings/src/a.rs": header("GPL-3.0-only") });
  check("a first-party file claiming another licence is caught",
        r.code === 1 && r.out.includes("says GPL-3.0-only") && r.out.includes("says AGPL-3.0-only"),
        r.out.trim().split("\n")[0]);
}

// The narrower block wins, which is the whole reason the kernel-layer file was
// wrong: under the broad rule alone its AGPL header would have looked correct.
{
  const r = gateOver({ "daemons/kernel-layer/src/probe.rs": header("GPL-2.0-only") });
  check("a later block overrides the broad rule", r.code === 0, r.out.trim().split("\n")[0]);
}
{
  const r = gateOver({ "daemons/kernel-layer/src/probe.rs": header("AGPL-3.0-only") });
  check("and the file that was actually wrong is caught",
        r.code === 1 && r.out.includes("GPL-2.0-only"), r.out.trim().split("\n")[0]);
}

// A single star does not cross a slash. If it did, the Apache block would swallow
// files two directories down and the gate would report the wrong expected licence.
{
  const r = gateOver({ "sdk/theme/nested/plain.rs": header("AGPL-3.0-only") });
  check("a single star stays inside one path segment", r.code === 0, r.out.trim().split("\n")[0]);
}
{
  const r = gateOver({ "sdk/theme/plain.rs": header("AGPL-3.0-only") });
  check("and it does match one segment down",
        r.code === 1 && r.out.includes("Apache-2.0"), r.out.trim().split("\n")[0]);
}

// The two quiet cases that keep the gate credible.
{
  const r = gateOver({ "apps/settings/src/a.rs": "//! A file with no header at all.\n" });
  check("a file carrying no header is left alone", r.code === 0 && /0 headers agree/.test(r.out),
        r.out.trim().split("\n")[0]);
}
{
  // The fault that broke the gate's own first run: it named the tag in its
  // docstring and in its own pattern, matched itself, and reported a file
  // licensed `\\s`. A mention is not a header.
  const r = gateOver({
    "dev/scripts/a.py": `PATTERN = r"${TAG}\\s*(.+)"  # not a header\n`,
  });
  check("the tag named in prose or a pattern is not read as a header",
        r.code === 0 && /0 headers agree/.test(r.out), r.out.trim().split("\n")[0]);
}
{
  const r = gateOver({ "apps/desktop-shell/src-tauri/src/layer_shell.rs": header("GPL-3.0-only") });
  check("a recorded carve-out is not reported", r.code === 0, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches a self-relicensing file, respects block order and stays quiet on lifted code");
