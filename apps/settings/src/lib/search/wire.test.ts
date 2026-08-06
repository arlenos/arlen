// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The shape the shell parses, pinned against what this app writes.
//
// The shell has its own tests, but their JSON was hand-written from reading this
// file, which only proves the author agreed with himself. On 6 August that exact
// pairing held while the two sides disagreed in production: this app moved to ids
// and the shell still required prose, so serde failed and Waypointer's settings
// search returned nothing behind a warn-level line.
//
// The fixture below is the shell's test input, generated from the real registry.
// Regenerate with `npx vitest run src/lib/search/wire.test.ts -u`-style intent:
// the test fails loudly and prints the new keys, so a rename cannot land on one
// side alone.
import { describe, it, expect } from "vitest";
import { buildExportPayload } from "./index";

/// The one table both sides answer. Reading it here rather than restating it is
/// the point: a list repeated in two languages is two lists, and they drift.
import WIRE from "../../../../../contracts/settings-index/wire-keys.json";

const SETTING_KEYS = [...WIRE.setting].sort();
const TOP_KEYS = [...WIRE.top].sort();

describe("the exported index shape", () => {
  const payload = buildExportPayload();

  it("carries the fields the shell parses, and only those", () => {
    expect(Object.keys(payload).sort()).toEqual(TOP_KEYS);
    for (const s of payload.settings) {
      // `inlineAction` is optional and drops out when absent, so compare against
      // the subset rather than requiring every entry to carry it.
      const keys = Object.keys(s).sort();
      expect(SETTING_KEYS).toEqual(expect.arrayContaining(keys));
      expect(keys).toContain("titleKey");
    }
  });

  it("is the version the shell reads", () => {
    // The shell refuses anything else outright. Bumping one side alone is the
    // failure this pair exists to stop.
    expect(payload.version).toBe(WIRE.version);
  });

  it("names a catalog, because the ids resolve against one", () => {
    expect(payload.catalog).toBe("settings");
  });

  it("carries ids, never prose", () => {
    for (const s of payload.settings) {
      // A message id, not a sentence. Prose here would be one language baked in
      // at whatever moment Settings happened to run.
      expect(s.titleKey).toMatch(/^[a-z][A-Za-z0-9]*\.[A-Za-z0-9._-]+$/);
      expect(s.descKey).toMatch(/^[a-z][A-Za-z0-9]*\.[A-Za-z0-9._-]+$/);
    }
  });

  it("gives every inline select option an id too", () => {
    for (const s of payload.settings) {
      for (const o of s.inlineAction?.options ?? []) {
        expect(o).toHaveProperty("labelKey");
        expect(o).not.toHaveProperty("label");
      }
    }
  });

  it("keeps the nested shapes inside the table as well", () => {
    // Otherwise half the table is load-bearing and half is decoration, and the
    // decorative half is where the next disagreement lands.
    for (const s of payload.settings) {
      const a = s.inlineAction;
      if (!a) continue;
      expect(WIRE.inlineAction).toEqual(expect.arrayContaining(Object.keys(a)));
      for (const o of a.options ?? []) {
        expect(WIRE.selectOption).toEqual(expect.arrayContaining(Object.keys(o)));
      }
    }
  });
});
