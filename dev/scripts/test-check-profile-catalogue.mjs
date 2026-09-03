// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the curated-catalogue check: plant each fault in a
// fixture catalogue and watch it refuse.
//
// The catalogue is over two thousand hand-written files and it grows by hand, one
// per application covered. Nothing looked at it until this check - `check-app-
// profiles.py` compares the apps the IMAGE installs and says in its own header
// that the catalogue is a different thing - so every fault below could have been
// sitting in it for as long as it has existed.
//
// Run: node dev/scripts/test-check-profile-catalogue.mjs

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-profile-catalogue.py");

const failures = [];

// One good profile, so a fixture is never refused for the wrong reason.
const GOOD = `# A text editor. Documents only.
[info]
app_id = "goodedit"
tier = "third-party"

[filesystem]
documents = true
`;

function check(name, files, expect) {
  const dir = mint("arlen-profcat-");
  for (const [rel, body] of Object.entries(files)) writeFileSync(join(dir, rel), body);
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  const got = { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  const ok = expect(got.code, got.out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, ...got });
  cleanup(dir);
}

console.log("check-profile-catalogue positive control");

check(
  "a catalogue with nothing wrong passes",
  { "goodedit.toml": GOOD },
  (code, out) => code === 0 && out.includes("1 curated profile"),
);

check(
  "a profile that does not parse is caught",
  { "goodedit.toml": GOOD, "broken.toml": "[info\napp_id = \"broken\"\n" },
  (code, out) => code === 1 && out.includes("does not parse"),
);

check(
  "a profile whose app_id is not its filename is caught",
  {
    "goodedit.toml": GOOD,
    "mislabelled.toml": '[info]\napp_id = "somethingelse"\ntier = "third-party"\n',
  },
  (code, out) => code === 1 && out.includes("must agree"),
);

check(
  "a profile with no [info] table is caught",
  { "goodedit.toml": GOOD, "headless.toml": "[filesystem]\ndocuments = true\n" },
  (code, out) => code === 1 && out.includes("no `[info]` table"),
);

check(
  "a profile that states no tier is caught",
  { "goodedit.toml": GOOD, "tierless.toml": '[info]\napp_id = "tierless"\n' },
  (code, out) => code === 1 && out.includes("no `[info] tier`"),
);

check(
  "a whole-home grant with no reason is caught",
  {
    "goodedit.toml": GOOD,
    "greedy.toml": '# An editor.\n[info]\napp_id = "greedy"\ntier = "third-party"\n\n[filesystem]\nhome = true\n',
  },
  (code, out) => code === 1 && out.includes("grants the whole home"),
);

check(
  "a whole-home grant that says why passes",
  {
    "backup.toml": '# A backup tool: it copies the whole home, so anything narrower\n# would silently miss files.\n[info]\napp_id = "backup"\ntier = "third-party"\n\n[filesystem]\nhome = true\n',
  },
  (code, out) => code === 0 && out.includes("each says why"),
);

check(
  "an empty catalogue is refused rather than reported clean",
  {},
  (code, out) => code === 1 && out.includes("holds no profiles"),
);

if (failures.length) {
  console.log("");
  for (const f of failures) console.log(`FAILED ${f.name}\n  exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("all cases behaved");
