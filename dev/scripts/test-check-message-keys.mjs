// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The control for the message-key check.
//
// The case that matters most is a key present in one locale and missing in
// another: it renders correctly for whoever wrote it and shows the raw key to
// everybody else, which is how it ships. The other cases pin the restraint - a
// dynamic key is not a finding, and neither is a catalogue entry nothing
// literally names, because 47 files in this tree build keys at runtime and
// reporting those would invite deleting a string an error path needs.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mint, cleanup } from "./lib/fixture.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GATE = join(ROOT, "dev/scripts/check-message-keys.py");

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

const catalogue = (en, de) =>
  `export const messages = {\n  en: {\n${en}\n  },\n  de: {\n${de}\n  },\n};\n`;

function run(files) {
  const dir = mint("message-keys-");
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  cleanup(dir);
  return { code: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

console.log("message keys:");

{
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue('    "a.hi": "Hi",', '    "a.hi": "Hallo",'),
    "apps/demo/src/routes/+page.svelte": '<p>{$t("a.hi")}</p>\n',
  });
  check("a key defined in every locale passes", r.code === 0);
}
{
  // The shape three keys reached the tree in: a mapper turning a daemon's refusal
  // token into a key, translated by whoever calls it, so the literal never sits
  // inside a `t(`.
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue(
      '    "a.hi.there": "Hi",',
      '    "a.hi.there": "Hallo",',
    ),
    "apps/demo/src/lib/stores/refusal.ts":
      'export function key(reason: string): string {\n' +
      '  if (reason === "gone") return "a.hi.missing";\n' +
      '  return "a.hi.there";\n' +
      "}\n",
  });
  check(
    "a key only ever RETURNED from a mapper is checked too",
    r.code === 1 && r.out.includes("a.hi.missing"),
  );
}
{
  // The bound on that rule: a returned string is only treated as a key when its
  // first segment is one this catalogue actually uses, so an ordinary dotted
  // string is not dragged in.
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue('    "a.hi": "Hi",', '    "a.hi": "Hallo",'),
    // A real use as well, or the gate refuses the whole tree for having read
    // nothing - which would make this case pass for the wrong reason.
    "apps/demo/src/routes/+page.svelte": '<p>{$t("a.hi")}</p>\n',
    "apps/demo/src/lib/stores/name.ts":
      'export function archive(): string {\n  return "backup.tar.gz";\n}\n',
  });
  check("a dotted string outside the catalogue's namespaces is not a key", r.code === 0);
}
{
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue('    "a.hi": "Hi",', '    "a.other": "X",'),
    "apps/demo/src/routes/+page.svelte": '<p>{$t("a.hi")}</p>\n',
  });
  check(
    "a key missing from one locale is caught, and names it",
    r.code === 1 && r.out.includes("missing from de"),
  );
}
{
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue('    "a.hi": "Hi",', '    "a.hi": "Hallo",'),
    "apps/demo/src/routes/+page.svelte": '<p>{$t("a.nope")}</p>\n',
  });
  check("a key no catalogue has is caught", r.code === 1 && r.out.includes("a.nope"));
}
{
  // A runtime key. The check cannot know what it resolves to, and guessing
  // would produce a finding nobody can act on.
  //
  // The literal alongside it is deliberate: a tree whose ONLY key is dynamic has
  // nothing for this check to read, and it says so (exit 2) rather than passing
  // - which is right, and would make this case pass for the wrong reason.
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue('    "a.hi": "Hi",', '    "a.hi": "Hallo",'),
    "apps/demo/src/routes/+page.svelte": '<p>{$t(err)}</p>\n<p>{$t("a.hi")}</p>\n',
  });
  check("a dynamic key beside a literal one is not a finding", r.code === 0);
}
{
  // The restraint that keeps this trustworthy: an entry nothing literally names
  // is NOT reported, because a dynamic use may well reach it.
  const r = run({
    "apps/demo/src/lib/i18n/messages.ts": catalogue(
      '    "a.hi": "Hi",\n    "a.unused": "U",',
      '    "a.hi": "Hallo",\n    "a.unused": "U",',
    ),
    "apps/demo/src/routes/+page.svelte": '<p>{$t("a.hi")}</p>\n',
  });
  check("an entry nothing literally names is left alone", r.code === 0);
}
{
  const r = run({ "README.md": "nothing here\n" });
  check("an empty tree refuses rather than passing", r.code === 2);
  check("and says nothing was read", r.out.includes("NOTHING WAS READ"));
}

console.log(failures ? `\n${failures} failure(s)` : "\nboth directions hold");
process.exit(failures ? 1 : 0);
