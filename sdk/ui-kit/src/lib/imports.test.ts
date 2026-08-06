// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { describe, expect, it } from "vitest";

/// The kit is consumed by source alias - `@arlen/ui-kit` points at
/// `sdk/ui-kit/src/lib`, so its files are compiled by the consuming app. That means
/// `$lib` inside a kit file is the APP's `src/lib`, not ours.
///
/// It bites in two ways and only one of them is loud. `$lib/i18n` simply failed to
/// resolve from an app, which was obvious. `$lib/utils` resolved fine - because every
/// app happens to keep a `utils.ts` exporting a compatible `cn` - so 64 kit files
/// were quietly binding to the consumer's module instead of ours, and would have
/// kept working right up until an app's copy drifted.
///
/// Neither the kit's own type-check nor its tests can see this: inside the kit,
/// `$lib` resolves to the kit. Only a consumer notices, and only for the loud half.
/// So the rule is checked here as a rule: a kit file reaches its own modules by
/// relative path.
// vitest runs from the package root, and `import.meta.url` here is a virtual
// path rather than a real one.
const ROOT = join(process.cwd(), "src");

function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sources(p, out);
    else if (name.endsWith(".svelte") || name.endsWith(".ts")) out.push(p);
  }
  return out;
}

describe("kit imports", () => {
  it("never reaches for $lib, which belongs to whoever is compiling us", () => {
    const offenders = sources(join(ROOT, "lib"))
      .filter((p) =>
        readFileSync(p, "utf8")
          .split("\n")
          // Not comment lines: the barrel documents the app-side spelling in a
          // `///` example, and a check that flags prose is one people learn to skip.
          .filter((l) => !/^\s*(\/\/|\*)/.test(l))
          .some((l) => /from\s+["']\$lib\//.test(l)),
      )
      .map((p) => relative(ROOT, p));
    expect(offenders).toEqual([]);
  });
});
