// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// The rule this directory holds above the others: a checker is not trusted until
// it has been SHOWN TO FAIL. This one had no such test, and the reason was
// mechanical rather than an oversight - its scan root was hardcoded, so nobody
// could hand it a planted violation. The root is an argument now and these are
// the plants.
//
// The defect it exists to catch, found four times in two hours on 8 August: the
// surface is updated first, the mutation fails, and the empty catch leaves the
// screen claiming something happened that did not. Unlike a fixture, which is at
// least visibly generic, this is the user's own action reflected back at them.
//
// Run: node dev/scripts/test-check-optimistic-write.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-optimistic-write.py");

const failures = [];

function check(name, files, expect) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-optw-"));
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

console.log("check-optimistic-write:");

// Zero catch blocks is a legitimate answer, so what has to be refused is finding
// no app source at all: this reads `apps/*/src/**` only, and pointed anywhere
// else it printed "0 catch block(s) checked" and exited 0.
check(
  "a tree with no app source is refused rather than reported clean",
  { "daemons/probe/src/main.rs": "fn main() {}\n" },
  (code, out) => code === 2 && out.includes("NOTHING WAS READ"),
);

// The promise form, invisible to this gate for its whole life while its own
// summary admitted it. A log is not a revert and not a sentence: the surface has
// already claimed the change, and the correction goes where nobody looks.
check(
  "an optimistic write whose rejection only logs is caught",
  {
    "apps/demo/src/lib/stores/thing.ts":
      "export async function drop(id) {\n" +
      "  items.update(($i) => $i.filter((n) => n.id !== id));\n" +
      '  invoke("remove_thing", { id }).catch((e) => console.error("failed", e));\n' +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("console"),
);

// The rune form, and the reason the gate was widened on 16 August: a component
// setting its own state is the same optimistic update as a store's, and
// `STORE_WRITE` matched neither `=` nor a spread. Six night-light setters sat in
// that gap - flip the switch with the daemon down, it stays flipped, and night
// light is off. Copied from the shape that shipped, not invented.
check(
  "a component setting its own state before a log-only rejection is caught",
  {
    "apps/demo/src/lib/Night.svelte":
      "<script lang=\"ts\">\n" +
      "  let cfg = $state({ enabled: false });\n" +
      "  function setEnabled(enabled) {\n" +
      "    cfg = { ...cfg, enabled };\n" +
      '    invoke("night_light_set", { enabled }).catch((e) => console.warn("failed", e));\n' +
      "  }\n" +
      "</script>\n",
  },
  (code, out) => code === 1 && out.includes("console"),
);

// And the boundary for it: reverting the component's own state is the fix, so a
// handler that puts `cfg` back must pass. Without this the widening would push
// people from a log to a silent catch.
check(
  "a component that reverts its own state on failure passes",
  {
    "apps/demo/src/lib/Night.svelte":
      "<script lang=\"ts\">\n" +
      "  let cfg = $state({ enabled: false });\n" +
      "  function setEnabled(enabled) {\n" +
      "    const previous = cfg;\n" +
      "    cfg = { ...cfg, enabled };\n" +
      '    invoke("night_light_set", { enabled }).catch(() => { cfg = previous; writeFailed = true; });\n' +
      "  }\n" +
      "</script>\n",
  },
  (code) => code === 0,
);

// The boundary that keeps it honest: a handler putting the store back is the fix.
check(
  "a rejection handler that reverts the store passes",
  {
    "apps/demo/src/lib/stores/thing.ts":
      "export async function drop(id) {\n" +
      "  const previous = current;\n" +
      "  items.update(($i) => $i.filter((n) => n.id !== id));\n" +
      '  invoke("remove_thing", { id }).catch(() => { items.set(previous); });\n' +
      "}\n",
  },
  (code) => code === 0,
);

check(
  "an optimistic update with a swallowed failure is caught",
  {
    "apps/demo/src/lib/stores/thing.ts":
      "export async function pause() {\n" +
      "  paused.set(true);\n" +
      "  try {\n" +
      '    await invoke("pause_the_thing");\n' +
      "  } catch {\n" +
      "  }\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("thing.ts"),
);

// The same shape with the surface put back is the correct form, and it has to
// pass or the rule would push people to remove the try instead of the lie.
check(
  "the same call that reverts on failure passes",
  {
    "apps/demo/src/lib/stores/thing.ts":
      "export async function pause() {\n" +
      "  paused.set(true);\n" +
      "  try {\n" +
      '    await invoke("pause_the_thing");\n' +
      "  } catch {\n" +
      "    paused.set(false);\n" +
      "  }\n" +
      "}\n",
  },
  (code) => code === 0,
);

// An empty catch with no optimistic write before it is somebody ignoring an
// error, which is a different thing and not this check's business - flagging it
// would bury the four real ones in noise.
check(
  "an empty catch with no surface update before it is not this rule's business",
  {
    "apps/demo/src/lib/stores/thing.ts":
      "export async function ping() {\n" +
      "  try {\n" +
      '    await invoke("ping");\n' +
      "  } catch {\n" +
      "  }\n" +
      "}\n",
  },
  (code) => code === 0,
);

// The count-drop half, added 12 Aug. The docstring had promised it since the file
// was written ("a file whose count drops asks to have its number lowered") and
// nothing implemented it, which went unnoticed until tightening the lookbehind
// took four of six entries to zero in one commit.
//
// It audits its own tree only - a fixture lacks the real KNOWN files, so every
// fixture case would report both of them as dropped. That is also why this case
// cannot use the harness above or a copy in /tmp: a copy elsewhere, handed the
// real tree, correctly declines to audit a list that is not its own. So the copy
// goes INSIDE dev/scripts under a dotted name and runs with no argument. Same
// shape as `test-check-peer-identity-sandbox.mjs`, for the same reason.
{
  const name = "a carried count with nothing left behind it is reported";
  const copy = join(ROOT, `dev/scripts/.tmp-optimistic-${process.pid}.py`);
  let got;
  try {
    writeFileSync(
      copy,
      readFileSync(join(ROOT, "dev/scripts/check-optimistic-write.py"), "utf8").replace(
        "KNOWN: dict[str, tuple[int, str]] = {",
        'KNOWN: dict[str, tuple[int, str]] = {\n    "apps/probe/src/lib/stores/none.ts": (2, "planted"),',
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

// The two widenings this gate got on 17 August, each with the shape that got past
// it: a write through a same-file helper, and a comment quoting the defect.
check(
  "a write through a local helper counts as the optimistic update",
  {
    "apps/demo/src/lib/stores/x.ts":
      'import { invoke } from "@tauri-apps/api/core";\n' +
      "function patch(fn) { inner.update(fn); }\n" +
      "export async function go() {\n" +
      "  patch((s) => ({ ...s, on: true }));\n" +
      "  try { await invoke('do_it'); } catch {}\n" +
      "}\n",
  },
  (code, out) => code === 1 && out.includes("stores/x.ts"),
);

check(
  "a comment quoting the defect is not the defect",
  {
    "apps/demo/src/lib/stores/y.ts":
      'import { invoke } from "@tauri-apps/api/core";\n' +
      "export async function go() {\n" +
      "  // was inner.update(...) then invoke('do_it').catch(() => {})\n" +
      "  await invoke('do_it');\n" +
      "}\n",
  },
  (code) => code === 0,
);

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("the plant is caught and the correct form passes");
