#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Control for check-promoted-fields.py. The fault is staged as the timeline bug
// arrived - a handler that reads one field of a payload and never touches the
// one the app cared about - and the passing cases are the two ways a field is
// legitimately not on a node.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = join(here, "check-promoted-fields.py");
let failures = 0;
const ok = (n) => console.log(`  ok   ${n}`);
const bad = (n, d) => { console.log(`  FAIL ${n}`); console.log(`       ${d}`); failures += 1; };

// The census is about the real tree, so a fixture registering its own handler
// would trip the "explains something that no longer happens" rule on every
// entry. The control reads the list out of the check and gives each entry a
// handler and a proto field to belong to, then adds whatever the case is about.
const CARRIED = [...readFileSync(check, "utf8").matchAll(/^\s{4}"(promote_\w+)\.(\w+)":/gm)]
  .map((m) => ({ handler: m[1], field: m[2] }));

function tree(handlers) {
  const root = mint("promoted-fields-");
  // One proto message per handler, holding every field the case names.
  const byMessage = new Map();
  let rust = "";
  for (const h of handlers) {
    const message = `${h.handler.replace(/^promote_/, "")}Payload`;
    byMessage.set(message, h.fields);
    const reads = h.reads.map((f) => `    let _ = p.${f};`).join("\n");
    rust += `async fn ${h.handler}(payload: &[u8]) -> Result<()> {\n    let p = ${message}::decode(payload)?;\n${reads}\n    Ok(())\n}\n\n`;
  }
  let proto = 'syntax = "proto3";\n';
  for (const [message, fields] of byMessage) {
    proto += `message ${message} {\n` + fields.map((f, i) => `  string ${f} = ${i + 1};`).join("\n") + "\n}\n";
  }
  mkdirSync(join(root, "daemons/knowledge/src"), { recursive: true });
  mkdirSync(join(root, "daemons/knowledge/proto"), { recursive: true });
  writeFileSync(join(root, "daemons/knowledge/src/promotion.rs"), rust);
  writeFileSync(join(root, "daemons/knowledge/proto/event.proto"), proto);
  return root;
}

// Every carried entry, satisfied, so only the case under test can fail.
const census = () => {
  const byHandler = new Map();
  for (const { handler, field } of CARRIED) {
    if (!byHandler.has(handler)) byHandler.set(handler, { handler, fields: [], reads: ["ignored"] });
    byHandler.get(handler).fields.push(field);
  }
  for (const h of byHandler.values()) h.fields.push("ignored");
  return [...byHandler.values()];
};

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [check, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

console.log("check-promoted-fields:");

{
  // THE TIMELINE BUG'S SHAPE: the handler reads the label and never the subject.
  const root = tree([
    ...census(),
    { handler: "promote_thing", fields: ["label", "subject"], reads: ["label"] },
  ]);
  const r = run(root);
  r.code === 1 && r.out.includes("promote_thing.subject")
    ? ok("a field the app sends and the graph never reads is caught")
    : bad("a field the app sends and the graph never reads is caught", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  const root = tree([
    ...census(),
    { handler: "promote_thing", fields: ["label", "subject"], reads: ["label", "subject"] },
  ]);
  const r = run(root);
  r.code === 0 ? ok("reading it passes") : bad("reading it passes", r.out);
  cleanup(root);
}

{
  // An entry for a field the handler now reads has to come out, the same rule
  // the shell-surface census follows: a stale explanation is where the next
  // real one hides.
  const first = CARRIED[0];
  const root = tree([
    ...census().map((h) =>
      h.handler === first.handler ? { ...h, reads: [...h.reads, first.field] } : h,
    ),
  ]);
  const r = run(root);
  r.code === 1 && /no longer happens/.test(r.out)
    ? ok("an explanation for a field that is read now has to leave")
    : bad("an explanation for a field that is read now has to leave", `got ${r.code}: ${r.out}`);
  cleanup(root);
}

{
  const root = mint("promoted-fields-empty-");
  const r = run(root);
  r.code === 2 ? ok("a tree with no promotion pass is a non-run") : bad("a tree with no promotion pass is a non-run", `got ${r.code}`);
  cleanup(root);
}

if (failures) {
  console.log(`\n${failures} control(s) failed`);
  process.exit(1);
}
console.log("a fact an app sends cannot vanish between the bus and the graph unnoticed");
