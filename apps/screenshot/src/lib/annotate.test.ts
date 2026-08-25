/// The annotation geometry, and the one property the whole tool rests on.
///
/// First tests this app has had: it shipped with no `test` script and no test
/// files, which `check-crate-coverage` cannot report because it counts the
/// package.json files that DECLARE one. An app with none is not counted as
/// uncovered; it is not counted at all.
import { describe, expect, it } from "vitest";
import { rectOf } from "./annotate";

describe("rectOf", () => {
  /// A drag has four directions and a rectangle has one representation. The tool
  /// stores `start` and `end` exactly as the pointer gave them, so every consumer
  /// - the crop, the blur region, the export bounds - depends on this
  /// normalisation rather than on the drag having gone down-right.
  it("normalises a drag from any corner to the same rectangle", () => {
    const want = { x: 10, y: 20, w: 30, h: 40 };
    expect(rectOf({ x: 10, y: 20 }, { x: 40, y: 60 })).toEqual(want);
    expect(rectOf({ x: 40, y: 60 }, { x: 10, y: 20 })).toEqual(want);
    expect(rectOf({ x: 40, y: 20 }, { x: 10, y: 60 })).toEqual(want);
    expect(rectOf({ x: 10, y: 60 }, { x: 40, y: 20 })).toEqual(want);
  });

  /// A click without a drag is a zero-size rectangle, not a negative one. The
  /// blur tool divides by the region's size, so a negative width is a NaN on the
  /// canvas rather than a visible mistake.
  it("gives a click no size rather than a negative one", () => {
    expect(rectOf({ x: 5, y: 5 }, { x: 5, y: 5 })).toEqual({ x: 5, y: 5, w: 0, h: 0 });
  });

  /// Negative coordinates happen: a drag that leaves the canvas is clamped by the
  /// caller, not here, and this must not quietly reorder them.
  it("keeps a rectangle that starts off-canvas", () => {
    expect(rectOf({ x: -20, y: -5 }, { x: 0, y: 5 })).toEqual({ x: -20, y: -5, w: 20, h: 10 });
  });
});
