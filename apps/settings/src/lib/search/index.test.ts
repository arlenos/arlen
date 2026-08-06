// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { search } from "./index";
import { get } from "svelte/store";

import { locale, t } from "$lib/i18n/messages";

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

  it("still finds it in the source language when that is what is shown", () => {
    locale.set("en");
    expect(search("night light").map((r) => r.setting.id)).toContain("display.panel");
  });
});
