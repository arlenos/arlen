// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Does the i18n-reactivity gate still catch a string frozen at import?
//
// What it guards does not fail any other way: both shapes compile, typecheck, and
// render correctly in English, which is the only locale most of us look at. They
// ship the source language forever, and the only signal is a locale switch nobody
// performs. So this gate going quiet would look exactly like clean code.
//
// The cases below drive both faults AND both of the two shapes the check's own
// header says it must leave alone - a const whose initialiser is a FUNCTION, and
// a declaration inside a function body. Those matter as much as the faults: this
// check has already turned CI red over correct code once, and a gate that cries
// wolf gets relaxed until it says nothing.
//
// Over a fixture; it takes a base directory as `argv[2]`.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mint, cleanup } from "./lib/fixture.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const gate = path.join(root, "dev/scripts/check-i18n-reactivity.mjs");

let failed = 0;
function check(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else { console.log(`  FAIL ${name}`); if (detail) console.log(`       ${detail}`); failed++; }
}

/// One app under a throwaway base, holding `body` as a component.
function gateOver(body) {
  const dir = mint("arlen-i18n-react-");
  try {
    const src = path.join(dir, "probe", "src", "lib");
    mkdirSync(src, { recursive: true });
    writeFileSync(path.join(src, "Probe.svelte"), body, "utf8");
    try {
      return { code: 0, out: execFileSync("node", [gate, dir], { encoding: "utf8" }) };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
    }
  } finally {
    cleanup(dir);
  }
}

console.log("i18n reactivity:");

{
  let r;
  try {
    r = { code: 0, out: execFileSync("node", [gate], { cwd: root, encoding: "utf8" }) };
  } catch (e) {
    r = { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
  check("the tree as it stands passes", r.code === 0, r.out.trim().split("\n").pop());
}

// FAULT ONE: a top-level constant, evaluated once at import, holding whatever the
// translator had then.
{
  const r = gateOver(
    `<script lang="ts">\n  import { t } from "$lib/i18n/messages";\n` +
    `  const OPTIONS = [{ label: $t("probe.one") }];\n</script>\n` +
    `{#each OPTIONS as o}<span>{o.label}</span>{/each}\n`,
  );
  check("a top-level constant holding $t is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

// FAULT TWO: `get()` reads the store imperatively, so the markup calling the
// function never re-renders on a locale switch.
{
  const r = gateOver(
    `<script lang="ts">\n  import { get } from "svelte/store";\n` +
    `  import { t } from "$lib/i18n/messages";\n` +
    `  function label() { return get(t)("probe.two"); }\n</script>\n<span>{label()}</span>\n`,
  );
  check("an imperative get() read is caught", r.code === 1, r.out.trim().split("\n")[0]);
}

// LEFT ALONE ONE: the correct shape, `$t` straight in markup.
{
  const r = gateOver(
    `<script lang="ts">\n  import { t } from "$lib/i18n/messages";\n</script>\n` +
    `<span>{$t("probe.three")}</span>\n`,
  );
  check("$t in markup passes", r.code === 0, r.out.trim().split("\n")[0]);
}

// LEFT ALONE TWO: a const whose initialiser is a FUNCTION. The check's header
// records flagging one of these and turning CI red over correct code.
{
  const r = gateOver(
    `<script lang="ts">\n  import { t } from "$lib/i18n/messages";\n` +
    `  const f = (n: number) => $t("probe.four", { n });\n</script>\n<span>{f(1)}</span>\n`,
  );
  check("a const whose initialiser is a function passes", r.code === 0, r.out.trim().split("\n")[0]);
}

// LEFT ALONE THREE: a declaration inside a function body is re-evaluated per call.
{
  const r = gateOver(
    `<script lang="ts">\n  import { t } from "$lib/i18n/messages";\n` +
    `  function build() {\n    const rows = [{ label: $t("probe.five") }];\n    return rows;\n  }\n` +
    `</script>\n{#each build() as r}<span>{r.label}</span>{/each}\n`,
  );
  check("a declaration inside a function body passes", r.code === 0, r.out.trim().split("\n")[0]);
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("the gate catches both frozen shapes and leaves the three correct ones alone");
