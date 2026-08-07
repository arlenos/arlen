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

  // The helper-side shape, in a `.ts` file: a formatter that reads the locale
  // store itself. Three of these shipped in one week and each read correctly.
  const getLocale = tree({
    "app/src/lib/fmt.ts": `import { get } from "svelte/store";
import { locale } from "@arlen/ui-kit/i18n";

export function size(bytes: number): string {
  return new Intl.NumberFormat(get(locale)).format(bytes);
}
`,
  });
  const rLoc = run("check-i18n-reactivity.mjs", getLocale);
  check(
    "a helper reading get(locale) in its body is reported",
    rLoc.code === 1 && rLoc.out.includes("get(locale)"),
    `exit ${rLoc.code}: ${rLoc.out.trim()}`,
  );

  // Both allowed shapes must stay silent, or the rule is noise: a parameter
  // default is what it asks for, and an imperative read handed straight to a
  // command formats nothing.
  const allowed = tree({
    "app/src/lib/ok.ts": `import { get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { locale } from "@arlen/ui-kit/i18n";

export function size(bytes: number, loc = get(locale)): string {
  return new Intl.NumberFormat(loc).format(bytes);
}

export async function search(query: string): Promise<void> {
  await invoke("settings_search", {
    query,
    locale: get(locale),
  });
}
`,
  });
  const rOk = run("check-i18n-reactivity.mjs", allowed);
  check(
    "a parameter default and a read passed to invoke are not reported",
    rOk.code === 0,
    `exit ${rOk.code}: ${rOk.out.trim()}`,
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

  // A serde rename is what the field is CALLED on the wire, and the wire name is
  // the only one the frontend sees. Reading past the attribute compared the Rust
  // identifier against the TS spelling and reported two sides that agree.
  const renamed = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[derive(Serialize)]
pub struct Row {
    pub id: u32,
    #[serde(rename = "memMB")]
    pub mem_mb: f64,
}

#[tauri::command]
pub fn list_rows() -> Vec<Row> { vec![] }
`,
    "apps/alpha/src/lib/rows.ts": `export interface Row {
  id: number;
  memMB: number;
}
const rows = await invoke<Row[]>("list_rows");
`,
  });
  const r4 = run("check-invoke-shape.py", renamed);
  check(
    "a serde-renamed field is compared by its wire name",
    r4.code === 0,
    `exit ${r4.code}: ${r4.out.trim()}`,
  );

  // A nested object is ONE field of the outer interface. Reading to the first `}`
  // promoted the inner fields to the outer level and then reported the Rust side
  // for not producing them at a level where they do not belong.
  const nested = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[derive(Serialize)]
pub struct Info {
    pub conventional: Properties,
}

#[tauri::command]
pub fn app_info() -> Info { todo!() }
`,
    "apps/alpha/src/lib/info.ts": `export interface Info {
  conventional: {
    kind: string;
    size: number;
  };
}
const info = await invoke<Info>("app_info");
`,
  });
  const r5 = run("check-invoke-shape.py", nested);
  check(
    "a nested object counts as one field of the outer interface",
    r5.code === 0,
    `exit ${r5.code}: ${r5.out.trim()}`,
  );

  // And a genuinely missing top-level field must still be reported, so the two
  // passes above are not the parser having given up.
  const reallyMissing = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[derive(Serialize)]
pub struct Info {
    pub conventional: Properties,
}

#[tauri::command]
pub fn app_info() -> Info { todo!() }
`,
    "apps/alpha/src/lib/info.ts": `export interface Info {
  conventional: {
    kind: string;
  };
  owner: string;
}
const info = await invoke<Info>("app_info");
`,
  });
  const r6 = run("check-invoke-shape.py", reallyMissing);
  check(
    "a field the struct really lacks is still reported",
    r6.code === 1 && r6.out.includes("owner"),
    `exit ${r6.code}: ${r6.out.trim()}`,
  );

  // A command invoked with no implementation anywhere is a call that cannot
  // succeed, and the check used to skip exactly those - `cmd not in own` reads an
  // absent command as "not ours" and moves on, so it verified the argument shape
  // of calls into the void. Undeclared ones are now findings.
  const undeclared = tree({
    "apps/alpha/src-tauri/src/lib.rs": `#[tauri::command]
pub fn real_one() {}
`,
    "apps/alpha/src/lib/x.ts": `await invoke("real_one");
await invoke("no_such_command");
`,
  });
  const r7 = run("check-invoke-shape.py", undeclared);
  check(
    "a command invoked with no implementation is reported",
    r7.code === 1 && r7.out.includes("no_such_command"),
    `exit ${r7.code}: ${r7.out.trim()}`,
  );

  for (const d of [sameName, interpolated, wrong, renamed, nested, reallyMissing, undeclared]) {
    rmSync(d, { recursive: true, force: true });
  }
}

invokeShapeFixtures();

// ---------------------------------------------------------------------------
// check-catalogs.mjs
//
// This gate spent an evening red for the right reason - it refuses when it
// cannot check - but nothing had ever shown it going red for the case it exists
// for: a message whose MessageFormat 2.0 source does not compile or format. The
// selector syntax inside a catalog string is never parsed until the message is
// first formatted, which for a locale nobody on the team reads is in front of a
// user.
// ---------------------------------------------------------------------------
function catalogFixtures() {
  console.log("\ncheck-catalogs:");

  const good = tree({
    "alpha/src/lib/i18n/messages.ts": `const messages: Catalogs = {
  en: {
    "a.plain": "Save changes",
    "a.count": ".input {$n :number} .match $n one {{One file}} * {{{$n} files}}",
  },
  de: {
    "a.plain": "Änderungen speichern",
    "a.count": ".input {$n :number} .match $n one {{Eine Datei}} * {{{$n} Dateien}}",
  },
};
`,
  });
  const r1 = run("check-catalogs.mjs", good);
  check(
    "catalogs that compile and format pass",
    r1.code === 0 && r1.out.includes("compile and format"),
    `exit ${r1.code}: ${r1.out.trim()}`,
  );

  // A selector with no catch-all arm: legal-looking, and it throws the moment a
  // value falls outside the arms it does list. In the locale that has it.
  const broken = tree({
    "alpha/src/lib/i18n/messages.ts": `const messages: Catalogs = {
  en: {
    "a.count": ".input {$n :number} .match $n one {{One file}} * {{{$n} files}}",
  },
  de: {
    "a.count": ".input {$n :number} .match $n one {{Eine Datei}}",
  },
};
`,
  });
  const r2 = run("check-catalogs.mjs", broken);
  check(
    "a message that does not format is reported",
    r2.code === 1 && r2.out.includes("a.count"),
    `exit ${r2.code}: ${r2.out.trim()}`,
  );

  // And a tree with no catalogs at all must fail rather than pass: a gate that
  // silently checks nothing is the shape this whole file exists to prevent.
  const empty = tree({ "alpha/src/lib/notes.md": "no catalogs here\n" });
  const r3 = run("check-catalogs.mjs", empty);
  check(
    "finding no catalog messages fails rather than passing",
    r3.code === 2,
    `exit ${r3.code}: ${r3.out.trim()}`,
  );

  // A duplicate id: both lines are valid MessageFormat, so every check that looks
  // at one message at a time passes it. The later one wins, and whatever used the
  // earlier id silently starts showing the other text. This happened - a mint
  // sentence landed on top of a "Done" button label.
  const dup = tree({
    "alpha/src/lib/i18n/messages.ts": `const messages: Catalogs = {
  en: {
    "a.done": "Done",
    "a.other": "Something else",
    "a.done": "{$what} is now shared.",
  },
};
`,
  });
  const r4 = run("check-catalogs.mjs", dup);
  check(
    "a duplicate message id is reported",
    r4.code === 1 && r4.out.includes("a.done") && r4.out.includes("duplicate"),
    `exit ${r4.code}: ${r4.out.trim()}`,
  );

  for (const d of [good, broken, empty, dup]) rmSync(d, { recursive: true, force: true });
}

catalogFixtures();

if (failures.length) {
  console.log(`\n${failures.length} fixture(s) failed: ${failures.join(", ")}`);
  process.exit(1);
}
console.log("\nevery kept input is still caught");
