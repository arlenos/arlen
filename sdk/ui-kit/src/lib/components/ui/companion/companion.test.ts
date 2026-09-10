// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// The companion under jsdom, which has neither SMIL nor the Web Animations
/// API: what can be proven here is the contract, not the motion. The drawing
/// is hidden from assistive technology and a status region says the state in
/// words; every state lands its pose in the path data; the bust is the same
/// drawing with another viewBox; and both locales have a sentence for every
/// state, because a state with no words would announce nothing.
import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import { render } from "@testing-library/svelte";
import { locale } from "../../../i18n";
import { kitMessages } from "../../../i18n/messages.kit";
import Companion from "./companion.svelte";
import { COMPANION_STATES, POSES, SLOTS, VIEW_BUST, VIEW_FIGURE } from "./rig.js";

describe("companion", () => {
  it("hides the drawing and says the state in words", () => {
    const { container } = render(Companion, { state: "thinking" });
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("aria-hidden")).toBe("true");
    const status = container.querySelector("[role=status]");
    expect(status?.textContent).toBe("Working on it.");
  });

  it("lands every state's pose in the path data", async () => {
    const { container, rerender } = render(Companion, { state: "resting", idle: false });
    for (const state of COMPANION_STATES) {
      await rerender({ state, idle: false });
      const pose = POSES[state];
      for (const slot of SLOTS) {
        expect(container.querySelector(`.${slot}`)?.getAttribute("d"), `${state}.${slot}`).toBe(pose[slot]);
      }
    }
  });

  it("frames the bust with the same drawing and another viewBox", () => {
    const figure = render(Companion, { state: "resting" }).container.querySelector("svg");
    expect(figure?.getAttribute("viewBox")).toBe(VIEW_FIGURE);
    const bust = render(Companion, { state: "resting", bust: true, radius: "50%" }).container;
    expect(bust.querySelector("svg")?.getAttribute("viewBox")).toBe(VIEW_BUST);
    expect(bust.querySelector(".companion")?.getAttribute("style")).toContain("border-radius: 50%");
  });

  it("speaks German when the locale does", async () => {
    const before = get(locale);
    locale.set("de");
    try {
      const { container } = render(Companion, { state: "failed" });
      expect(container.querySelector("[role=status]")?.textContent).toBe("Etwas ist schiefgegangen.");
    } finally {
      locale.set(before);
    }
  });

  it("has a sentence for every state in both locales, and never the species", () => {
    for (const lang of ["en", "de"] as const) {
      const table = kitMessages[lang] as Record<string, string>;
      for (const state of COMPANION_STATES) {
        const text = table[`k.companion.${state}`];
        expect(text, `${lang} ${state}`).toBeTruthy();
        expect(text).not.toMatch(/marmot|murmel/i);
      }
    }
  });
});
