// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// This is a development fixture, not a page of the app.
///
/// It renders mock data so the screenshot harness can photograph a component
/// without the daemons behind it. SvelteKit has no notion of a route that is not
/// shipped, so without this it is built into the bundle and reachable by URL in
/// the packaged app - `_difftest.html` and four siblings were sitting in
/// `apps/harness/build/` next to `index.html`.
///
/// The guard is `dev` rather than a build exclusion because the harness drives
/// these routes through `npm run dev`, where `dev` is true. So the fixtures keep
/// working for the tooling they exist for, and a user of the built app gets the
/// same 404 as for any address that is not a page.
import { dev } from "$app/environment";
import { error } from "@sveltejs/kit";

export const load = () => {
  if (!dev) error(404, "not a page of this app");
};
