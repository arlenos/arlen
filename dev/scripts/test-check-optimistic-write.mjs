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

if (failures.length) {
  console.log(`\n${failures.length} case(s) failed:`);
  for (const f of failures) console.log(`  ${f.name}\n    exit ${f.code}\n${f.out}`);
  process.exit(1);
}
console.log("the plant is caught and the correct form passes");
