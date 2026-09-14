// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// The app icon under jsdom, which has no Web Animations API: what can be
/// proven here is the contract, not the motion. Every app draws its glyph
/// inside the plate, the drawing is hidden from assistive technology, the
/// size lands on the element, and the state is written where a stylesheet
/// and a harness can read it.
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import AppIcon from "./app-icon.svelte";
import { APP_IDS, APP_ICON_STATES, GLYPHS } from "./glyphs.js";

describe("app icon", () => {
  it("draws every app's glyph inside the plate and hides it from readers", () => {
    for (const app of APP_IDS) {
      const { container } = render(AppIcon, { app });
      const svg = container.querySelector("svg");
      expect(svg?.getAttribute("aria-hidden"), app).toBe("true");
      expect(container.querySelector(".plate"), app).not.toBeNull();
      expect(container.querySelectorAll(".glyph > *").length, app).toBe(GLYPHS[app].length);
    }
  });

  it("takes its size from the prop", () => {
    const { container } = render(AppIcon, { app: "files", size: 24 });
    expect(container.querySelector(".app-icon")?.getAttribute("style")).toContain("--app-icon-size: 24px");
  });

  it("writes the state on the element for the plate's stylesheet", async () => {
    const { container, rerender } = render(AppIcon, { app: "mail" });
    for (const state of APP_ICON_STATES) {
      await rerender({ app: "mail", state });
      expect(container.querySelector(".app-icon")?.getAttribute("data-state")).toBe(state);
    }
  });

  it("keeps every glyph on the stroke idiom: no fill, no colour of its own", () => {
    for (const app of APP_IDS) {
      for (const el of GLYPHS[app]) {
        expect(el, app).not.toMatch(/fill=|stroke=|#[0-9a-f]{3}/i);
      }
    }
  });
});
