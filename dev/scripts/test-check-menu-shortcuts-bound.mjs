#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-menu-shortcuts-bound.py. The fault is staged as both of the
// real ones arrived - a File menu saying Ctrl+S over a window that binds only
// Ctrl+K, and a Message menu saying Ctrl+N over a window with no keydown at all
// - and every case that must PASS is a shape the tree actually carries, because
// the first cut of this matcher reported two correct apps and that is the
// failure worth guarding against.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-menu-shortcuts-bound.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

function tree(files) {
  const root = mint("menu-shortcuts-");
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

const menu = (shortcut) =>
  'import { invoke } from "@tauri-apps/api/core";\n' +
  "export function appMenuGroups(t) {\n" +
  "  return [{ label: t('m.file'), items: [\n" +
  `    { label: t('m.save'), action: "file.save", shortcut: "${shortcut}", type: "item" },\n` +
  "  ] }];\n}\n" +
  'export async function initAppMenu() {\n' +
  '  await invoke("plugin:arlen-shell|menu_register", { groups: appMenuGroups(t) });\n}\n';

console.log("check-menu-shortcuts-bound:");

{
  // A window that binds one key and advertises another.
  const root = tree({
    "apps/editor/src/lib/menu.ts": menu("Ctrl+S"),
    "apps/editor/src/routes/+page.svelte":
      "<script>\n  function onKeydown(e) {\n" +
      '    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") lens();\n' +
      "  }\n</script>\n<svelte:window onkeydown={onKeydown} />\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("Ctrl+S")
    ? ok("a menu advertising a key the window never binds is caught")
    : bad("a menu advertising a key the window never binds is caught", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // THE TEXT EDITOR'S REAL SHAPE, and the one the first cut of this matcher got
  // wrong: the key is bound BY NAME in an editor keymap, and no comparison
  // against "s" appears anywhere. Reporting this asks for a second handler on the
  // window, which saves twice per keystroke.
  const root = tree({
    "apps/editor/src/lib/menu.ts": menu("Ctrl+S"),
    "apps/editor/src/lib/components/Buffer.svelte":
      "<script>\n  const ext = keymap.of([\n" +
      '    { key: "Mod-s", preventDefault: true, run: () => { onsave?.(); return true; } },\n' +
      "  ]);\n</script>\n<div></div>\n",
  });
  const r = run(root);
  r.code === 0 ? ok("an editor keymap binding by name passes") : bad("an editor keymap binding by name passes", r.out);
  cleanup(root);
}

{
  // And the keymap form must not be so loose that any word ending in the key
  // counts: `key: "someones"` binds nothing called `s`.
  const root = tree({
    "apps/editor/src/lib/menu.ts": menu("Ctrl+S"),
    "apps/editor/src/lib/components/Buffer.svelte":
      '<script>\n  const ext = keymap.of([{ key: "someones", run: go }]);\n</script>\n<div></div>\n',
  });
  const r = run(root);
  r.code === 1
    ? ok("a keymap name merely ENDING in the key is not a binding")
    : bad("a keymap name merely ENDING in the key is not a binding", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // MAIL'S: no keydown handler in the app at all.
  const root = tree({
    "apps/mailer/src/lib/menu.ts": menu("Ctrl+N"),
    "apps/mailer/src/routes/+page.svelte": "<script>\n  let composing = false;\n</script>\n<div></div>\n",
  });
  const r = run(root);
  r.code === 1 && r.out.includes("Ctrl+N")
    ? ok("an app with no keyboard at all is caught")
    : bad("an app with no keyboard at all is caught", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // THE FIX. One binding is the whole difference.
  const root = tree({
    "apps/editor/src/lib/menu.ts": menu("Ctrl+S"),
    "apps/editor/src/routes/+page.svelte":
      "<script>\n  function onKeydown(e) {\n" +
      '    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") save();\n' +
      "  }\n</script>\n<svelte:window onkeydown={onKeydown} />\n",
  });
  const r = run(root);
  r.code === 0 ? ok("binding the key passes") : bad("binding the key passes", r.out);
  cleanup(root);
}

{
  // THE CALENDAR'S SHAPE, which the first matcher called seven false reds: the
  // key is read into a local first and the comparison never names `.key`.
  const root = tree({
    "apps/cal/src/lib/menu.ts": menu("W"),
    "apps/cal/src/routes/+page.svelte":
      "<script>\n  function globalKeys(e) {\n    const k = e.key;\n" +
      '    if (k === "w") view = "week";\n' +
      "  }\n</script>\n<svelte:window onkeydown={globalKeys} />\n",
  });
  const r = run(root);
  r.code === 0 ? ok("a key compared through a local passes") : bad("a key compared through a local passes", r.out);
  cleanup(root);
}

{
  // THE PDF READER'S: `+` is a key, not only the modifier separator. The first
  // matcher split it away and read an empty key.
  const root = tree({
    "apps/reader/src/lib/menu.ts": menu("+"),
    "apps/reader/src/routes/+page.svelte":
      "<script>\n  function onKey(event) {\n" +
      '    if (event.key === "+" || event.key === "=") zoom(1);\n' +
      "  }\n</script>\n<svelte:window onkeydown={onKey} />\n",
  });
  const r = run(root);
  r.code === 0 ? ok("`+` is read as the key it is, not as a separator") : bad("`+` is read as the key it is, not as a separator", r.out);
  cleanup(root);
}

{
  // The menu names `Del`; the handler names `Delete`. The same key.
  const root = tree({
    "apps/lister/src/lib/menu.ts": menu("Del"),
    "apps/lister/src/routes/+page.svelte":
      "<script>\n  function keydown(e) {\n" +
      '    if (e.key === "Delete" || e.key === "Backspace") remove();\n' +
      "  }\n</script>\n<div onkeydown={keydown}></div>\n",
  });
  const r = run(root);
  r.code === 0 ? ok("the menu's `Del` and the event's `Delete` are one key") : bad("the menu's `Del` and the event's `Delete` are one key", r.out);
  cleanup(root);
}

{
  // A file that does NOT register a menu makes no claim, so a `shortcut` field
  // in it - the keybindings surfaces carry them, naming a compositor binding -
  // is not this check's business.
  const root = tree({
    "apps/settingsish/src/lib/rows.ts": 'export const rows = [{ shortcut: "Ctrl+Q", label: "Quit" }];\n',
    "apps/settingsish/src/routes/+page.svelte": "<div></div>\n",
  });
  const r = run(root);
  r.code === 2
    ? ok("a tree with no menu registration at all is a non-run, not a pass")
    : bad("a tree with no menu registration at all is a non-run, not a pass", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  // An empty tree must be a non-run rather than a clean bill.
  const root = tree({});
  const r = run(root);
  r.code === 2 ? ok("an empty tree is a non-run") : bad("an empty tree is a non-run", `got ${r.code}`);
  cleanup(root);
}

if (failures) {
  console.log(`\n${failures} control(s) failed`);
  process.exit(1);
}
console.log("a menu cannot advertise a keystroke its app never learnt");
