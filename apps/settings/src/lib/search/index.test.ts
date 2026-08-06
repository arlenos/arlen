// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { search } from "./index";
import { get } from "svelte/store";

import { CATALOGS, locale, t } from "$lib/i18n/messages";
import { SETTINGS_REGISTRY } from "./settings-registry";

const tr = (id: string) => get(t)(id);

/// `s.idx.display.panel.*` is translated, so these say something about the
/// mechanism rather than about the German catalog being empty: if the source-locale
/// match were dropped, the first case would fail while the second still passed.
describe("settings search", () => {
  it("finds a setting by the name somebody learned it under, in another UI language", () => {
    locale.set("de");
    const byEnglish = search("night light").map((r) => r.setting.id);
    expect(byEnglish).toContain("display.panel");
    locale.set("en");
  });

  it("finds it by the language on screen", () => {
    locale.set("de");
    const byGerman = search("nachtlicht").map((r) => r.setting.id);
    expect(byGerman).toContain("display.panel");
    locale.set("en");
  });

  it("shows the German title once the locale is German", () => {
    // Not just "a result came back": the text a user reads has to change.
    locale.set("de");
    const de = search("tastatur").map((r) => tr(r.setting.titleKey));
    expect(de).toContain("Tastaturbelegung");
    locale.set("en");
    const en = search("keyboard layout").map((r) => tr(r.setting.titleKey));
    expect(en).toContain("Keyboard Layout");
  });

  it("finds a setting by a German word that appears nowhere in English", () => {
    // The case the German keyword sets exist for. "Lupe" is not a translation of
    // any English keyword on that entry - the English says magnifier, zoom, a11y -
    // so this can only match through the German keywords.
    locale.set("de");
    expect(search("lupe").map((r) => r.setting.id)).toContain("accessibility.zoom.shortcuts");
    expect(search("linkshänder").map((r) => r.setting.id)).toContain("mouse.left_handed");
    locale.set("en");
  });

  it("has every indexed message in every catalog locale", () => {
    // A missing translation degrades quietly: the entry falls back to English and
    // still matches through the source text, so nothing looks broken while a German
    // user reads an English row. This is the only thing that would say so.
    const indexed = new Set(
      SETTINGS_REGISTRY.flatMap((s) => [s.titleKey, s.descKey, s.keywordsKey, s.sectionKey]),
    );
    for (const [loc, catalog] of Object.entries(CATALOGS)) {
      const missing = [...indexed].filter((id) => catalog[id] === undefined);
      expect({ loc, missing }).toEqual({ loc, missing: [] });
    }
  });

  it("still finds it in the source language when that is what is shown", () => {
    locale.set("en");
    expect(search("night light").map((r) => r.setting.id)).toContain("display.panel");
  });
});
