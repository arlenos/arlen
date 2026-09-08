// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the unheard-event check: watch it fail on each shape it
// claims to catch, and pass on the shapes it must not.
//
// Fixture trees, not the repo, so it keeps working as the carried list shrinks.
// The repo is asked one thing at the end: that the scan reads it at all.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHECK = join(ROOT, "dev/scripts/check-tauri-events-heard.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/** One app: `front` under its `src/`, `rust` under its `src-tauri/src/`, plus
 *  optional shared `sdk/` Rust. */
function tree({ front, rust = "", sdk = null }) {
  const root = mint("tauri-events-");
  const app = join(root, "apps/example");
  mkdirSync(join(app, "src/lib"), { recursive: true });
  mkdirSync(join(app, "src-tauri/src"), { recursive: true });
  writeFileSync(join(app, "src/lib/thing.ts"), front);
  writeFileSync(join(app, "src-tauri/src/lib.rs"), rust);
  if (sdk !== null) {
    mkdirSync(join(root, "sdk/kit/src"), { recursive: true });
    writeFileSync(join(root, "sdk/kit/src/lib.rs"), sdk);
  }
  return root;
}

function run(root) {
  const r = spawnSync("python3", [CHECK, root], { encoding: "utf8" });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

const LISTENS = `import { listen } from "@tauri-apps/api/event";
listen("arlen://thing-changed", () => {});
`;

// The tile shape: a listener whose name nothing emits.
{
  const root = tree({ front: LISTENS, rust: `fn main() { app.emit("thing-changed", ()); }` });
  const { code, out } = run(root);
  check(
    "a listener with a name nothing sends fails",
    code === 1 && out.includes("arlen://thing-changed"),
  );
  cleanup(root);
}

// The ordinary wired case.
{
  const root = tree({ front: LISTENS, rust: `fn main() { app.emit("arlen://thing-changed", ()); }` });
  const { code } = run(root);
  check("a listener the app's own Rust sends passes", code === 0);
  cleanup(root);
}

// Held in a const, which is how half the tree writes it. A matcher that only
// read `emit("literal")` would report every one of those.
{
  const root = tree({
    front: LISTENS,
    rust: `const EVENT: &str = "arlen://thing-changed";\nfn main() { app.emit(EVENT, ()); }`,
  });
  const { code } = run(root);
  check("a name held in a const counts as sent", code === 0);
  cleanup(root);
}

// Sent by the shared kit or plugin, which emits into every app's webview.
{
  const root = tree({
    front: LISTENS,
    rust: `fn main() {}`,
    sdk: `const EVENT: &str = "arlen://thing-changed";`,
  });
  const { code } = run(root);
  check("a name the sdk sends counts too", code === 0);
  cleanup(root);
}

// The reverse direction is deliberately not this check's business: a shell that
// emits for a surface not yet built is ordinary.
{
  // A listener beside it, because an app with no listener at all is a tree the
  // scan reports as pointed wrong rather than as clean - and that is the right
  // answer for a repo, so the case is written the way it occurs.
  const root = tree({
    front: LISTENS,
    rust: `fn main() {
      app.emit("arlen://thing-changed", ());
      app.emit("arlen://nobody-hears", ());
    }`,
  });
  const { code } = run(root);
  check("an emitted event nobody hears is not a failure", code === 0);
  cleanup(root);
}

// A carried count that is too high has to come down, the same ratchet the
// sibling checks run: a stale allowance is a place a new one hides.
{
  const r = spawnSync("python3", [CHECK], { encoding: "utf8" });
  const out = (r.stdout || "") + (r.stderr || "");
  check("the repo scan reads the tree", r.status === 0 && out.includes("app(s) scanned"));
}

console.log(failures === 0 ? "all ok" : `${failures} failure(s)`);
process.exit(failures === 0 ? 0 : 1);
