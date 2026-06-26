import { describe, it, expect } from "vitest";
import {
  clampFontSize,
  zoomStep,
  matchZoom,
  FONT_SIZE_MIN,
  FONT_SIZE_MAX,
} from "./zoom";

describe("clampFontSize", () => {
  it("holds a size inside the range", () => {
    expect(clampFontSize(14)).toBe(14);
  });
  it("clamps below the floor and above the ceiling", () => {
    expect(clampFontSize(FONT_SIZE_MIN - 5)).toBe(FONT_SIZE_MIN);
    expect(clampFontSize(FONT_SIZE_MAX + 100)).toBe(FONT_SIZE_MAX);
  });
});

describe("zoomStep", () => {
  it("steps up and down by one pixel", () => {
    expect(zoomStep(14, "in")).toBe(15);
    expect(zoomStep(14, "out")).toBe(13);
  });
  it("rounds a fractional base onto whole pixels", () => {
    expect(zoomStep(14.4, "in")).toBe(15);
  });
  it("never steps past the bounds", () => {
    expect(zoomStep(FONT_SIZE_MAX, "in")).toBe(FONT_SIZE_MAX);
    expect(zoomStep(FONT_SIZE_MIN, "out")).toBe(FONT_SIZE_MIN);
  });
});

describe("matchZoom", () => {
  const ev = (key: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; metaKey: boolean }> = {}) => ({
    ctrlKey: true,
    altKey: false,
    metaKey: false,
    key,
    ...mods,
  });

  it("maps the universal zoom chords", () => {
    expect(matchZoom(ev("="))).toBe("in");
    expect(matchZoom(ev("+"))).toBe("in");
    expect(matchZoom(ev("-"))).toBe("out");
    expect(matchZoom(ev("_"))).toBe("out");
    expect(matchZoom(ev("0"))).toBe("reset");
  });
  it("requires Ctrl and rejects Alt/Meta so WM chords are not stolen", () => {
    expect(matchZoom(ev("=", { ctrlKey: false }))).toBeNull();
    expect(matchZoom(ev("=", { altKey: true }))).toBeNull();
    expect(matchZoom(ev("=", { metaKey: true }))).toBeNull();
  });
  it("ignores a non-zoom key", () => {
    expect(matchZoom(ev("a"))).toBeNull();
    expect(matchZoom(ev("1"))).toBeNull();
  });
});
