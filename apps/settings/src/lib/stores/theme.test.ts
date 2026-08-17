/// Which typography field wins, and what a malformed one does.
///
/// The case that motivated these: the Appearance page's rows write
/// `overrides.typography.*` and every other app picked them up, while this app
/// kept applying its own older `[fonts]` section. Dragging the size slider
/// changed the whole desktop except the window it was being dragged in.

import { describe, expect, it } from "vitest";
import {
  cssLengthToPx,
  resolveTypography,
  type AppearanceConfig,
} from "./theme";

const base: AppearanceConfig = { theme: { active: "dark" } };

describe("cssLengthToPx", () => {
  it("reads the theme's own spelling of a size", () => {
    expect(cssLengthToPx("15px")).toBe(15);
    expect(cssLengthToPx("15")).toBe(15);
  });

  it("gives nothing for an absent or unparseable value", () => {
    // Not zero, and not NaN: the caller must fall through to its next source
    // rather than apply either as a font size.
    expect(cssLengthToPx(undefined)).toBeUndefined();
    expect(cssLengthToPx("")).toBeUndefined();
    expect(cssLengthToPx("large")).toBeUndefined();
  });
});

describe("resolveTypography", () => {
  it("takes the typography override over the older fonts section", () => {
    const config: AppearanceConfig = {
      ...base,
      overrides: {
        typography: { font_sans: "Cantarell", font_mono: "Fira Code", size_base: "17px" },
      },
      fonts: { interface: "Inter Variable", monospace: "JetBrains Mono", size: 15 },
    };
    expect(resolveTypography(config)).toEqual({
      fontInterface: "Cantarell",
      fontMono: "Fira Code",
      fontSize: 17,
    });
  });

  it("falls back to the fonts section where no override is set", () => {
    const config: AppearanceConfig = {
      ...base,
      overrides: { typography: { font_mono: "Fira Code" } },
      fonts: { interface: "Cantarell", monospace: "JetBrains Mono", size: 13 },
    };
    const out = resolveTypography(config);
    expect(out.fontInterface).toBe("Cantarell");
    expect(out.fontMono).toBe("Fira Code");
    expect(out.fontSize).toBe(13);
  });

  it("falls through a malformed override rather than applying it", () => {
    const config: AppearanceConfig = {
      ...base,
      overrides: { typography: { size_base: "very large" } },
      fonts: { size: 13 },
    };
    expect(resolveTypography(config).fontSize).toBe(13);
  });

  it("has an answer with no config at all", () => {
    const out = resolveTypography(undefined);
    expect(typeof out.fontInterface).toBe("string");
    expect(out.fontSize).toBeGreaterThan(0);
  });
});
