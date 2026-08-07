// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// Catch the two ways to route a string through the message catalog and still ship the
// source language forever.
//
// Both compile. Both typecheck. Both render correctly in English, which is the only
// locale most of us look at. Neither survives a locale switch:
//
//   const OPTIONS = [{ label: $t("k") }]     a top-level constant is evaluated once at
//                                            import and captures whatever the
//                                            translator held then. Fix: `$derived([...])`.
//
// What matters is WHEN `$t` is called, not where the declaration sits. A const whose
// initialiser is a FUNCTION is fine - `const f = (n) => $t("k")` reads the store when
// the markup calls it, inside that reaction, so it re-renders on a locale switch. This
// check flagged one of those and turned CI red over correct code; wrapping it in
// `$derived` would have silenced the check without creating a dependency, since a
// derived that only builds a closure reads nothing while it runs.
//
//   function label() { return get(t)("k") }  `get()` reads the store imperatively, so it
//                                            is not a tracked dependency and the markup
//                                            calling the function never re-renders.
//                                            Fix: return the KEY, call `$t` in markup.
//
// The third shape, `{$t("k")}` straight in markup, is the correct one and is left alone.
//
// Two things this deliberately does not flag. A declaration inside a function body is
// re-evaluated on every call, so it is not frozen; only top-level ones are. And the
// `$derived` test is done on the captured initialiser rather than as an inline negative
// lookahead: written `=\s*(?!\$derived)` the engine backtracks `\s*` to zero width and
// the lookahead passes, which is exactly how the first version of this check reported
// three correctly-wrapped tables as broken.

import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("../..", import.meta.url).pathname;

/// Every `.svelte` file under a source tree, skipping build output and dependencies.
function svelteFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === ".svelte-kit" || name === "build") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) svelteFiles(path, out);
    else if (name.endsWith(".svelte")) out.push(path);
  }
  return out;
}

/// Every `.ts` file, for the helper-side shape. Tests are excluded: a test may
/// read the store on purpose to assert what it holds.
function tsFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === ".svelte-kit" || name === "build") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) tsFiles(path, out);
    else if (name.endsWith(".ts") && !name.endsWith(".test.ts") && !name.endsWith(".d.ts"))
      out.push(path);
  }
  return out;
}

/// Whether this `get(locale)` is the allowed form: a parameter default.
///
/// `formatDecimal(value, digits, loc = get(locale))` is the shape the rule asks
/// for - the caller passes `$locale` to get a dependency, and the default is
/// there for callers outside a reactive context. What must go is the read inside
/// a body, which is invisible to the compiler and to the reader.
///
/// A default ends the expression with `,` or `)` and is preceded by `=`; a read
/// inside a body is preceded by `return`, `=` at statement level, or a call
/// paren, and followed by `;` or an operator. Distinguishing those two is enough
/// without parsing, and getting it wrong in the permissive direction only means
/// the rule misses one - it never invents one.
/// Whether this read hands the locale to a backend call rather than formatting
/// with it.
///
/// `invoke("settings_search", { query, locale: get(locale) })` is the right
/// shape: the value is consumed at once by a command, nothing rendered is frozen,
/// and reading the store imperatively is exactly what an action wants. The bug is
/// a function that FORMATS by locale and reads the store itself - the result is
/// held in the markup and never recomputed.
function isCallArgument(lines, i) {
  // The statement around the read: back to the last line that ended one, forward
  // to the next. Cheap, and enough to see the `invoke(` that owns it.
  let start = i;
  // A line ending in `{` is the statement continuing into an object argument,
  // which is where `locale: get(locale)` usually sits - stopping there would cut
  // the read off from the `invoke(` that owns it.
  while (start > 0 && !/[;}]\s*$/.test(lines[start - 1])) start--;
  let end = i;
  while (end < lines.length - 1 && !/[;]\s*$/.test(lines[end])) end++;
  return /\binvoke\s*[<(]/.test(lines.slice(start, end + 1).join("\n"));
}

function isParameterDefault(line, index) {
  const before = line.slice(0, index);
  const after = line.slice(index).replace(/^get\s*\(\s*locale\s*\)/, "");
  return /=\s*$/.test(before) && /^\s*[,)]/.test(after);
}

const problems = [];
let scanned = 0;
let tsScanned = 0;

// A directory argument scans that tree instead of the repo's. Only the fixture
// runner passes one; CI passes nothing and gets the trees below. The check has
// twice been wrong about which shapes it sees, so it needs to be runnable
// against inputs chosen to fool it.
const BASES = process.argv[2]
  ? [process.argv[2]]
  : [join(ROOT, "apps"), join(ROOT, "sdk")];

for (const base of BASES) {
  if (!existsSync(base)) continue;
  for (const file of svelteFiles(base)) {
    const text = readFileSync(file, "utf8");
    const script = text.match(/<script[^>]*>([\s\S]*?)<\/script>/);
    if (!script) continue;
    scanned++;
    const lines = script[1].split("\n");
    const rel = relative(ROOT, file);

    lines.forEach((line, i) => {
      if (/\bget\s*\(\s*t\s*\)\s*\(/.test(line)) {
        problems.push(
          `${rel}:${i + 2}: reads the translator with get(t), which is not a tracked ` +
            `dependency, so this text keeps whichever locale rendered first. Return the ` +
            `message key and call $t in the markup.`,
        );
      }

      // Top level in a Svelte `<script>` is two spaces; a function body is deeper.
      // The `=` may be several lines down when the type annotation is a multi-line
      // generic, so a declaration without one on its own line is still a candidate:
      // that shape is how a `Record<\n  Kind,\n  {...}\n> = {` table slipped past
      // the first version of this check.
      const decl = line.match(/^ {2}(?:const|let)\s+(\w+)[^=]*=\s*(.*)$/)
        ?? line.match(/^ {2}(?:const|let)\s+(\w+)\s*:[^=]*$/)?.concat([""]);
      if (!decl) return;
      const rest = lines.slice(i + 1, i + 30).join("\n");
      const initialiser = decl[2]
        ? [decl[2], rest].join("\n")
        : rest.slice(rest.indexOf("=") + 1);
      // `$derived` may sit on the next line when the type annotation is long.
      if (initialiser.trimStart().startsWith("$derived")) return;
      // A function initialiser DEFERS its `$t` reads to the call, and the call
      // happens inside the markup's reaction, so the store read is tracked there
      // and the text does follow a locale switch. Only an initialiser that calls
      // `$t` at declaration time captures anything. Measured rather than argued:
      // `const f = (n) => $t(k)` re-renders on a locale change and
      // `const s = $t(k)` does not, verified by mounting both and switching.
      if (/^\s*(?:async\s+)?(?:function\b|\(|<|[\w$]+\s*=>)/.test(initialiser)) return;
      const end = initialiser.indexOf(";");
      if (!(end === -1 ? initialiser : initialiser.slice(0, end)).includes("$t(")) return;
      problems.push(
        `${rel}:${i + 2}: \`${decl[1]}\` builds text with $t outside $derived - a ` +
          `top-level constant captures the locale at import and never updates. Wrap the ` +
          `initialiser in $derived(...).`,
      );
    });
  }
}

// The helper-side shape, in `.ts`: a function that formats by locale and reads
// the store itself. Three of these shipped in one week - a Files sidebar, a
// Settings profile list, a timeline day header - and each looked right in the
// source. They can only be seen on a screen in the wrong language, and nobody
// looks every time, which is why this is a check rather than a habit.
for (const base of BASES) {
  if (!existsSync(base)) continue;
  for (const file of tsFiles(base)) {
    const text = readFileSync(file, "utf8");
    if (!text.includes("get(locale)") && !text.includes("get( locale")) continue;
    tsScanned++;
    const rel = relative(ROOT, file);
    const allLines = text.split("\n");
    allLines.forEach((line, i) => {
      const m = /\bget\s*\(\s*locale\s*\)/.exec(line);
      if (!m) return;
      if (isParameterDefault(line, m.index)) return;
      if (isCallArgument(allLines, i)) return;
      problems.push(
        `${rel}:${i + 1}: reads the locale with get(locale) inside a body, which is ` +
          `not a tracked dependency, so anything formatted here keeps whichever ` +
          `language rendered first. Take the locale as a parameter and let the call ` +
          `site pass $locale.`,
      );
    });
  }
}

// Nothing at all means the walk is broken, not that the tree is clean. Either
// half being empty is fine: a `.ts`-only tree has no components, and a tree with
// no locale-reading helper never opens one.
if (!scanned && !tsScanned) {
  console.error("found nothing to check; the walk needs updating");
  process.exit(2);
}
if (problems.length) {
  console.log("catalog strings wired so they cannot follow a locale switch:\n");
  for (const p of problems) console.log(`  - ${p}`);
  process.exit(1);
}
console.log(
  `no non-reactive catalog wiring in ${scanned} component(s)` +
    ` and ${tsScanned} locale-reading helper file(s)`,
);
