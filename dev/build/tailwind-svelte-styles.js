// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `@tailwindcss/vite`, told to leave Svelte's virtual style modules alone.
 *
 * SvelteKit emits one module per `<style>` block, at an id like
 * `.../ProvenanceHalo.svelte?svelte&type=style&lang.css`. Tailwind's transform
 * filter includes `/&lang\.css/`, so it matches those, reads the file at the
 * un-queried path, and gets the RAW `.svelte` source - script, template and all.
 * Its CSS parser then fails on the first thing that is not CSS. An apostrophe in
 * a prose comment is enough: the stylesheet request answers `Unterminated` and
 * the app wears an error overlay.
 *
 * This adds one exclude pattern to Tailwind's own id filter, which wins over its
 * include. Tailwind declines exactly those ids; nothing else changes and no
 * other plugin's hooks are touched.
 *
 * **THAT LAST PART IS THE WHOLE POINT, and the obvious fix gets it wrong.** The
 * shell carried a private plugin that answered those ids from an
 * `enforce: "pre"` `load` hook with an empty string. It does stop the crash, and
 * it also empties the style module: a pre-load returning a value is the first
 * answer, so vite-plugin-svelte's own load for that module never runs. Measured
 * cold, clean tree, same component - `.ph-trigger` present without it, absent
 * with it.
 *
 * How much that SHOWS depends on the component. Rendering the shell's `_qstest`
 * both ways puts the difference at a few pixels of vertical offset, not missing
 * styling, even though both components on it carry `<style>` blocks - so "the
 * shell rendered unstyled" would be more than was measured. What is measured is
 * that the rules do not arrive, which is not a state to leave standing whether
 * or not a given surface happens to survive it.
 *
 * Two other things measured on the way, since both cost an hour:
 *
 * - **A warm dev server cannot answer this question.** Two probes said the trap
 *   was gone; the transform was cached from before the edit. Only a server
 *   started after the change tells the truth.
 * - Tailwind's documented plugin order (`tailwindcss()` first) does not fix it.
 */

/** Svelte's compiled `<style>` modules, which Tailwind has no business reading. */
const SVELTE_STYLE_QUERY = /[?&]svelte&type=style/;

/**
 * Wrap an app's own `tailwindcss()` call: `withoutSvelteStyles(tailwindcss())`.
 *
 * The app passes its plugins in rather than this module importing the package,
 * because `dev/build/` has no `node_modules` of its own and the root workspace
 * does not carry `@tailwindcss/vite` - so an import here resolves at runtime
 * through vite's own resolver and fails under every app's type check. Two errors
 * in `apps/knowledge` said so within minutes of it landing.
 *
 * @param {any[]} plugins the array `tailwindcss()` returns
 * @returns {any[]} the same array, with the svelte style ids excluded
 */
export function withoutSvelteStyles(plugins) {
  for (const plugin of plugins) {
    const filter = plugin.transform?.filter?.id;
    if (filter?.exclude) {
      filter.exclude = [...filter.exclude, SVELTE_STYLE_QUERY];
    }
  }
  return plugins;
}
