#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-plugin-surfaces-reached.py. The two ways of reaching a
// command are staged as the tree actually reaches them - a direct invoke, and a
// call through the TS wrapper - and the two ways of losing the census (a new
// unreached command, a carried one that no longer exists) both have to fail.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-plugin-surfaces-reached.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(files) {
  const root = mint("plugin-surfaces-");
  for (const [rel, body] of Object.entries(files ?? {})) {
    mkdirSync(join(root, dirname(rel)), { recursive: true });
    writeFileSync(join(root, rel), body);
  }
  return root;
}

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

// THE CENSUS IS READ, NOT COPIED. Its entries are about the real tree, so a
// fixture that registers only the command a case is about would trip the
// "carried but no longer registered" rule on every one of them. The control
// takes the list out of the check itself, registers all of it, and adds whatever
// the case needs on top - so the two rules stay separable and neither is
// restated here to drift.
const CARRIED = [...readFileSync(check, "utf8").matchAll(/^\s{4}"(\w+)":/gm)].map((m) => m[1]);

// A plugin registering the census plus the commands named, and a wrapper for each.
const plugin = (...extra) => makePlugin([...CARRIED, ...extra]);

const makePlugin = (cmds) => ({
  "sdk/tauri-plugin-shell/src/lib.rs":
    "fn build() {\n  .invoke_handler(tauri::generate_handler![\n" +
    cmds.map((c) => `    commands::${c},\n`).join("") +
    "  ])\n}\n",
  "sdk/tauri-plugin-shell/index.ts":
    'const PLUGIN = "plugin:arlen-shell";\n' +
    "export const surface = {\n" +
    cmds
      .map((c) => `  async ${c.replace(/_(\w)/g, (_, x) => x.toUpperCase())}(): Promise<void> {\n    return invoke(\`\${PLUGIN}|${c}\`);\n  },\n`)
      .join("") +
    "};\n",
});

console.log("check-plugin-surfaces-reached:");

{
  // A command nothing calls, and nothing in the census either. Deliberately a
  // name the census does not hold: a carried one would pass, which is the point
  // of carrying it.
  const root = tree({
    ...plugin("wallpaper_set"),
    "apps/thing/src/routes/+page.svelte": "<div></div>\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("wallpaper_set")
    ? ok("a command nothing reaches and nothing explains is caught")
    : bad("a command nothing reaches and nothing explains is caught", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // Reached by a DIRECT invoke - how the menus and the printer do it.
  const root = tree({
    ...plugin("wallpaper_set"),
    "apps/thing/src/lib/menu.ts":
      'await invoke("plugin:arlen-shell|wallpaper_set", { count: 3 });\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a direct invoke reaches it") : bad("a direct invoke reaches it", r.out);
  cleanup(root);
}

{
  // Reached through the TS WRAPPER - how the file manager's breadcrumb does it.
  const root = tree({
    ...plugin("wallpaper_set"),
    "apps/thing/src/lib/topbar.ts":
      'import { surface } from "@arlen/tauri-plugin-shell";\nvoid surface.wallpaperSet();\n',
  });
  const r = run(root);
  r.code === 0 ? ok("a call through the wrapper reaches it") : bad("a call through the wrapper reaches it", r.out);
  cleanup(root);
}

{
  // The KIT counts, because a control it owns calls on every app's behalf -
  // which is how `theme_get` is reached and nothing else in the tree calls it.
  const root = tree({
    ...plugin("theme_get"),
    "sdk/ui-kit/src/lib/theme.ts":
      'const css = await invoke("plugin:arlen-shell|theme_get");\n',
  });
  const r = run(root);
  r.code === 0 ? ok("the kit calling on every app's behalf counts") : bad("the kit calling on every app's behalf counts", r.out);
  cleanup(root);
}

{
  // THE SHELL IS THE CONSUMER, not a producer: its own call must not clear the
  // census entry for a surface no app has ever published into.
  const root = tree({
    ...plugin("wallpaper_set"),
    "apps/desktop-shell/src/lib/stores/appStateStores.ts":
      'await invoke("plugin:arlen-shell|wallpaper_set");\n',
    // An app that is not the shell, so the run has producer sources to read at
    // all and the case is about the exclusion rather than an empty tree.
    "apps/thing/src/routes/+page.svelte": "<div></div>\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("wallpaper_set")
    ? ok("the shell's own call does not count as an app reaching it")
    : bad("the shell's own call does not count as an app reaching it", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // A census entry for a command the plugin no longer registers.
  const root = tree({
    ...makePlugin(["menu_register"]),
    "apps/thing/src/lib/menu.ts": 'await invoke("plugin:arlen-shell|menu_register", {});\n',
  });
  const r = run(root);
  r.code === 1 && /no longer registers/.test(r.out)
    ? ok("a census entry that outlived its command is caught")
    : bad("a census entry that outlived its command is caught", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  const root = tree({});
  const r = run(root);
  r.code === 2 ? ok("a tree with no plugin is a non-run") : bad("a tree with no plugin is a non-run", `got ${r.code}`);
  cleanup(root);
}

if (failures) {
  console.log(`\n${failures} control(s) failed`);
  process.exit(1);
}
console.log("a shell surface cannot go unreached without saying what is true about it");
