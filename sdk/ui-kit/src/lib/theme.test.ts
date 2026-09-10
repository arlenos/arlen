// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// `injectThemeVariables` is what actually paints an Arlen app: every colour,
// radius and duration in the tree arrives through this one function, and every
// app calls it again on every desktop-wide theme change. Untested until now,
// which meant nothing said whether a second call replaces the block or adds
// another one - and a leak there is a page whose colours are decided by whichever
// stale `<style>` sorts last.
import { describe, it, expect, beforeEach } from "vitest";
import { injectThemeVariables, applyTokens, PANDA_TOKENS } from "./theme.js";

const STYLE_ID = "arlen-theme-vars";

function block(): string {
  return document.getElementById(STYLE_ID)?.textContent ?? "";
}

describe("injectThemeVariables", () => {
  beforeEach(() => {
    document.head.innerHTML = "";
    document.documentElement.style.colorScheme = "";
  });

  it("writes every variable onto :root with its double dash", () => {
    injectThemeVariables({
      variables: { "color-accent": "#6366f1", "radius-card": "12px" },
      font_scale: 1.0,
      variant: "dark",
    });
    expect(block()).toContain("--color-accent: #6366f1;");
    expect(block()).toContain("--radius-card: 12px;");
    expect(block().startsWith(":root {")).toBe(true);
  });

  it("sorts the variables, so two runs of one theme are the same text", () => {
    injectThemeVariables({
      variables: { zeta: "1", alpha: "2", mid: "3" },
      font_scale: 1,
      variant: "dark",
    });
    const order = [...block().matchAll(/--([a-z]+):/g)].map((m) => m[1]);
    expect(order).toEqual(["alpha", "mid", "zeta"]);
  });

  it("replaces its own block rather than stacking another one", () => {
    injectThemeVariables({ variables: { a: "1" }, font_scale: 1, variant: "dark" });
    injectThemeVariables({ variables: { b: "2" }, font_scale: 1, variant: "dark" });
    expect(document.querySelectorAll(`#${STYLE_ID}`)).toHaveLength(1);
    expect(block()).toContain("--b: 2;");
    expect(block()).not.toContain("--a: 1;");
  });

  it("only writes a root font size when the scale is not 1", () => {
    injectThemeVariables({ variables: {}, font_scale: 1.0, variant: "dark" });
    expect(block()).not.toContain("font-size");
    // Inside the tolerance: still no size, because 16.008px is not a decision.
    injectThemeVariables({ variables: {}, font_scale: 1.0005, variant: "dark" });
    expect(block()).not.toContain("font-size");
    injectThemeVariables({ variables: {}, font_scale: 1.25, variant: "dark" });
    expect(block()).toContain("font-size: 20px;");
  });

  it("tells the document which scheme it is in", () => {
    injectThemeVariables({ variables: {}, font_scale: 1, variant: "light" });
    expect(document.documentElement.style.colorScheme).toBe("light");
    injectThemeVariables({ variables: {}, font_scale: 1, variant: "dark" });
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });
});

describe("applyTokens", () => {
  it("puts the ten surface tokens on the root element", () => {
    applyTokens(PANDA_TOKENS);
    const root = document.documentElement;
    expect(root.style.getPropertyValue("--color-bg-shell")).toBe(PANDA_TOKENS.bgShell);
    expect(root.style.getPropertyValue("--color-accent")).toBe(PANDA_TOKENS.accent);
    expect(root.style.getPropertyValue("--radius")).toBe(PANDA_TOKENS.radius);
  });
});
