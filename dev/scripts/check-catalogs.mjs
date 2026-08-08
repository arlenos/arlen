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
// The same map keyed by (file, locale) but holding the VALUES, for the
// untranslated-string pass below.
const values = new Map();

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
    if (!values.has(scope)) values.set(scope, new Map());
    let text;
    try {
      text = JSON.parse(`"${raw}"`);
    } catch {
      // A key whose value spans lines is not something this reader can see whole,
      // and guessing at it would report noise. Skip rather than mis-report.
      continue;
    }
    values.get(scope).set(id, text);
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

// The positive control. This gate has been wrong twice in the way that matters:
// it fed a string to a declared number and the formatter logged rather than threw,
// and separately it printed a complaint and exited 0. Both times it reported a
// healthy catalog. So before believing a clean run, prove it can still see a
// broken message - plant the two failures it has actually had and require both.
const CONTROLS = [
  // Syntactically invalid: an unclosed placeholder.
  "hello {$name",
  // The historical one: a numeric selector reached with a string. Caught only if
  // the `.input` type declaration is still being read.
  ".input {$n :number}\n.match $n\none {{one thing}}\n* {{{$n} things}}",
];
for (const [i, text] of CONTROLS.entries()) {
  let caught = false;
  try {
    const onError = (e) => {
      throw e;
    };
    const mf = new MessageFormat("en", text);
    // Deliberately the wrong shape: a string where the source declares a number.
    mf.format({ name: "x", n: "not a number" }, onError);
  } catch {
    caught = true;
  }
  if (!caught) {
    console.error(`the catalog check did not catch its own planted failure #${i + 1}:`);
    console.error(`  ${text.replace(/\n/g, " / ")}`);
    console.error("A clean result from this gate would mean nothing. Fix the check.");
    process.exit(2);
  }
}

if (!checked) {
  console.error("found no catalog messages; the check needs updating");
  process.exit(2);
}
// Parity, which is a different property from validity and was not checked.
//
// A key that exists in `en` and not in `de` compiles and formats perfectly - in
// `en`. The gate above reads "every locale it carries", and a one-locale key
// carries in the locale it has. Three such keys shipped in the desktop shell on
// 8 August, all three on surfaces whose whole point was telling the user the
// truth, and a German reader got the key or a fallback instead.
//
// The tree is at zero when this lands, so it is a ratchet rather than a backlog.
// If a half-finished translation ever needs to sit in the tree, that is a
// decision to take here, out loud, rather than by the check quietly not looking.
const byFile = new Map();
for (const [scope, ids] of seen) {
  const [file, locale] = scope.split("\u0000");
  if (!byFile.has(file)) byFile.set(file, new Map());
  byFile.get(file).set(locale, ids);
}
for (const [file, locales] of byFile) {
  if (locales.size < 2) continue;
  const union = new Set([...locales.values()].flatMap((s) => [...s]));
  // The reader above deliberately skips a key whose value spans lines, and one
  // file indents a key differently from its neighbours. Both make a key INVISIBLE
  // to `seen` while it is plainly present in the file, so a parity report built
  // on `seen` alone accuses a translation that exists. Confirm against the raw
  // text of that locale's own slice before saying anything - measured: without
  // this, three entries were reported and all three were there.
  const text = readFileSync(file, "utf8");
  const slice = (loc) => {
    const start = text.indexOf(`\n  ${loc}: {`);
    if (start === -1) return "";
    const rest = text.slice(start + 1);
    const end = rest.search(/\n  [a-z]{2}(?:-[A-Z]{2})?: \{/);
    return end === -1 ? rest : rest.slice(0, end);
  };
  for (const [locale, ids] of locales) {
    const raw = slice(locale);
    for (const id of union) {
      if (!ids.has(id) && !raw.includes(`"${id}":`)) {
        broken.push(
          `${file.slice(ROOT.length)} [${locale}] ${id}: present in another locale and missing here, so this locale falls back to the key or the source language`,
        );
      }
    }
  }
}

// A `de` value byte-identical to its `en` one is usually fine - "CPU", "Bluetooth",
// a version line, a message that is only a number format - and occasionally it is
// an English sentence sitting in the German block that nobody noticed, because
// every other check is satisfied by the key being present.
//
// That happened on 9 August: `s.wallpaper.unavailable` was untranslated English in
// `de`, and a script adding a neighbouring key keyed off "does this line look
// German" and got it wrong, so the new sentence landed in English too. Two
// untranslated strings, one gate that had nothing to say.
//
// The threshold is four words. Below it the identical values are overwhelmingly
// real (measured: 124 identical pairs across the tree, 121 of them under four
// words and every one legitimate), and above it a match is a sentence somebody
// forgot. The three known long ones are named, because a check that reports them
// every run teaches people to ignore it.
const SHARED_BY_DESIGN = new Set([
  "sh.audio.appCount", // the plural arms are "Apps (n)" in both languages
  "s.display.refreshHz", // a number format and the unit "Hz"
  "k.about.build", // "Arlen OS · {$version}"
]);
for (const [file, locales] of byFile) {
  const en = values.get(`${file}\u0000en`);
  if (!en) continue;
  for (const locale of locales.keys()) {
    if (locale === "en") continue;
    const other = values.get(`${file}\u0000${locale}`);
    if (!other) continue;
    for (const [id, text] of other) {
      if (SHARED_BY_DESIGN.has(id)) continue;
      const source = en.get(id);
      if (source === undefined || source !== text) continue;
      if (text.trim().split(/\s+/).length < 4) continue;
      broken.push(
        `${file.slice(ROOT.length)} [${locale}] ${id}: identical to the English, word for word - a sentence this long is a translation nobody wrote, not a term both languages share`,
      );
    }
  }
}

if (broken.length) {
  // Not only formatting any more: duplicates, missing keys in a locale and
  // untranslated sentences arrive here too.
  console.log("catalog problems:\n");
  for (const b of broken) console.log(`  - ${b}`);
  process.exit(1);
}
console.log(`${checked} catalog message(s) compile and format`);
