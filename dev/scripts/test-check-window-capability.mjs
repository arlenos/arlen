// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// What the window-capability gate must catch, and what it must leave alone.
//
// The gate came back green on its first run against the real tree, which is the
// most dangerous result a new check can give: green because the tree is clean, or
// green because it matches nothing? So the first two cases here rebuild the exact
// bug it was written for - the shell's `consent` window, in both spellings - and
// require it to fail. The rest are the shapes that must stay quiet, because a
// check that cries about a run-time label or an app with no capabilities is a
// check people route around.
//
// Run: node dev/scripts/test-check-window-capability.mjs

import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";

const ROOT = new URL("../..", import.meta.url).pathname;
const GATE = join(ROOT, "dev/scripts/check-window-capability.py");

const failures = [];

function tree(files) {
  const dir = mkdtempSync(join(tmpdir(), "arlen-wincap-gate-"));
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

function check(name, dir, expect) {
  const { code, out } = run(dir);
  const ok = expect(code, out);
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures.push({ name, code, out });
  rmSync(dir, { recursive: true, force: true });
}

const CONF_MAIN = JSON.stringify({ app: { windows: [{}] } });
const CAP = (windows) =>
  JSON.stringify({ identifier: "default", windows, permissions: ["core:default"] });

check(
  "a literal label in no capability fails",
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["main"]),
    "apps/thing/src-tauri/src/lib.rs":
      'fn f() { WebviewWindowBuilder::new(app, "consent", url).build(); }\n',
  }),
  (code, out) => code !== 0 && out.includes("consent"),
);

check(
  "the real spelling - a const label - fails too",
  // This is how the shell writes it, and a gate that only understood string
  // literals would have called the tree clean while the consent window sat dead.
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["main"]),
    "apps/thing/src-tauri/src/consent_window.rs":
      'const LABEL: &str = "consent";\nfn f() { WebviewWindowBuilder::new(app, LABEL, url).build(); }\n',
  }),
  (code, out) => code !== 0 && out.includes("consent"),
);

check(
  "listing the label passes",
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["main", "consent"]),
    "apps/thing/src-tauri/src/lib.rs":
      'fn f() { WebviewWindowBuilder::new(app, "consent", url).build(); }\n',
  }),
  (code) => code === 0,
);

check(
  "a wildcard capability covers everything",
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["*"]),
    "apps/thing/src-tauri/src/lib.rs":
      'fn f() { WebviewWindowBuilder::new(app, "anything", url).build(); }\n',
  }),
  (code) => code === 0,
);

check(
  "a config window with no label is Tauri's `main`",
  // Tauri defaults an unlabelled window to `main`; reading the absent label as a
  // window called "" would fail every app in the tree.
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["main"]),
  }),
  (code) => code === 0,
);

check(
  "a run-time label is not guessed at",
  // Documented limit rather than a silent one: a label built from a variable
  // cannot be read here, and inventing a finding for it would train people to
  // ignore this gate.
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/capabilities/default.json": CAP(["main"]),
    "apps/thing/src-tauri/src/lib.rs":
      'fn f(id: &str) { WebviewWindowBuilder::new(app, format!("doc-{id}"), url).build(); }\n',
  }),
  (code) => code === 0,
);

check(
  "an app with no capabilities directory is out of scope",
  tree({
    "apps/thing/src-tauri/tauri.conf.json": CONF_MAIN,
    "apps/thing/src-tauri/src/lib.rs":
      'fn f() { WebviewWindowBuilder::new(app, "consent", url).build(); }\n',
  }),
  (code) => code === 0,
);

console.log(failures.length ? "\nsome cases regressed" : "\nevery shape holds");
process.exit(failures.length ? 1 : 0);
