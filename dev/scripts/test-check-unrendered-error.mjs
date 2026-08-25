// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Controls for check-unrendered-error.
//
// The first case is the defect this was written from, reduced: knowledge's
// `savedUnavailable`, set by a catch and read by no component, so a failed read
// rendered as "No saved searches yet."
//
// Run: node dev/scripts/test-check-unrendered-error.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-unrendered-error.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-unrend-"));
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

const STORE =
  'import { writable } from "svelte/store";\n' +
  "export const saved = writable([]);\n" +
  "export const savedUnavailable = writable(false);\n" +
  "export async function load() {\n" +
  "  try {\n" +
  '    saved.set(await invoke("searches"));\n' +
  "  } catch {\n" +
  "    saved.set([]);\n" +
  "    savedUnavailable.set(true);\n" +
  "  }\n" +
  "}\n";

console.log("check-unrendered-error:");

// Pointed somewhere with no apps it must refuse, not report a clean tree: a
// checker that reads nothing and prints ok is the failure mode this whole
// directory exists to avoid.
check(
  "a tree with no apps is refused rather than reported clean",
  { "daemons/probe/src/main.rs": "fn main() {}\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

check(
  "a failure store no component reads is caught",
  {
    "apps/demo/src/lib/stores/search.ts": STORE,
    "apps/demo/src/lib/View.svelte":
      "<p>{$saved.length === 0 ? 'No saved searches yet.' : ''}</p>\n",
  },
  (code, out) => code === 1 && out.includes("savedUnavailable"),
);

// The fix, and it must pass - otherwise the gate pushes people to delete the
// flag rather than render it, which is the wrong direction entirely.
check(
  "the same store passes once a component reads it",
  {
    "apps/demo/src/lib/stores/search.ts": STORE,
    "apps/demo/src/lib/View.svelte":
      "<p>{$savedUnavailable ? 'Cannot read your saved searches.' : ''}</p>\n",
  },
  (code) => code === 0,
);

// An app with no failure store at all is not this rule's business.
check(
  "an app that records no failure is not a finding",
  {
    "apps/demo/src/lib/stores/plain.ts":
      'import { writable } from "svelte/store";\nexport const items = writable([]);\n',
    "apps/demo/src/lib/View.svelte": "<p>{$items.length}</p>\n",
  },
  (code) => code === 0,
);

// Shape 2: the failure is a field on the store's state rather than a store of
// its own. This is the shape that got past the first rule and reached a shipped
// page, where a failed read printed `No bindings match ""`.
// The words the vocabulary gained. Each of these is a real store name from the
// tree, and each was invisible to this gate until the pattern was widened - they
// all happened to be read, so nothing was broken and nothing said the gate was
// not watching them either.
for (const name of ["openFailure", "searchRefusals", "accessDenied", "printBlocked", "hostOffline"]) {
  check(
    `a store named ${name} is watched, not just one named error`,
    {
      "apps/demo/src/lib/stores/thing.ts":
        'import { writable } from "svelte/store";\n' +
        `export const ${name} = writable(null);\n`,
      "apps/demo/src/lib/View.svelte": "<p>hello</p>\n",
    },
    (code, out) => code === 1 && out.includes(name),
  );
}

// The boundary: a name that merely CONTAINS one of the words but is not a
// failure. `errorCount` would be a metric, and reporting it would train people
// to add acknowledgements for things that are fine. It is a store, so it is
// caught - which is the honest limit of a name rule and is written down rather
// than pretended away.
check(
  "a plain data store is not a finding",
  {
    "apps/demo/src/lib/stores/thing.ts":
      'import { writable } from "svelte/store";\nexport const rows = writable([]);\n',
    "apps/demo/src/lib/View.svelte": "<p>hello</p>\n",
  },
  (code) => code === 0,
);

const STATE_STORE =
  'import type { Readable } from "svelte/store";\n' +
  "interface BindingsState {\n" +
  "  data: string[];\n" +
  "  loading: boolean;\n" +
  "  error: string | null;\n" +
  "}\n" +
  "export const bindings: Readable<BindingsState> = inner;\n";

check(
  "a store whose state carries an unread error is caught",
  {
    "apps/demo/src/lib/stores/bindings.ts": STATE_STORE,
    "apps/demo/src/lib/View.svelte":
      "<p>{$bindings.data.length ? 'ok' : 'No bindings match.'}</p>\n",
  },
  (code, out) => code === 1 && out.includes("bindings"),
);

check(
  "the same store passes once a component reads the field",
  {
    "apps/demo/src/lib/stores/bindings.ts": STATE_STORE,
    "apps/demo/src/lib/View.svelte":
      "<p>{$bindings.error ? 'Cannot read your bindings.' : ''}</p>\n",
  },
  (code) => code === 0,
);

// A state object without the field is not this rule's business, and neither is
// a same-file derived store - only the one declared to hold that state.
check(
  "a state object with no error field is not a finding",
  {
    "apps/demo/src/lib/stores/plain2.ts":
      'import type { Readable } from "svelte/store";\n' +
      "interface Shape {\n  data: string[];\n  loading: boolean;\n}\n" +
      "export const shape: Readable<Shape> = inner;\n",
    "apps/demo/src/lib/View.svelte": "<p>{$shape.data.length}</p>\n",
  },
  (code) => code === 0,
);

for (const f of failures) {
  console.error(`\n--- ${f.name}\nexit=${f.code}\n${f.out}`);
}
if (failures.length) process.exit(1);
console.log("a recorded failure must have a reader, as a store or as a field, and both fixes pass");
