// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the invoke-exists gate must catch, and what it must leave alone.
//
// This gate decides whether a control on screen has anything behind it: a Tauri
// command is reachable only inside the binary that registers it, so an app
// invoking a name its own host does not register throws on every press, and
// whatever the catch does is what the user gets. That makes both wrong answers
// expensive - a miss ships a dead button, a false alarm trains people to ignore
// the count - so both directions are pinned here.
//
// NOT covered, deliberately: the known-missing inventory and its stale-ENTRY
// guard. Those key off a hardcoded per-app table, so a fixture for them would pin
// this test to today's inventory and break every time the count legitimately
// changes. The guard is real (`check-invoke-exists.py` reports an entry whose call
// is gone); it just cannot be fixture-tested without coupling.
//
// COVERED since 5 September: the stale-REASON checks. Different thing, and the
// distinction is the point - those read FILES rather than the table, so they pin
// nothing about today's inventory and are tested at the bottom of this file.
//
// Run: node dev/scripts/test-check-invokes.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-invoke-exists.py");

const failures = [];

function tree(files) {
  const dir = mint("arlen-invoke-gate-");
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return dir;
}

function run(dir) {
  // Both streams on every path. Reading `execFileSync`'s return value catches
  // stdout alone, so a case asserting on something the gate writes to stderr while
  // still exiting 0 would silently compare against an empty string - and the sync
  // call additionally echoes the child's stderr here, printing a wall of red above
  // an EXPECTED failure. Found twice in sibling gate tests before being fixed here.
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

/// The same, with the uncalled listing turned on.
///
/// That direction is advisory: it prints under `ARLEN_LIST_UNCALLED` and never
/// fails the gate, so a case asserting on the exit code alone would pass whether
/// the scanner saw the call or not. Which is worse than no case - I wrote one
/// like that and it stayed green with the fix reverted.
function runListing(dir) {
  const r = spawnSync("python3", [GATE, dir], {
    encoding: "utf8",
    env: { ...process.env, ARLEN_LIST_UNCALLED: "1" },
  });
  return { code: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  cleanup(dir);
}

// Registration, not annotation. The gate reads the `generate_handler!` list
// because that is what makes a command reachable; a `#[tauri::command]` nobody
// registers is exactly the dead call this gate is for. My first version of this
// fixture annotated without registering and the gate correctly reported it - the
// test was wrong, not the gate.
const HOST = `#[tauri::command]
pub fn open_thing() -> Result<(), String> { Ok(()) }

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![open_thing])
        .run(tauri::generate_context!())
        .expect("run");
}
`;

// Annotated but never registered: still dead, and the gate must say so.
const HOST_UNREGISTERED = `#[tauri::command]
pub fn open_thing() -> Result<(), String> { Ok(()) }
`;

check(
  "a call whose host registers it passes",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code) => code === 0,
);

check(
  "a call with no handler fails and names the command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_missing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing"),
);

check(
  "a call registered in ANOTHER app's host still fails",
  // The binary boundary this gate exists for, and the one that had `topbar_items`
  // looking like a missing producer when the producer was written all along.
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": "// nothing registered here\n",
    "apps/other/package.json": "{}",
    "apps/other/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_thing"),
);

check(
  "a command annotated but never registered is still reported",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST_UNREGISTERED,
  }),
  (code, out) => code !== 0 && out.includes("open_thing"),
);

// Both drift directions on the inventory, using the real one: `apps/knowledge`
// carries entries, and an app named `knowledge` in a throwaway tree is measured
// against them. The direction that bit was the second one - the gate simply
// stopped counting a fixed command and the entry sat there forever, so the total
// read as debt that someone had already paid.
check(
  "an inventory entry whose command now exists is reported",
  tree({
    "apps/knowledge/package.json": "{}",
    "apps/knowledge/src/lib/x.ts": 'await invoke("knowledge_library");\n',
    "apps/knowledge/src-tauri/src/lib.rs": HOST.replace(/open_thing/g, "knowledge_library"),
  }),
  (code, out) => code !== 0 && out.includes("knowledge_library") && out.includes("now registers it"),
);

check(
  "an inventory entry with neither a call nor a command stays quiet",
  // Carried, still missing, still invoked: the ordinary state of the inventory,
  // which must not fail the check or the count would be unusable.
  tree({
    "apps/knowledge/package.json": "{}",
    "apps/knowledge/src/lib/x.ts": 'await invoke("knowledge_library");\n',
    "apps/knowledge/src-tauri/src/lib.rs": "// no host commands here\n",
  }),
  (code) => code === 0,
);

// The shape that inflated the uncalled count: the name is chosen at runtime and
// assigned, so the literal never sits inside the `invoke(` call. Settings' module
// store does this for real, and both of its commands were reported as called by
// nothing while being in daily use.
check(
  "a command reached through a variable is not reported as uncalled",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'const cmd = flag ? "open_thing" : "open_other";\nawait invoke(cmd, { id });\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && out.includes("0 registered command(s)"),
);

// And the boundary that keeps it safe: the same literals must NOT satisfy the
// missing-command check, or a discriminant string in an assignment would become
// an invoked command and fail a gate over nothing.
check(
  "a variable-borne name is not treated as a call that needs a command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'const cmd = kind === "builtin" ? "open_thing" : "open_thing";\nawait invoke(cmd);\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && !out.includes("builtin"),
);

// A wrapper call is a real call, and the two cases below are the pair that makes
// that claim mean something. The helper's own body carries the proof the ASSIGNED
// case above lacks: the literal reaches `invoke` as its first argument, so it is a
// command name or the app throws.
check(
  "a command reached through a wrapper is not reported as uncalled",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "async function send(cmd: string, args?: unknown) { await invoke(cmd, args); }\n" +
      'await send("open_thing", { id });\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && out.includes("0 registered command(s)"),
);

// The half that had to be argued for, because it breaks the one-way rule the
// assignment case needs: a typo inside a wrapper call has to FAIL. Left relaxing-
// only, this is a call that throws for every user while the gate reports nothing -
// and it is exactly the shape the scanner cannot see at the `invoke` site, so
// nothing else would catch it either. `fileref_open_with` in the harness was found
// by turning this on, having been throwing on every click of a shipped menu item.
check(
  "a typo inside a wrapper call is caught like any other missing command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "async function send(cmd: string, args?: unknown) { await invoke(cmd, args); }\n" +
      'await send("open_thnig", { id });\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 1 && out.includes("open_thnig"),
);

// The two ways the wrapper hop was blind, both found in the text editor and both
// green at the time. A first-argument rule and a stop-at-the-first-brace body scan
// each read three unregistered commands as no call at all, so the gate reported
// nothing about a review panel that throws on every button.
check(
  "a command at a wrapper's LATER parameter is followed too",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "async function drive(index: number, cmd: string) { await invoke(cmd, { index }); }\n" +
      'await drive(1, "open_thnig");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 1 && out.includes("open_thnig"),
);

check(
  "a wrapper that opens a block before its invoke is still followed",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "async function drive(cmd: string) {\n" +
      "  store.update((p) => { return p; });\n" +
      "  await invoke(cmd);\n" +
      "}\n" +
      'await drive("open_thnig");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 1 && out.includes("open_thnig"),
);

// The boundary for the position rule: an argument that is not at the forwarded
// parameter's index must not be harvested. Without this, a helper taking a label
// beside its command would turn every label into a command that needs a host.
check(
  "an argument beside the forwarded one is not read as a command",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "async function drive(cmd: string, label: string) { await invoke(cmd, { label }); }\n" +
      'await drive("open_thing", "Some Label");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && !out.includes("Some Label"),
);

check(
  "an app with no host at all does not crash the gate",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts": "export const x = 1;\n",
  }),
  (code) => code === 0,
);

// A template literal can hold a document rather than code. The text editor ships
// two demo files that way, one of them showing example Arlen code with an
// `invoke` in it - and the scanner read that sample as a call this binary makes,
// so it sat on the missing-command list for weeks as work nobody could finish.
check(
  "an invoke inside a template literal is sample text, not a call site",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "const DOC = `example:\n  await invoke(\"not_a_real_command\");\n`;\nexport default DOC;\n",
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code === 0 && !out.includes("not_a_real_command"),
);

// The other direction, so the blanking cannot quietly swallow real calls: a
// genuine invoke beside a template literal is still found.
check(
  "a real call next to a template literal is still seen",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      "const DOC = `await invoke(\"decoy\")`;\nawait invoke(\"open_missing\");\n",
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing") && !out.includes("decoy"),
);

// A NESTED generic. `invoke<ReadOutcome<FileEntry>>("x")` is a real shape here,
// and the pattern used to stop at the first `>` and match nothing - so the day one
// was written the call went invisible in BOTH directions: it stopped counting as a
// caller AND a typo inside it stopped being caught. Found because the file
// manager's own `files_list_location`, which it plainly calls, turned up on the
// nobody-invokes-this list.
check(
  "a nested generic on invoke is still a call site",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'await invoke<Outcome<Row>>("open_missing");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing"),
);

// A call routed through an IMPORTED helper. The wrapper finder walks one file,
// so a helper declared in another one is invisible to it - and the day
// `shellAction` took over a call, the gate reported that command as invoked by
// nobody and told me to delete the entry carrying it. Both directions pinned:
// the typo inside the helper call is still caught...
check(
  "a call through an imported invoke helper is still a call site",
  tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'import { shellAction } from "$lib/shellAction";\n'
      + 'await shellAction("open_missing", {}, "e.key");\n',
    "apps/demo/src-tauri/src/lib.rs": HOST,
  }),
  (code, out) => code !== 0 && out.includes("open_missing"),
);

// ...and a command reached ONLY through the helper counts as reached, which is
// the direction that was actually broken: the gate did not merely miss a typo,
// it named a live command as called by nobody.
{
  const dir = tree({
    "apps/demo/package.json": "{}",
    "apps/demo/src/lib/x.ts":
      'import { shellAction } from "$lib/shellAction";\n'
      + 'await shellAction("open_thing", {});\n',
    "apps/demo/src-tauri/src/lib.rs":
      "#[tauri::command]\nfn open_thing() {}\n"
      + "fn main() { tauri::generate_handler![open_thing]; }\n",
  });
  const { out } = runListing(dir);
  const ok = !out.includes("`open_thing`");
  console.log(`  ${ok ? "ok  " : "FAIL"} a command called only through the helper is not listed as uncalled`);
  if (!ok) failures.push({ name: "helper-only call counts as a call", out });
  cleanup(dir);
}

// Each falsifier fixture carries a well-formed app as well as the file under
// test. The gate lists `apps/` before anything else and raises on a tree without
// one, so a fixture holding only the portal file exercised the traceback instead
// of the check - and the control read as a gate bug until I ran it by hand.
const SOUND_APP = {
  "apps/demo/package.json": "{}",
  "apps/demo/src/lib/x.ts": 'await invoke("open_thing");\n',
  "apps/demo/src-tauri/src/lib.rs": HOST,
};

// A FALSIFIER THAT NAMES ITS OWN TEST, and whether the gate runs it. Two of the
// inventory reasons say in words what would make them false, and until 5 September
// nothing checked either - I caught myself repeating "all gated" across six reports
// from memory before measuring it by hand. These pin the measurement.
//
// Fixture-testable where the inventory table is not: they read files, not the
// per-app table, and the gate takes its root as an argument - so a tree with those
// files in it exercises them without coupling to today's inventory.
{
  const portal = "daemons/xdg-portal/dist/xdg-desktop-portal/portals/arlen.portal";
  check("the capture five stay quiet while the portal offers no ScreenCast",
    tree({ ...SOUND_APP, [portal]: "[portal]\nInterfaces=org.freedesktop.impl.portal.FileChooser;\n" }),
    (_code, out) => !out.includes("capture five"));

  check("and speak up the moment it does - the check that entry names for itself",
    tree({ ...SOUND_APP, [portal]: "[portal]\nInterfaces=org.freedesktop.impl.portal.ScreenCast;\n" }),
    (code, out) => code === 1 && out.includes("capture five"));
}

{
  // `set_bottle_config`'s reason: no measured value to write, FALSE WHEN a bottle
  // records a Wine version, DLL overrides or a window mode that can be read back.
  const src = "daemons/bottled/src/bottle.rs";
  check("set_bottle_config stays quiet while a bottle records none of those fields",
    tree({ ...SOUND_APP, [src]: "pub struct Bottle { pub id: String }\n" }),
    (_code, out) => !out.includes("set_bottle_config"));

  check("and speaks up when a bottle records one",
    tree({ ...SOUND_APP, [src]: "pub struct Bottle { pub wine_version: String }\n" }),
    (code, out) => code === 1 && out.includes("set_bottle_config"));

  // The tree is full of prose ABOUT these fields - the entry itself names all
  // three - so a checker keying on the word rather than the code would fire on
  // every comment that explains why the entry exists.
  check("a comment naming the field is prose about the gap, not the gap closing",
    tree({ ...SOUND_APP, [src]: "// The compat recipe would give a wine_version and a window_mode.\npub struct Bottle {}\n" }),
    (_code, out) => !out.includes("set_bottle_config"));
}

console.log(failures.length ? "\nsome cases regressed" : "\nboth directions hold");
process.exit(failures.length ? 1 : 0);
