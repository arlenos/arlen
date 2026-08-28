// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A grant nobody can see is a grant nobody can revoke. `schedule_wake` was added
// to `PowerPermissions` on 7 August, compiled everywhere, and appeared in no
// summary at all - the parent bound `power` whole - so an app could be granted
// the ability to wake the machine and no page would say so. Nothing was red.
//
// The third case here is the one that matters. These summaries open with
// `let Self { a, b, c } = self;`, which names every field, so a check searching
// for the bare name finds it whatever the body then does. The gate strips the
// destructuring pattern before searching for exactly that reason, and its own
// docstring records that two mutation tests sailed through the version that did
// not. This pins the strip.
//
// Run: node dev/scripts/test-check-grant-visibility.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-grant-visibility.py");

const failures = [];

function check(name, source, expect) {
  const dir = mint("arlen-grantvis-");
  if (source !== null) {
    mkdirSync(join(dir, "sdk/permissions/src"), { recursive: true });
    writeFileSync(join(dir, "sdk/permissions/src/lib.rs"), source);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

/// A struct plus a summary body, in the shape the real file uses.
const unit = (fields, body) => `pub struct ProbePermissions {
${fields.map((f) => `    pub ${f}: bool,`).join("\n")}
}

impl ProbePermissions {
    pub fn reach_summary(&self) -> Option<String> {
        let Self { ${fields.join(", ")} } = self;
${body}
    }
}
`;

console.log("check-grant-visibility:");

check(
  "a summary that names every field passes",
  unit(
    ["camera", "microphone"],
    `        let mut parts = Vec::new();
        if *camera { parts.push("camera"); }
        if *microphone { parts.push("microphone"); }
        Some(parts.join(", "))`,
  ),
  (code) => code === 0,
);

check(
  "a field the summary never turns into words is named",
  unit(
    ["camera", "schedule_wake"],
    `        let mut parts = Vec::new();
        if *camera { parts.push("camera"); }
        Some(parts.join(", "))`,
  ),
  (code, out) => code === 1 && out.includes("schedule_wake") && out.includes("invisible on the App-access page"),
);

// The defect the gate was itself fixed for. `schedule_wake` appears in the
// destructuring pattern and nowhere else, which is what every one of these
// summaries looks like for a field that projects nothing.
check(
  "a field named only in the destructuring pattern does not count as projected",
  `pub struct ProbePermissions {
    pub camera: bool,
    pub schedule_wake: bool,
}

impl ProbePermissions {
    pub fn reach_summary(&self) -> Option<String> {
        let Self { camera, schedule_wake } = self;
        let mut parts = Vec::new();
        if *camera { parts.push("camera"); }
        Some(parts.join(", "))
    }
}
`,
  (code, out) => code === 1 && out.includes("schedule_wake"),
);

check(
  "a struct with no summary and no declared parent is named",
  `pub struct ProbePermissions {
    pub camera: bool,
}
`,
  (code, out) => code === 1 && out.includes("no reach_summary"),
);

// Found by this control. A source with no structs in it printed "0 permission
// dimension(s)" AND a negative count - "-4 reach a summary", because the summary
// subtracts the ledger from a total of zero - and exited 0. A scan that has
// stopped finding things is not a scan that found nothing wrong.
check(
  "a source with no permission structs is a moved layout, not a pass",
  "// the types live somewhere else now\n",
  (code, out) => code === 2 && out.includes("has not passed"),
);

check(
  "a missing source says so instead of raising",
  null,
  (code, out) => code === 2 && out.includes("moved"),
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all grant-visibility cases passed");
