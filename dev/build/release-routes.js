// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Keep the dev harness routes out of a release build.
 *
 * Every app has `_`-prefixed routes - `_rendertest`, `_nettest`, `_facettest` -
 * that exist so the states a person cannot reach on purpose can still be
 * rendered and screenshot: a sidebar's refusal sentence, a toggle that could not
 * be saved. They are how anything gets looked at, so they must keep working in
 * the dev and verify builds.
 *
 * They must NOT reach a user's machine. They are test surfaces wired to real
 * commands with mock data - a set of doors nobody designed for a person to open
 * - and they ship today: a production build of the greeter emits the compiled
 * `_a11ytest` page, its stylesheet, and a route-manifest entry naming it, so an
 * app pointed at that path renders the harness.
 *
 * WHY A ROUTES DIRECTORY RATHER THAN A RUNTIME GUARD. A guard inside the page
 * still ships the page; the code, its imports and its strings are all in the
 * bundle, and "it refuses to render" is a promise rather than an absence. Kit
 * builds its route manifest by walking `kit.files.routes`, so pointing a release
 * build at a directory that does not contain the harnesses removes them from the
 * manifest, the chunks and the CSS at once.
 *
 * WHY SYMLINKS RATHER THAN A COPY. A copy has to be kept in step and doubles the
 * source of truth; a move is destructive if the build dies halfway. A farm of
 * symlinks is neither: it is rebuilt from scratch each time, and Vite resolves
 * through them to the real file, so relative imports and `$lib` behave exactly
 * as they do in the dev build.
 */

import fs from "node:fs";
import path from "node:path";

/** A route directory that exists for tooling rather than for a person. */
export const isDevRoute = (name) => name.startsWith("_");

/**
 * The routes directory this build should use.
 *
 * Returns `src/routes` unchanged unless `ARLEN_RELEASE` is set, so a dev server,
 * `npm run check` and the screenshot tooling all keep every route. Under
 * `ARLEN_RELEASE` it builds `.svelte-kit/release-routes` as a symlink farm over
 * everything except the harnesses, and returns that.
 *
 * @param {string} appDir absolute path to the app (where `src/routes` lives)
 * @returns {string} the directory to hand to `kit.files.routes`
 */
export function routesDir(appDir) {
  const real = path.join(appDir, "src", "routes");
  if (!process.env.ARLEN_RELEASE) return real;

  const staged = path.join(appDir, ".svelte-kit", "release-routes");
  fs.rmSync(staged, { recursive: true, force: true });
  fs.mkdirSync(staged, { recursive: true });

  for (const entry of fs.readdirSync(real)) {
    if (isDevRoute(entry)) continue;
    // Relative link targets, so the farm survives the tree being moved.
    fs.symlinkSync(path.relative(staged, path.join(real, entry)), path.join(staged, entry));
  }
  return staged;
}
