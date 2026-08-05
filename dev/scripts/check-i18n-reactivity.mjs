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
import { join } from "node:path";

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

const problems = [];
let scanned = 0;

for (const base of [join(ROOT, "apps"), join(ROOT, "sdk")]) {
  if (!existsSync(base)) continue;
  for (const file of svelteFiles(base)) {
    const text = readFileSync(file, "utf8");
    const script = text.match(/<script[^>]*>([\s\S]*?)<\/script>/);
    if (!script) continue;
    scanned++;
    const lines = script[1].split("\n");
    const rel = file.slice(ROOT.length);

    lines.forEach((line, i) => {
      if (/\bget\s*\(\s*t\s*\)\s*\(/.test(line)) {
        problems.push(
          `${rel}:${i + 2}: reads the translator with get(t), which is not a tracked ` +
            `dependency, so this text keeps whichever locale rendered first. Return the ` +
            `message key and call $t in the markup.`,
        );
      }

      // Top level in a Svelte `<script>` is two spaces; a function body is deeper.
      const decl = line.match(/^ {2}(?:const|let)\s+(\w+)[^=]*=\s*(.*)$/);
      if (!decl) return;
      const initialiser = [decl[2], ...lines.slice(i + 1, i + 30)].join("\n");
      // `$derived` may sit on the next line when the type annotation is long.
      if (initialiser.trimStart().startsWith("$derived")) return;
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

if (!scanned) {
  console.error("found no Svelte components; the check needs updating");
  process.exit(2);
}
if (problems.length) {
  console.log("catalog strings wired so they cannot follow a locale switch:\n");
  for (const p of problems) console.log(`  - ${p}`);
  process.exit(1);
}
console.log(`no non-reactive catalog wiring in ${scanned} component(s)`);
