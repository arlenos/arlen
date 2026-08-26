/// The annotation geometry, and the one property the whole tool rests on.
///
/// First tests this app has had: it shipped with no `test` script and no test
/// files, which `check-crate-coverage` cannot report because it counts the
/// package.json files that DECLARE one. An app with none is not counted as
/// uncovered; it is not counted at all.
import { describe, expect, it } from "vitest";
import { drawShape, rectOf } from "./annotate";

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

/// The blur tool, because it is the one shape whose failure is a privacy failure.
///
/// Every other tool draws a mark and you can see whether it landed. A blur is a
/// claim that something underneath is now gone, and a blur that did not draw
/// looks exactly like one that did until the file is opened somewhere else.
///
/// The context is a recorder rather than a real canvas: what is worth pinning is
/// which calls happen, not the pixels they produce.
describe("drawShape, blur", () => {
  function recorder() {
    const calls: string[] = [];
    const ctx = {
      save: () => calls.push("save"),
      restore: () => calls.push("restore"),
      drawImage: () => calls.push("drawImage"),
      strokeRect: () => calls.push("strokeRect"),
      beginPath: () => calls.push("beginPath"),
      stroke: () => calls.push("stroke"),
      moveTo: () => {},
      lineTo: () => {},
      ellipse: () => {},
      fillText: (s: string) => calls.push(`fillText:${s}`),
      set globalAlpha(_v: number) {},
      set lineWidth(_v: number) {},
      set strokeStyle(_v: string) {},
      set fillStyle(_v: string) {},
      set lineCap(_v: string) {},
      set lineJoin(_v: string) {},
      set font(_v: string) {},
      set textBaseline(_v: string) {},
      set imageSmoothingEnabled(_v: boolean) {},
    };
    return { ctx: ctx as unknown as CanvasRenderingContext2D, calls };
  }

  const base = {} as CanvasImageSource;

  /// `drawBlur` reaches for `document.createElement("canvas")` to build the
  /// scratch surface it downsamples through, so the node environment needs one.
  /// A stub rather than jsdom: jsdom's `getContext("2d")` returns null without a
  /// native canvas backend, `drawBlur` treats that as a reason to return, and the
  /// test would then pass for the same reason a broken redaction does.
  function withStubbedCanvas<T>(run: () => T): T {
    const had = "document" in globalThis;
    const scratch = {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage: () => {} }),
    };
    (globalThis as Record<string, unknown>).document = {
      createElement: () => scratch,
    };
    try {
      return run();
    } finally {
      if (!had) delete (globalThis as Record<string, unknown>).document;
    }
  }

  it("pixelates the region by drawing it twice, small then back", () => {
    const { ctx, calls } = recorder();
    withStubbedCanvas(() =>
      drawShape(
        ctx,
        { id: 1, kind: "blur", start: { x: 10, y: 10 }, end: { x: 90, y: 60 }, color: "#000", size: 2 },
        base,
      ),
    );
    // The scratch canvas takes one and the visible context takes the other, so
    // only the second lands here. Zero means the redaction did not happen.
    expect(calls.filter((c) => c === "drawImage").length).toBe(1);
  });

  it("draws nothing for a drag too small to be a redaction", () => {
    const { ctx, calls } = recorder();
    drawShape(
      ctx,
      { id: 2, kind: "blur", start: { x: 10, y: 10 }, end: { x: 11, y: 11 }, color: "#000", size: 2 },
      base,
    );
    expect(calls.filter((c) => c === "drawImage").length).toBe(0);
  });
});

/// Text is the other shape carrying content rather than geometry, and the only
/// one that has to decide something about it: where the next line goes.
describe("drawShape, text", () => {
  it("draws one line per newline rather than one run with escapes in it", () => {
    const calls: string[] = [];
    const ctx = {
      save: () => {},
      restore: () => {},
      fillText: (s: string) => calls.push(s),
      set font(_v: string) {},
      set textBaseline(_v: string) {},
      set fillStyle(_v: string) {},
      set strokeStyle(_v: string) {},
      set lineWidth(_v: number) {},
      set lineCap(_v: string) {},
      set lineJoin(_v: string) {},
    } as unknown as CanvasRenderingContext2D;
    drawShape(
      ctx,
      {
        id: 3,
        kind: "text",
        start: { x: 0, y: 0 },
        end: { x: 0, y: 0 },
        color: "#000",
        size: 2,
        text: "first\nsecond",
      },
      {} as CanvasImageSource,
    );
    expect(calls).toEqual(["first", "second"]);
  });
});
