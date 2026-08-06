// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { search } from "./index";
import { locale } from "$lib/i18n/messages";

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

  it("still finds it in the source language when that is what is shown", () => {
    locale.set("en");
    expect(search("night light").map((r) => r.setting.id)).toContain("display.panel");
  });
});
