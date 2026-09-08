// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the unread-settings-key check: watch it fail on each shape it
// claims to catch, and pass on the shapes it must not.
//
// Fixture trees, not the repo, so it keeps working as keys come and go. The repo
// is asked one thing at the end: that the scan reads it at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-settings-keys-read.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** A tree with one Settings page writing `page`, and `reader` under the owner. */
function tree({ page, reader = null, ownerPath = "daemons/knowledge/src/lib.rs" }) {
  const root = mint("settings-keys-");
  const settings = join(root, "apps/settings/src/routes");
  mkdirSync(settings, { recursive: true });
  writeFileSync(join(settings, "+page.svelte"), page);
  if (reader !== null) {
    mkdirSync(dirname(join(root, ownerPath)), { recursive: true });
    writeFileSync(join(root, ownerPath), reader);
  }
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const WRITES = `<script lang="ts">
  await graph.setValue("timeline.excluded_apps", list);
</script>
`;

// The shape the timeline exclusions had: written, and the owning daemon reads
// one other field of the same section.
{
  const root = tree({
    page: WRITES,
    reader: "pub struct TimelineConfig {\n    pub paused: bool,\n}\n",
  });
  const { code, out } = run(root);
  check(
    "a key the owner never reads fails, and is named",
    code === 1 && out.includes("timeline.excluded_apps"),
  );
  cleanup(root);
}

// Read as a serde field, which is how most of them are read.
{
  const root = tree({
    page: WRITES,
    reader: "pub struct TimelineConfig {\n    pub excluded_apps: Vec<String>,\n}\n",
  });
  check("a key read as a struct field passes", run(root).code === 0);
  cleanup(root);
}

// Read as a quoted literal, which is how a dot-notation reader spells it.
{
  const root = tree({
    page: WRITES,
    reader: 'let v = cfg.get("excluded_apps");\n',
  });
  check("a key read as a literal passes", run(root).code === 0);
  cleanup(root);
}

// A reader in the WRONG component does not count. An unscoped search finds a
// word like `mode` or `active` somewhere in a tree this size no matter what,
// and passing on that coincidence is the one failure this check cannot afford.
{
  const root = tree({
    page: WRITES,
    reader: "pub struct Whatever {\n    pub excluded_apps: Vec<String>,\n}\n",
    ownerPath: "daemons/power-daemon/src/lib.rs",
  });
  const { code, out } = run(root);
  check(
    "a reader in another component does not answer for the owner",
    code === 1 && out.includes("timeline.excluded_apps"),
  );
  cleanup(root);
}

// The compositor's keys are written here and read in the other repo.
{
  const root = tree({
    page: `<script lang="ts">
  await compositor.setValue("layout.inner_gap", 4);
</script>
`,
  });
  const { code, out } = run(root);
  check(
    "a compositor key is skipped rather than reported",
    code === 0 && out.includes("skipped as compositor's"),
  );
  cleanup(root);
}

// A store with no owner recorded is reported rather than silently skipped: an
// unchecked file is exactly the gap this check exists to close.
{
  const root = tree({
    page: `<script lang="ts">
  await mystery.setValue("some.key", 1);
</script>
`,
  });
  const { code, out } = run(root);
  check("an unknown store is named, not skipped", code === 1 && out.includes("mystery"));
  cleanup(root);
}

{
  const r = run(ROOT);
  check(
    "and the repo itself passes",
    r.code === 0 && /key\(s\) written and read/.test(r.out),
  );
}

console.log(failures === 0 ? "all ok" : `${failures} failure(s)`);
process.exit(failures === 0 ? 0 : 1);
