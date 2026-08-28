// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The capabilities checker, checked: it must FAIL on the shape it was written
// for and pass on the shape that is fine. A gate nobody has seen fail is a gate
// nobody knows the polarity of.

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const checker = join(here, "check-app-capabilities.py");
let failed = 0;

function run(root) {
  try {
    return { code: 0, out: execFileSync("python3", [checker, root], { encoding: "utf8" }) };
  } catch (e) {
    return { code: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
}

function say(name, ok, detail) {
  if (ok) console.log(`  ok   ${name}`);
  else {
    console.log(`  FAIL ${name}\n       ${detail}`);
    failed = 1;
  }
}

function app(root, name, { capabilities } = {}) {
  mkdirSync(join(root, "apps", name, "src-tauri"), { recursive: true });
  mkdirSync(join(root, "apps", name, "src/lib/i18n"), { recursive: true });
  writeFileSync(join(root, "apps", name, "src/lib/i18n/messages.ts"), "export const t = 0;\n");
  if (capabilities !== undefined) {
    mkdirSync(join(root, "apps", name, "src-tauri/capabilities"), { recursive: true });
    writeFileSync(
      join(root, "apps", name, "src-tauri/capabilities/default.json"),
      capabilities,
    );
  }
}

console.log("check-app-capabilities:");

// The shape this exists for: an app with a backend and no capabilities at all.
const bare = mint("caps-bare-");
app(bare, "winebottle");
let r = run(bare);
say(
  "an app with no capabilities file fails",
  r.code === 1 && r.out.includes("denied every"),
  `code ${r.code}: ${r.out}`,
);

// The same tree with the file present passes, so the gate is about the file and
// not about the app.
const fixed = mint("caps-fixed-");
app(fixed, "winebottle", {
  capabilities: JSON.stringify({ identifier: "default", permissions: ["core:default"] }),
});
r = run(fixed);
say("and passes once the file is there", r.code === 0, `code ${r.code}: ${r.out}`);

// A capabilities file that will not parse is worse than none, because the app
// looks configured. It has to be named rather than skipped.
const broken = mint("caps-broken-");
app(broken, "winebottle", { capabilities: "{ not json" });
r = run(broken);
say(
  "a capabilities file that will not parse is reported",
  r.code === 1 && r.out.includes("will not parse"),
  `code ${r.code}: ${r.out}`,
);

// An empty tree must say it read nothing rather than pass, which is how a
// checker silently stops covering anything after a directory move.
const empty = mint("caps-empty-");
mkdirSync(join(empty, "apps"), { recursive: true });
r = run(empty);
say(
  "an empty tree refuses rather than passing",
  r.code === 2 && r.out.includes("NOTHING WAS READ"),
  `code ${r.code}: ${r.out}`,
);

for (const d of [bare, fixed, broken, empty]) cleanup(d);
process.exit(failed);
