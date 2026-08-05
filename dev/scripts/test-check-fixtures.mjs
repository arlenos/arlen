// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Inputs that have fooled our checks, kept so they cannot fool them again.
//
// When a gate is wrong, keep the input that fooled it - not a description of it,
// the input. Every false-green we have found had a concrete file behind it, and
// a check that has been wrong once is a check whose fix needs proving rather
// than reading. These fixtures are that proof: each case is a shape that really
// did slip past, plus the neighbouring shape that must NOT be reported, because
// a check that reports everything is as useless as one that reports nothing.
//
// Run: node dev/scripts/test-check-fixtures.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;

const failures = [];

/** Write `files` into a throwaway tree and return its path. */
function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-fixture-"));
  for (const [rel, body] of Object.entries(files)) {
    const path = join(dir, rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
  }
  return dir;
}

/** Run a check over a fixture tree; returns {code, out}. */
function run(script, dir) {
  try {
    const runner = script.endsWith(".py") ? "python3" : "node";
    const out = execFileSync(runner, [join(ROOT, "dev/scripts", script), dir], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { code: 0, out };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

function check(name, ok, detail) {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    console.log(`  FAIL ${name}: ${detail}`);
    failures.push(name);
  }
}

// ---------------------------------------------------------------------------
// check-i18n-reactivity.mjs
//
// The shape that slipped past: a top-level table whose `=` sits several lines
// below its name, because the type annotation is a multi-line generic. The first
// version of the check matched `const NAME ... = ...` on ONE line, so the whole
// declaration was invisible and `$t` in it was never reported. It went unnoticed
// because the check then printed a clean pass over the very file that had it.
// ---------------------------------------------------------------------------
function reactivityFixtures() {
  console.log("check-i18n-reactivity:");

  const caught = tree({
    "app/src/Multiline.svelte": `<script lang="ts">
  const RESET_COPY: Record<
    ResetKind,
    { title: string; body: string }
  > = {
    soft: { title: $t("reset.soft.title"), body: $t("reset.soft.body") },
  };
</script>

<p>{RESET_COPY.soft.title}</p>
`,
  });
  const r1 = run("check-i18n-reactivity.mjs", caught);
  check(
    "a $t table behind a multi-line generic is reported",
    r1.code === 1 && r1.out.includes("RESET_COPY"),
    `exit ${r1.code}: ${r1.out.trim()}`,
  );

  // The same shape done RIGHT must stay silent, or the fix is just noise: the
  // annotation is equally long, the initialiser is wrapped.
  const ok = tree({
    "app/src/Derived.svelte": `<script lang="ts">
  const RESET_COPY: Record<
    ResetKind,
    { title: string; body: string }
  > = $derived({
    soft: { title: $t("reset.soft.title"), body: $t("reset.soft.body") },
  });
</script>

<p>{RESET_COPY.soft.title}</p>
`,
  });
  const r2 = run("check-i18n-reactivity.mjs", ok);
  check(
    "the same table wrapped in $derived is not reported",
    r2.code === 0,
    `exit ${r2.code}: ${r2.out.trim()}`,
  );

  const getT = tree({
    "app/src/GetT.svelte": `<script lang="ts">
  function label(kind: string): string {
    return get(t)("kind." + kind);
  }
</script>

<p>{label("a")}</p>
`,
  });
  const r3 = run("check-i18n-reactivity.mjs", getT);
  check(
    "reading the translator with get(t) is reported",
    r3.code === 1 && r3.out.includes("get(t)"),
    `exit ${r3.code}: ${r3.out.trim()}`,
  );

  // $t inside a function body runs per call, so it follows the locale and must
  // not be reported. Indentation is the whole signal here, which is exactly why
  // it is worth pinning.
  const inFn = tree({
    "app/src/InFunction.svelte": `<script lang="ts">
  function describe(kind: string): string {
    const text = $t("kind." + kind);
    return text;
  }
</script>

<p>{describe("a")}</p>
`,
  });
  const r4 = run("check-i18n-reactivity.mjs", inFn);
  check(
    "$t inside a function body is not reported",
    r4.code === 0,
    `exit ${r4.code}: ${r4.out.trim()}`,
  );

  // A tree with no components at all must fail loudly rather than pass: a check
  // that silently scans nothing is the false-green this whole file is about.
  const empty = tree({ "app/src/notes.md": "no components here\n" });
  const r5 = run("check-i18n-reactivity.mjs", empty);
  check(
    "scanning no components fails rather than passing",
    r5.code === 2,
    `exit ${r5.code}: ${r5.out.trim()}`,
  );

  for (const d of [caught, ok, getT, inFn, empty]) rmSync(d, { recursive: true, force: true });
}

reactivityFixtures();

// ---------------------------------------------------------------------------
// check-invoke-shape.py
//
// Two false-green inputs. The first version kept ONE command map for the whole
// repo, so an app calling its own `frontend_log` was matched against another
// app's - eight findings, all false. And its payload reader stopped at the first
// `}`, which inside a template literal is the end of a `${...}`, so any call
// that interpolated looked like it passed no arguments at all.
// ---------------------------------------------------------------------------
function invokeShapeFixtures() {
  console.log("\ncheck-invoke-shape:");

  // Two apps, each with its OWN command of the same name and a DIFFERENT
  // parameter. A shared map would check one app's call against the other's
  // signature and report a mismatch that does not exist.
  const sameName = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[tauri::command]
pub fn frontend_log(level: String, msg: String) {}
`,
    "apps/alpha/src/lib/log.ts": `await invoke("frontend_log", { level: "warn", msg: "x" });\n`,
    "apps/beta/src-tauri/src/lib.rs": `#[tauri::command]
pub fn frontend_log(line: String) {}
`,
    "apps/beta/src/lib/log.ts": `await invoke("frontend_log", { line: "x" });\n`,
  });
  const r1 = run("check-invoke-shape.py", sameName);
  check(
    "two apps with a same-named command are not cross-checked",
    r1.code === 0,
    `exit ${r1.code}: ${r1.out.trim()}`,
  );

  // An interpolating payload. The `}` that closes `${dir}` is not the end of the
  // object, and reading it as one made this call look argument-less.
  const interpolated = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[tauri::command]
pub fn read_dir(path: String) {}
`,
    "apps/alpha/src/lib/fs.ts":
      "await invoke(\"read_dir\", { path: `${dir}/sub` });\n",
  });
  const r2 = run("check-invoke-shape.py", interpolated);
  check(
    "a payload containing ${...} is read to its real end",
    r2.code === 0 && r2.out.includes("1 invoke call(s) checked"),
    `exit ${r2.code}: ${r2.out.trim()}`,
  );

  // And the check must still CATCH a real mismatch, or the two passes above
  // could just mean it stopped looking.
  const wrong = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[tauri::command]
pub fn read_dir(path: String) {}
`,
    "apps/alpha/src/lib/fs.ts": `await invoke("read_dir", { dir: "/tmp" });\n`,
  });
  const r3 = run("check-invoke-shape.py", wrong);
  check(
    "a call passing the wrong key is still reported",
    r3.code === 1 && r3.out.includes("read_dir"),
    `exit ${r3.code}: ${r3.out.trim()}`,
  );

  for (const d of [sameName, interpolated, wrong]) rmSync(d, { recursive: true, force: true });
}

invokeShapeFixtures();

if (failures.length) {
  console.log(`\n${failures.length} fixture(s) failed: ${failures.join(", ")}`);
  process.exit(1);
}
console.log("\nevery kept input is still caught");
