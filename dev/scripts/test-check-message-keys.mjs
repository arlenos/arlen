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
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

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
  const dir = mkdtempSync(join(tmpdir(), "message-keys-"));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(dir, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, body);
  }
  const r = spawnSync("python3", [GATE, dir], { encoding: "utf8" });
  rmSync(dir, { recursive: true, force: true });
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
