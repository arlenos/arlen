// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Compile and format every message in every catalog, in every locale it carries.
//
// A catalog entry is a MessageFormat 2.0 source string living inside a TypeScript
// string literal, so `svelte-check` and `tsc` see a `string` and are satisfied. The
// selector syntax inside it is never parsed until the message is first formatted -
// which, for a locale nobody on the team reads, is in front of a user. A missing
// `.match`, an unbalanced brace, a plural arm the language needs and the source
// lacks: all green until then.
//
// This compiles each one and formats it with a plausible argument for every
// parameter it names, twice where it selects on a count so both the one and the
// other arm are exercised. It is deliberately dumber than a parser: it reads the
// catalogs as text rather than importing them, so it needs no build step and cannot
// be defeated by a catalog that fails to compile for an unrelated reason.

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const ROOT = new URL("../..", import.meta.url).pathname;

// `dev/scripts` has no `node_modules` of its own, and node resolves a bare import
// from the importing FILE, not the working directory. So find the copy the apps
// already depend on and load it by path - no new install, and it is by
// construction the same version the UI runs.
async function loadMessageFormat() {
  // The repo root comes first: CI's structural-checks job has no app
  // node_modules, so it installs this one package at the version the apps
  // declare. A developer running this locally hits an app copy instead, which is
  // the same version by construction.
  const homes = ["", "sdk/ui-kit", "apps/settings", "apps/system-monitor", "apps/store"];
  for (const home of homes) {
    const entry = join(ROOT, home, "node_modules/messageformat/lib/index.js");
    if (existsSync(entry)) return (await import(pathToFileURL(entry).href)).MessageFormat;
  }
  console.error("messageformat is not installed anywhere; run npm install in an app first");
  process.exit(2);
}
const MessageFormat = await loadMessageFormat();

/// Every `messages*.ts` under an app's or the kit's `lib/i18n`.
function catalogFiles() {
  const out = [];
  // A directory argument scans that tree instead of the repo's; only the fixture
  // runner passes one. This gate was red all evening for the right reason (it
  // refuses when it cannot check), but nothing had ever shown it going RED for a
  // catalog that genuinely does not format - which is the case it exists for.
  const bases = process.argv[2] ? [process.argv[2]] : [join(ROOT, "apps"), join(ROOT, "sdk")];
  for (const base of bases) {
    if (!existsSync(base)) continue;
    for (const app of readdirSync(base)) {
      const dir = join(base, app, "src/lib/i18n");
      if (!existsSync(dir)) continue;
      for (const f of readdirSync(dir)) {
        if (f.startsWith("messages") && f.endsWith(".ts")) out.push(join(dir, f));
      }
    }
  }
  return out;
}

let checked = 0;
const broken = [];
// A duplicate id is invisible to everything else: this reader takes the last one,
// the bundler's object literal takes the last one, and the app silently shows that
// message wherever the earlier id was used. It cost a real one - a new
// `h.mint.done` sentence landed on top of the existing "Done" button label, and
// only `svelte-check` noticed. Seen ids are per (file, locale).
const seen = new Map();

for (const file of catalogFiles()) {
  let locale = null;
  for (const line of readFileSync(file, "utf8").split("\n")) {
    // `  en: {` / `  de: {` open a locale; a key line sits two levels in.
    const open = line.match(/^  ([a-z]{2}(?:-[A-Z]{2})?): \{/);
    if (open) {
      locale = open[1];
      continue;
    }
    const kv = line.match(/^    "([^"]+)": "(.*)",$/);
    if (!kv || !locale) continue;
    const [, id, raw] = kv;
    const scope = `${file}\u0000${locale}`;
    if (!seen.has(scope)) seen.set(scope, new Set());
    if (seen.get(scope).has(id)) {
      broken.push(`${file.slice(ROOT.length)} [${locale}] ${id}: duplicate id, the later one wins and the earlier use silently changes`);
    }
    seen.get(scope).add(id);
    let text;
    try {
      text = JSON.parse(`"${raw}"`);
    } catch {
      // A key whose value spans lines is not something this reader can see whole,
      // and guessing at it would report noise. Skip rather than mis-report.
      continue;
    }
    checked++;
    // The source declares its own types (`.input {$n :number}`), so read them
    // rather than guessing from the parameter's name: a guess feeds a numeric
    // selector a string and reports the checker's mistake as the catalog's.
    // `[^}]*` because MF2 allows options after the function name
    // (`:number minimumFractionDigits=2`). Requiring `:number}` exactly made the
    // checker feed a string to a declared number, and the formatter logged that and
    // carried on, so the run still said the catalog was fine.
    const numeric = new Set(
      [...text.matchAll(/\.input\s*\{\$(\w+)\s*:number[^}]*\}/g)].map((m) => m[1]),
    );
    const params = {};
    for (const p of text.matchAll(/\{\$(\w+)[\s:}]/g)) {
      params[p[1]] = numeric.has(p[1]) ? 1 : "x";
    }
    try {
      // Formatting errors are reported to an `onError` callback and otherwise only
      // logged, so without this the gate prints a complaint to stderr and exits 0.
      const onError = (e) => {
        throw e;
      };
      const mf = new MessageFormat(locale, text);
      mf.format(params, onError);
      // Exercise the other plural arm too: a source that names only `one` still
      // formats fine at 1 and fails the user at 2.
      for (const n of numeric) mf.format({ ...params, [n]: 2 }, onError);
    } catch (e) {
      broken.push(`${file.slice(ROOT.length)} [${locale}] ${id}: ${e.message}`);
    }
  }
}

if (!checked) {
  console.error("found no catalog messages; the check needs updating");
  process.exit(2);
}
if (broken.length) {
  console.log("catalog messages that do not format:\n");
  for (const b of broken) console.log(`  - ${b}`);
  process.exit(1);
}
console.log(`${checked} catalog message(s) compile and format`);
