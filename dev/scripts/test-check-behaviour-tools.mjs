#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-behaviour-tools.py: put the fault back and watch it fail.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-behaviour-tools.py");
let failures = 0;
const ok = (name) => console.log(`  ok   ${name}`);
const bad = (name, detail) => {
  console.log(`  FAIL ${name}`);
  console.log(`       ${detail}`);
  failures += 1;
};

const skill = (tools) =>
  `---\nname: b\ndescription: d\nkind: agent\nreads: project\nmode: suggest\n` +
  `trigger:\n  type: event\n  event: e\ntools:\n${tools}` +
  `budget:\n  max_steps: 1\n  max_tokens: 1\n  max_wall_ms: 1\n` +
  `terminal:\n  done: silent\n---\nBody.\n`;

const proxy = `export const DEFAULT_PROXY_TOOLS = [\n  { name: "graph.read" },\n  { name: "graph.write" },\n];\n`;

function tree(tools, proxySrc = proxy) {
  const root = mkdtempSync(join(tmpdir(), "behaviour-tools-"));
  mkdirSync(join(root, "ai", "ai-skills", "behaviours", "b"), { recursive: true });
  writeFileSync(join(root, "ai", "ai-skills", "behaviours", "b", "SKILL.md"), skill(tools));
  mkdirSync(join(root, "ai", "pi-plugins", "src"), { recursive: true });
  writeFileSync(join(root, "ai", "pi-plugins", "src", "proxy.ts"), proxySrc);
  return root;
}

// The check derives its paths from its own location, so it is copied beside a
// fake tree rather than pointed at one.
function run(root) {
  const copied = join(root, "dev", "scripts");
  mkdirSync(copied, { recursive: true });
  writeFileSync(join(copied, "check-behaviour-tools.py"), readCheck());
  try {
    execFileSync("python3", [join(copied, "check-behaviour-tools.py")], { encoding: "utf8" });
    return 0;
  } catch (e) {
    return e.status ?? 1;
  }
}

function readCheck() {
  return execFileSync("cat", [check], { encoding: "utf8" });
}

{
  const rc = run(tree("  graph.read: []\n"));
  rc === 0 ? ok("a registered tool passes") : bad("a registered tool passes", `expected 0, got ${rc}`);
}

{
  // The case this exists for: a name the plugin never registers.
  const rc = run(tree("  graph.invented: []\n"));
  rc === 1
    ? ok("a privileged tool nothing registers is caught")
    : bad("a privileged tool nothing registers is caught", `expected 1, got ${rc}`);
}

{
  // A generic tool is the engine's own business, so this says nothing about it
  // rather than guessing.
  const rc = run(tree("  web.search: []\n"));
  rc === 0
    ? ok("a generic tool is not this check's business")
    : bad("a generic tool is not this check's business", `expected 0, got ${rc}`);
}

{
  // A carried mismatch passes, because the whole point of the baseline is that
  // the tree as it stands is describable rather than red.
  const rc = run(tree("  graph.query: []\n"));
  rc === 0
    ? ok("a mismatch with a written reason is carried, not failed")
    : bad("a mismatch with a written reason is carried, not failed", `expected 0, got ${rc}`);
}

{
  // Reading nothing must not read as a pass.
  const root = mkdtempSync(join(tmpdir(), "behaviour-tools-empty-"));
  mkdirSync(join(root, "ai", "ai-skills", "behaviours"), { recursive: true });
  mkdirSync(join(root, "ai", "pi-plugins", "src"), { recursive: true });
  writeFileSync(join(root, "ai", "pi-plugins", "src", "proxy.ts"), proxy);
  const rc = run(root);
  rc === 2
    ? ok("finding no behaviour at all is not a pass")
    : bad("finding no behaviour at all is not a pass", `expected 2, got ${rc}`);
}

{
  const rc = (() => {
    try {
      execFileSync("python3", [check], { encoding: "utf8" });
      return 0;
    } catch (e) {
      return e.status ?? 1;
    }
  })();
  rc === 0 ? ok("the repository itself passes") : bad("the repository itself passes", `got ${rc}`);
}

console.log(
  failures === 0
    ? "a declared tool has to be one that exists"
    : `\n${failures} case(s) failed`,
);
process.exit(failures === 0 ? 0 : 1);
